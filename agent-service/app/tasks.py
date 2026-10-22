"""后台生成任务注册表与执行运行时（生成的所有权移到 Python）。

核心语义：
- POST /api/generations 提交 → {taskId} 毫秒级返回（asyncio.create_task 跑完整
  LangGraph，连接断开 / 页面关闭 / Rust 重启都不影响生成）；
- 每任务一份有序事件日志（seq 从 1 自增；事件 dict 与 NDJSON 行完全同构）；
- WS /api/generations/{taskId}/events?cursor=N 游标回放 + 直播，回放↔直播
  切换用 per-task asyncio.Lock 衔接（不丢不重）；
- done 必为事件日志最后一项：意外异常 → done(error, code=service)；
  取消 → done(error, code=cancelled)（done 是消费者判断终态的唯一凭借）；
- 生命周期防线：全局并发 MAX_CONCURRENT=3（plan/regen 共享池，超限 429）、
  registry 容量 MAX_REGISTRY_SIZE=64、finished/cancelled 超过 TASK_TTL_SECONDS
  驱逐——三者均为模块常量，便于测试 monkeypatch。
"""

from __future__ import annotations

import asyncio
import logging
import uuid
from collections import deque
from collections.abc import AsyncIterator, Callable
from dataclasses import dataclass, field
from enum import Enum
from time import monotonic
from typing import Any, cast

from app.errors import AgentServiceError
from app.schemas import ErrorDetail, PlanRequest, PlanResponse, RegenRequest

logger = logging.getLogger("praxis.tasks")

# 并发与容量防线：常量化便于测试 monkeypatch（引用处一律运行时读本模块属性）
MAX_CONCURRENT = 3  # running 任务全局上限（plan / regen 共享池）
MAX_REGISTRY_SIZE = 64  # registry 最多缓存条数（sweep 过期后仍超 → 拒绝 429）
TASK_TTL_SECONDS = 600  # finished / cancelled 任务保留 10 分钟供补拉，随后驱逐


class TaskKind(str, Enum):
    """任务种类：与事件生成器一一对应（plan / 全阶段重生成 / 单章重生成）。"""

    plan = "plan"
    regen_phase = "regen_phase"
    regen_chapter = "regen_chapter"


class TaskStatus(str, Enum):
    """任务生命周期状态。"""

    running = "running"
    finished = "finished"
    cancelled = "cancelled"


class ConcurrencyExceededError(Exception):
    """并发上限不足 / registry 满：端点必须翻译为 429。"""


@dataclass
class TaskEntry:
    """单个后台生成任务的 registry 记录。"""

    task_id: str
    kind: TaskKind
    events: deque[tuple[int, dict[str, Any]]] = field(default_factory=deque)
    status: TaskStatus = TaskStatus.running
    created_at: float = field(default_factory=monotonic)
    finished_at: float | None = None
    task: asyncio.Task | None = None

    @property
    def done(self) -> bool:
        """事件日志是否已收尾（末条必为 done，见 _run 的异常翻译）。"""
        return bool(self.events) and self.events[-1][1].get("type") == "done"


class TaskRegistry:
    """任务注册表：id → TaskEntry；submit / cancel / snapshot / WS 衔接。"""

    def __init__(self) -> None:
        self.tasks: dict[str, TaskEntry] = {}
        # per-task 回放↔直播衔接锁：WS 连接建立 + 历史快照期间新事件不得追加，
        # 否则回放结束到直播开始的间隙可能丢事件（不丢不重裁定）
        self._locks: dict[str, asyncio.Lock] = {}

    def get(self, task_id: str) -> TaskEntry | None:
        """按 id 查任务；不存在返回 None（由端点翻译 404 / WS 4404）。

        读前先 sweep：TTL 过期的终态任务即刻驱逐（供 snapshot / WS 复用）。
        """
        self._sweep_expired()
        return self.tasks.get(task_id)

    def lock_for(self, task_id: str) -> asyncio.Lock:
        """取任务专属锁（WS 快照衔接用；与 _append 事件追加路径互斥）。"""
        return self._locks.setdefault(task_id, asyncio.Lock())

    # ------------------------------------------------------------------
    # 事件追加与终态：per-task 单协程顺序追加（asyncio 串行调度天然有序）
    # ------------------------------------------------------------------

    async def _append(self, entry: TaskEntry, event: dict[str, Any]) -> None:
        """以自增 seq 追加一条事件；与 WS 回放快照互斥（同一把 per-task lock）。

        cancelled 任务的迟到事件直接丢弃：done(cancelled) 已是日志末项，
        不得出现尾随事件破坏「done 必为最后一项」契约。
        """
        if entry.status is TaskStatus.cancelled:
            return
        await self._append_raw(entry, event)

    async def _append_raw(self, entry: TaskEntry, event: dict[str, Any]) -> None:
        """获取锁后追加（终态 done 翻译路径专用；常规事件走 _append）。

        注意：调用方**不得**已持有该锁 —— asyncio.Lock 非重入，嵌套申请即死锁；
        已持锁的调用方（cancel）改用 _append_locked。另：所有 per-task 锁
        临界区必须保持无 await（append 协议的隐式前提）——若需在锁内挂起，
        必须重新设计追加协议（见审查 P2-1）。
        """
        async with self.lock_for(entry.task_id):
            self._append_locked(entry, event)

    def _append_locked(self, entry: TaskEntry, event: dict[str, Any]) -> None:
        """已持锁前提下的同步追加（seq 自增进日志）。"""
        seq = (entry.events[-1][0] + 1) if entry.events else 1
        entry.events.append((seq, event))

    async def _append_done(self, entry: TaskEntry, response: PlanResponse) -> None:
        """统一收尾：终态包络 done（必为事件日志最后一项，任何路径不得绕过）。"""
        await self._append(
            entry,
            {
                "type": "done",
                "response": response.model_dump(mode="json", by_alias=True),
            },
        )

    # ------------------------------------------------------------------
    # 提交
    # ------------------------------------------------------------------

    async def submit(self, kind: TaskKind | str, payload: Any) -> str:
        """提交任务：校验容量与并发 → create_task 跑完整生成 → 立即返回 taskId。

        不等待生成完成（毫秒级返回）；kind 可传枚举或同名字符串（HTTP 层传入）；
        并发占满或缓存超限时抛 ConcurrencyExceededError（端点翻译 429，中文 message）。
        """
        self._sweep_expired()
        kind = TaskKind(kind)

        running = sum(1 for t in self.tasks.values() if t.status is TaskStatus.running)
        if running >= MAX_CONCURRENT:
            raise ConcurrencyExceededError(
                f"已达到最大并发数（{MAX_CONCURRENT}），请稍后再试或等待当前任务完成"
            )
        if len(self.tasks) >= MAX_REGISTRY_SIZE:
            raise ConcurrencyExceededError(
                f"任务缓存已满（{MAX_REGISTRY_SIZE} 条），请稍后再试"
            )

        task_id = uuid.uuid4().hex
        entry = TaskEntry(task_id=task_id, kind=kind)
        self.tasks[task_id] = entry
        self._locks[task_id] = asyncio.Lock()

        generator_factory: Callable[[], AsyncIterator[dict[str, Any]]]
        if kind is TaskKind.plan:
            request = cast(PlanRequest, payload)

            def generator_factory() -> AsyncIterator[dict[str, Any]]:
                return _plan_event_dicts(request)

        else:
            regen = cast(RegenRequest, payload)

            def generator_factory() -> AsyncIterator[dict[str, Any]]:
                return _regen_event_dicts(regen)

        entry.task = asyncio.create_task(self._run(entry, generator_factory))
        logger.info(
            "任务已提交：taskId=%s kind=%s running=%d",
            task_id,
            kind.value,
            running + 1,
        )
        return task_id

    # ------------------------------------------------------------------
    # 执行：迭代事件生成器，逐事件追加；一切终态都以 done 收尾
    # ------------------------------------------------------------------

    async def _run(
        self,
        entry: TaskEntry,
        generator_factory: Callable[[], AsyncIterator[dict[str, Any]]],
    ) -> None:
        """驱动单条任务：dict 生成器 → 事件日志追加；异常 → done(service)；
        cancel → done(cancelled)。退出时统一翻转 status 并记录 TTL 起点。"""
        try:
            async for event in generator_factory():
                await self._append(entry, event)
        except asyncio.CancelledError:
            # 取消翻译为明确终态（在 _run 内捕获，保证消费者拿到 done）；
            # 若 DELETE 分支已抢先落 done（幂等防线）则不重复追加
            if not entry.done:
                await self._append_done(
                    entry,
                    PlanResponse(
                        status="error",
                        error=ErrorDetail(code="cancelled", message="生成已取消"),
                    ),
                )
            entry.status = TaskStatus.cancelled
            entry.finished_at = monotonic()
            logger.info("任务已取消：taskId=%s", entry.task_id)
            raise  # 取消语义仍以 CancelledError 呈现给调度方（task 状态正常收场）
        except AgentServiceError as exc:
            logger.warning("任务业务失败：taskId=%s err=%s", entry.task_id, exc)
            await self._append_done(
                entry,
                PlanResponse(
                    status="error",
                    error=ErrorDetail(code="service", message=str(exc)),
                ),
            )
            entry.status = TaskStatus.finished
            entry.finished_at = monotonic()
        except Exception as exc:
            # 意外异常同样落 done(error, service)，不得让日志停在半路
            logger.exception("任务意外失败：taskId=%s", entry.task_id)
            await self._append_done(
                entry,
                PlanResponse(
                    status="error",
                    error=ErrorDetail(code="service", message=f"服务内部错误：{exc}"),
                ),
            )
            entry.status = TaskStatus.finished
            entry.finished_at = monotonic()
        else:
            entry.status = TaskStatus.finished
            entry.finished_at = monotonic()

    # ------------------------------------------------------------------
    # 取消（幂等）
    # ------------------------------------------------------------------

    async def cancel(self, task_id: str) -> TaskEntry:
        """幂等 cancel：running → task.cancel() + 立即落 done(error, cancelled)。

        done 在本方法内同步追加（不再等 _run 的 CancelledError 分支），保证
        DELETE 返回时日志已收尾、重复 DELETE 结果一致（幂等）；_run 的
        CancelledError 分支见 done 已落不再追加。已终态任务原样返回；
        不存在抛 KeyError（端点 404）。
        """
        entry = self.tasks.get(task_id)
        if entry is None:
            raise KeyError(task_id)
        if entry.status is TaskStatus.running and not entry.done:
            # 同一锁窗口内：先封锁后续 append（防线见 _append），
            # 再落 done(cancelled)——字节级保证 done 是日志最后一项
            async with self.lock_for(entry.task_id):
                entry.status = TaskStatus.cancelled
                if entry.task and not entry.task.done():
                    entry.task.cancel()
                # 已持锁：直接同步追加（_append_raw 会重入同一把非重入锁 → 死锁）
                self._append_locked(
                    entry,
                    {
                        "type": "done",
                        "response": PlanResponse(
                            status="error",
                            error=ErrorDetail(code="cancelled", message="生成已取消"),
                        ).model_dump(mode="json", by_alias=True),
                    },
                )
                entry.finished_at = monotonic()
        return entry

    # ------------------------------------------------------------------
    # 快照
    # ------------------------------------------------------------------

    @staticmethod
    def snapshot(entry: TaskEntry) -> dict[str, Any]:
        """任务状态摘要（camelCase）：WS 诊断 / Rust 断线重连参考。"""
        return {
            "taskId": entry.task_id,
            "kind": entry.kind.value,
            "status": entry.status.value,
            "firstSeq": entry.events[0][0] if entry.events else 0,
            "lastSeq": entry.events[-1][0] if entry.events else 0,
            "done": entry.done,
        }

    # ------------------------------------------------------------------
    # 驱逐
    # ------------------------------------------------------------------

    def _sweep_expired(self) -> None:
        """sweep 过期（finished / cancelled 超过 TTL）→ 驱逐。"""
        now = monotonic()
        expired: list[str] = []
        for task_id, entry in self.tasks.items():
            if entry.status is TaskStatus.running:
                continue
            if (
                entry.finished_at is not None
                and now - entry.finished_at > TASK_TTL_SECONDS
            ):
                expired.append(task_id)
        for task_id in expired:
            self.tasks.pop(task_id, None)
            self._locks.pop(task_id, None)
            logger.info("任务已驱逐：taskId=%s", task_id)


# ---------------------------------------------------------------------------
# 事件生成器：main.py 的 NDJSON 事件管道降层而来（端点成薄包装，wire 不变）
# ---------------------------------------------------------------------------


async def _plan_event_dicts(
    request: PlanRequest,
    *,
    outline: dict[str, Any] | None = None,
    regen_scope: dict[str, Any] | None = None,
) -> AsyncIterator[dict[str, Any]]:
    """驱动图流式执行，把节点增量翻译为事件 dict（终态 done 行同 dict）。

    这是**生成与重生成共用的唯一事件生成器**：
    - 首次生成：outline/regen_scope 均为 None，图走 clarify → outline → 扇出；
    - 重生成：outline 已给定（来自库）、regen_scope 指定目标范围，图从 intake
      直接扇出到那一段，之后 assemble / review / expand_revision 完全复用。

    消费面：TaskRegistry 后台任务（kind=plan / regen_*，WS 主链路）。

    业务异常（AgentServiceError）在此捕获并翻译为 error 包络的 done 终态
    （与全局 handler 的包络产出一致，见 app/errors.py）；意外异常
    **不在**此处捕获——由 TaskRegistry._run 统一翻译为 done(error, code=service)，
    保证任何路径 done 都是事件日志最后一项。
    """
    from app.graph import agent_graph
    from app.main import build_response

    # langgraph v1 astream 的 Sequence stream_mode 重载未建模 (mode, chunk)
    # 元组返回形状，cast 对齐运行时实际产出（与 main.py 同理）
    initial_state: dict[str, Any] = {"request": request.model_dump(by_alias=True)}
    if outline is not None:
        initial_state["outline"] = outline
    if regen_scope is not None:
        initial_state["regen_scope"] = regen_scope

    stream = cast(
        AsyncIterator[tuple[str, Any]],
        agent_graph.astream(
            initial_state,
            # custom：节点经 get_stream_writer() 直发事件（重生成链的 chapterStart）
            stream_mode=["updates", "values", "custom"],
        ),
    )
    final_state: dict[str, Any] = {}
    try:
        async for mode, chunk in stream:
            if mode == "custom":
                # 节点直发事件（chapterStart）：形状已与 wire 一致，原样透传
                yield chunk
            elif mode == "updates":
                for node, update in chunk.items():
                    # 图终态哨兵（如 __end__）的增量可能非 dict，直接跳过
                    if not isinstance(update, dict):
                        continue
                    if node == "outline" and "outline" in update:
                        outline = update["outline"]
                        yield {
                            "type": "outline",
                            "phaseCount": len(outline.get("phases") or []),
                            "title": outline.get("title", ""),
                            "overview": outline.get("overview", ""),
                            "phases": [
                                {
                                    "name": phase.get("name", ""),
                                    "weekRange": phase.get("weekRange", ""),
                                    "chapterTitles": phase.get("chapterTitles") or [],
                                }
                                for phase in outline.get("phases") or []
                            ],
                            # 完整大纲（含章型 / 知识点 / 阶段目标 / 版本事实 /
                            # 项目简介）：骨架字段之外的重生成上下文由 Rust 原样
                            # 存进 plans.outline_json，regen 时读回。缺了它，重生成
                            # 只能拿章标题当唯一检索词、章型兜底成 concept、版本事实
                            # 丢失——正文质量会明显低于首次生成。
                            "outline": outline,
                        }
                    elif node in ("expand_chapter", "expand_revision") and (
                        "chapter_drafts" in update
                    ):
                        # 章级扇出：每章实例产一条 chapter 事件（与 WS 重生成单章
                        # 事件同构；回炉轮返回的增量仅含重扩成功的章）
                        traces = update.get("chapter_traces") or {}
                        for key, chapter in update["chapter_drafts"].items():
                            phase_index, chapter_index = (
                                int(part) for part in key.split(":")
                            )
                            trace = traces.get(key) or {}
                            yield {
                                "type": "chapter",
                                "index": chapter_index,
                                "phaseIndex": phase_index,
                                "chapter": chapter,
                                # 生成输入留档：素材供重生成复用，提示词变量
                                # 快照供复原完整提示词。Rust 落专门留档表。
                                "researchSnippets": trace.get("researchSnippets") or [],
                                "promptTrace": trace.get("promptTrace"),
                            }
                    elif node == "assemble" and "plan" in update:
                        # 拼装即推完整阶段（含全章，消费方 upsert_phase 幂等覆盖）；
                        # 回炉后第二轮 assemble 会再推一次，旧语义不变。
                        # 重生成作用域下 plan.phases 只有目标阶段，索引必须用
                        # assemble 回填的真实值（enumerate 会从 0 起错位）。
                        offset = update.get("phase_event_index")
                        for index, phase in enumerate(
                            update["plan"].get("phases") or []
                        ):
                            yield {
                                "type": "phase",
                                "index": offset if offset is not None else index,
                                "phase": phase,
                            }
            else:  # values：整图状态快照，记录最新一份作为终态
                final_state = chunk
    except AgentServiceError as exc:
        # 业务失败不中断：翻译为 error 终态（200 语义 + code=service）
        logger.warning("流式生成业务失败：%s", exc)
        response = PlanResponse(
            status="error", error=ErrorDetail(code="service", message=str(exc))
        )
    else:
        response = build_response(final_state)

    # 终态包络：mode="json" 先落成 JSON 安全标量；plan / clarification / error
    # 三种终态统一走 done 收尾
    yield {
        "type": "done",
        "response": response.model_dump(mode="json", by_alias=True),
    }


async def _regen_event_dicts(payload: RegenRequest) -> AsyncIterator[dict[str, Any]]:
    """阶段/章节级重生成事件流。

    与首次生成走**同一条图流水线**（`_plan_event_dicts`），只换初始状态：
    大纲来自库（不经 outline 节点）、作用域限定在目标阶段/章节。因此校验、
    评审、定向回炉、并发全部复用生成链的同一套实现——不再有第二份编排。

    越界索引仍走 done(error, code=validation)（不抛 4xx）；done 包络只报成败
    （plan 置空），数据已由前置 chapter / phase 事件携带，与原 wire 契约一致。
    """
    outline_dict = payload.outline.model_dump(by_alias=True)
    phases = outline_dict.get("phases") or []
    phase_index = payload.phase_index
    if not 0 <= phase_index < len(phases):
        yield _validation_done(
            f"重生成目标阶段越界：phaseIndex={phase_index}，大纲共 {len(phases)} 个阶段"
        )
        return

    chapter_titles = phases[phase_index].get("chapterTitles") or []
    chapter_index = payload.chapter_index
    if chapter_index is not None and not 0 <= chapter_index < len(chapter_titles):
        yield _validation_done(
            f"重生成目标章节越界：phaseIndex={phase_index} 的 chapterIndex="
            f"{chapter_index}，该阶段共 {len(chapter_titles)} 个章节"
        )
        return

    scope = {
        "phase": phase_index,
        "chapters": None if chapter_index is None else [chapter_index],
    }
    async for event in _plan_event_dicts(
        payload.request, outline=outline_dict, regen_scope=scope
    ):
        if event.get("type") == "done":
            # 保持既有 wire 契约：done 只报成败，不携带计划体
            response = dict(event.get("response") or {})
            response["plan"] = None
            event = {**event, "response": response}
        yield event


def _validation_done(message: str) -> dict[str, Any]:
    """参数层面失败（越界等）的 done 事件 dict（code=validation）。"""
    response = PlanResponse(
        status="error", error=ErrorDetail(code="validation", message=message)
    )
    return {"type": "done", "response": response.model_dump(mode="json", by_alias=True)}


# ---------------------------------------------------------------------------
# 模块级单例：main.py 的任务化端点与测试共用同一 registry 实例
# ---------------------------------------------------------------------------

registry = TaskRegistry()
