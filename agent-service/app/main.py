"""FastAPI 入口：Praxis Agent 服务。

端点：
- POST /api/generate-plan：生成学习计划（分层拓扑：outline → 并行扩展 → 拼装 → 评审）
- POST /api/generate-plan/stream：同上，NDJSON 流式推送（outline/phase/done 事件，
  供 Rust 侧逐阶段入库；done 必为最后一行）
- POST /api/regenerate/stream：阶段/章节级重生成流式推送（phase/chapter/done 事件，
  供 Rust 侧 upsert 已入库的失败阶段/章节）
- GET  /api/config：返回当前生效配置（设置页读取）
- POST /api/config：热更新配置（设置页保存后由 Rust 推送，无需重启）
- GET  /health：健康检查（Tauri 端 ping_agent 探测在线状态）

协议约定：
- 请求 / 响应均为 camelCase JSON；
- 响应始终是统一包络 PlanResponse（status = plan | clarification | error），
  即使发生业务错误也返回 200 + envelope，便于前端统一处理；
- 仅校验失败（422 类）与未捕获异常分别转换为 envelope 与 500 envelope。
"""

from __future__ import annotations

import asyncio
import json
import logging
import warnings
from collections.abc import AsyncIterator
from typing import Any

from fastapi import FastAPI, HTTPException, Request, WebSocket
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from pydantic import ValidationError

from app import tasks
from app.config import settings
from app.errors import register_error_handlers
from app.graph import agent_graph
from app.llm import reset_chat_model_cache
from app.logging_config import setup_logging, uvicorn_log_config
from app.schemas import (
    AgentSettingsModel,
    GenerationSubmit,
    HealthResponse,
    PlanRequest,
    PlanResponse,
    RegenRequest,
    StudyPlan,
)
from app.tasks import (
    ConcurrencyExceededError,
    TaskKind,
    TaskStatus,
    registry,
)

setup_logging()
logger = logging.getLogger("praxis.main")

app = FastAPI(
    title="Praxis Agent Service",
    version="0.0.1",
    description="学习路线规划 Agent：clarify（澄清）→ plan（规划）→ review（评审）",
)

# 本地桌面应用场景：仅允许 Tauri 的开发 / 生产来源（收紧 CORS，不放通配符）
app.add_middleware(
    CORSMiddleware,
    allow_origins=[
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "tauri://localhost",
        "http://tauri.localhost",
    ],
    allow_methods=["*"],
    allow_headers=["*"],
)

# 全局异常处理：任何异常统一转成 200/500 + status=error 包络（定义见 app/errors.py）
register_error_handlers(app)


# ---------------------------------------------------------------------------
# 请求日志 + 跨进程关联：Rust 壳为每个请求生成 req_id 经 X-Request-ID 头传入，
# 双端日志按 req_id 对齐时间线（格式契约见 src-tauri/src/logging.rs）
@app.middleware("http")
async def _log_requests(request: Request, call_next):
    req_id = request.headers.get("x-request-id", "-")
    logger.info("--> %s %s req_id=%s", request.method, request.url.path, req_id)
    response = await call_next(request)
    logger.info(
        "<-- %s %s %s req_id=%s",
        request.method,
        request.url.path,
        response.status_code,
        req_id,
    )
    return response


# ---------------------------------------------------------------------------
# 业务端点
# ---------------------------------------------------------------------------


def build_response(final_state: dict[str, Any]) -> PlanResponse:
    """把图终态组装成统一响应包络（流式 / 非流式两端点共用的纯函数）。

    路径一：clarify 节点判定信息不完整，返回澄清问题；
    路径二：正常产出计划。
    业务异常不在此处理：可预期异常已由 llm 层翻译为 AgentServiceError，
    非流式端点由全局 exception_handler、流式端点由生成器捕获后
    分别转成 status=error 包络（前者 HTTP 200，后者为 done 终态行）。
    """
    # 路径一：澄清问题非空即 clarification 终态
    if final_state.get("clarifications"):
        return PlanResponse(
            status="clarification",
            clarifications=list(final_state["clarifications"]),
        )

    # 路径二：正常产出计划
    plan = StudyPlan.model_validate(final_state["plan"])
    return PlanResponse(status="plan", plan=plan)


@app.post(
    "/api/generate-plan", response_model=PlanResponse, response_model_by_alias=True
)
@warnings.deprecated(
    "v0.1 G1 清理 —— Rust 已走 WS 后台生成（任务 22），前端不再调用"
)
async def generate_plan(request: PlanRequest) -> PlanResponse:
    """弃用：v0.1 G1 清理移除 —— Rust 已走 WS 后台生成（任务 22），前端不再调用。

    生成学习计划：驱动 LangGraph 状态图并组装统一包络（一次性返回）。
    保留期间行为不变（wire 兼容），仅标记等待 v0.1 完成后统一清理。
    """
    logger.info(
        "收到计划请求：goal=%r level=%s weeklyHours=%s weeks=%s",
        request.goal,
        request.level.value,
        request.weekly_hours,
        request.weeks,
    )

    final_state = await agent_graph.ainvoke(
        {"request": request.model_dump(by_alias=True)}
    )
    response = build_response(final_state)

    if response.status == "clarification":
        logger.info("需要澄清：%d 个问题", len(response.clarifications or []))
    else:
        logger.info(
            "计划生成成功：%s（%d 个阶段）",
            response.plan.title if response.plan else "",
            len(response.plan.phases) if response.plan else 0,
        )
    return response


@app.post("/api/generate-plan/stream")
@warnings.deprecated(
    "v0.1 G1 清理 —— Rust 已走 WS 后台生成（任务 22），前端不再调用"
)
async def generate_plan_stream(request: PlanRequest) -> StreamingResponse:
    """弃用：v0.1 G1 清理移除 —— Rust 已走 WS 后台生成（任务 22），前端不再调用。

    流式生成学习计划：NDJSON 逐行推送事件，每阶段完成即到达。

    事件序列（application/x-ndjson，每行一个 JSON 对象）：
    - {"type": "outline", "phaseCount": N, "title": "…", "overview"?: "…",
   "phases"?: [{"name": "…", "weekRange": "…", "chapterTitles": ["…"]}]}：
  骨架到达时推 1 条（phases 为向后兼容的可选载荷，供渐进式骨架渲染）
    - {"type": "phase", "index": i, "phase": {…}}：每阶段完成推 1 条，
      可能乱序，回炉重生成时同 index 再次推送（覆盖语义）；
    - {"type": "done", "response": {…统一包络…}}：终态，必为最后一行
      （plan / clarification / error 三种终态统一走 done；clarification
      与 error 终态无 outline / phase 事件，error 之前可能已有部分
      outline / phase 事件，消费方以 done.response.status 判定成败）。

    业务异常（AgentServiceError）不中断流：生成器捕获后翻译为 error 包络的
    done 终态，与非流式通道全局 handler 的产出语义一致（见 app/errors.py）；
    意外异常不在此捕获，仍由 Starlette 中断流；请求校验失败（422）发生在
    流开始前，无需处理。
    """
    logger.info(
        "收到流式计划请求：goal=%r level=%s weeklyHours=%s weeks=%s",
        request.goal,
        request.level.value,
        request.weekly_hours,
        request.weeks,
    )
    return StreamingResponse(
        _stream_plan_events(request), media_type="application/x-ndjson"
    )


@warnings.deprecated(
    "v0.1 G1 清理 —— 仅 /api/generate-plan/stream 独占的 NDJSON 薄包装，随端点一并退役"
)
async def _stream_plan_events(request: PlanRequest) -> AsyncIterator[str]:
    """弃用（v0.1 G1 清理）：仅 /api/generate-plan/stream 独占的 NDJSON 薄包装。

    透传 tasks._plan_event_dicts 的 event dict（wire 不变）。事件管道本体
    _plan_event_dicts 已降层到 app/tasks.py 供后台任务与流端点共用（后台生成
    仍在消费，不随本端点退役）；本函数只负责 NDJSON 序列化（ensure_ascii=False
    + 尾行换行），字节级兼容旧线协议。
    """
    async for event in tasks._plan_event_dicts(request):
        yield _ndjson_line(event)


@app.post("/api/regenerate/stream")
async def regenerate_stream(payload: RegenRequest) -> StreamingResponse:
    """阶段/章节级重生成流式端点（NDJSON，done 必为最后一行）。

    事件序列（application/x-ndjson，每行一个 JSON 对象）：
    - chapterIndex 缺省（全阶段）：{"type": "phase", "index": i, "phase": {…}} 1 条，
      phase 为 StudyPhase 的 camelCase dict（与 generate-plan 流的 phase 事件同构，
      Rust 侧直接复用 upsert_phase）；
    - chapterIndex 指定（单章）：{"type": "chapter", "index": c,
      "phaseIndex": i, "chapter": {…}} 1 条，chapter 为 Chapter 的 camelCase dict
      （by_alias，供 Rust 侧单章 upsert）；
    - {"type": "done", "response": {…统一包络…}}：终态，必为最后一行；
      成功时包络 status=plan（数据已由 phase/chapter 事件携带），失败时 status=error。

    越界 phaseIndex / chapterIndex 属业务失败：不抛 4xx，走 done(error) 包络
    （code=validation）；AgentServiceError 同样翻译为 error 终态，与
    _stream_plan_events 的捕获模式一致；意外异常不在此捕获，仍由
    Starlette 中断流。结构合法性由 Pydantic 模型兜底，不跑整计划 validators。
    """
    logger.info(
        "收到重生成请求：phaseIndex=%s chapterIndex=%s",
        payload.phase_index,
        payload.chapter_index,
    )
    return StreamingResponse(
        _stream_regen_events(payload), media_type="application/x-ndjson"
    )


async def _stream_regen_events(payload: RegenRequest) -> AsyncIterator[str]:
    """NDJSON 薄包装：透传 tasks._regen_event_dicts 的 event dict（wire 不变）。"""
    async for event in tasks._regen_event_dicts(payload):
        yield _ndjson_line(event)


# ---------------------------------------------------------------------------
# 生成任务化端点（v7）：提交即返 + 快照 + 取消 + WS 游标回放/直播
# ---------------------------------------------------------------------------


@app.post("/api/generations")
async def create_generation(request: Request):
    """提交后台生成任务：毫秒级返回 {taskId}，生成由 Python 后台任务跑完整图。

    连接断开 / 页面关闭 / Rust 重启都不影响生成；进度经 WS 事件通道查询。
    并发池（plan/regen 共享 MAX_CONCURRENT）占满返回 429（中文 message）。
    """
    try:
        body = await request.json()
    except Exception as exc:  # 非 JSON / 解析失败统一 422
        raise HTTPException(status_code=422, detail=f"请求体不是合法 JSON：{exc}") from exc
    kind, payload = _parse_generation_submit(body)
    try:
        task_id = await registry.submit(kind, payload)
    except ConcurrencyExceededError as exc:
        logger.warning("提交被拒（容量不足）：%s", exc)
        raise HTTPException(status_code=429, detail=str(exc)) from exc
    return {"taskId": task_id}


@app.get("/api/generations/{task_id}/snapshot")
async def generation_snapshot(task_id: str):
    """任务状态摘要：{taskId, kind, status, firstSeq, lastSeq, done}；不存在 404。"""
    entry = registry.get(task_id)
    if entry is None:
        raise HTTPException(status_code=404, detail="任务不存在或已驱逐")
    return registry.snapshot(entry)


@app.delete("/api/generations/{task_id}")
async def cancel_generation(task_id: str):
    """幂等 cancel：running → 取消（done(error, cancelled) 由 registry.cancel 同步落日志）；

    已终态任务原样返回当前状态（200）；不存在返回 404。
    """
    try:
        entry = await registry.cancel(task_id)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail="任务不存在或已驱逐") from exc
    return {"taskId": entry.task_id, "status": entry.status.value}


def _parse_generation_submit(body: Any) -> tuple[TaskKind, Any]:
    """提交体显式校验（kind + 二选一载荷），错配翻译为 422（不走全局 200 包络）。

    返回 (kind, payload)，payload 为与 kind 配对的 PlanRequest / RegenRequest。
    """
    try:
        submitted = GenerationSubmit.model_validate(body)
    except ValidationError as exc:
        errors = exc.errors()[:3]
        raise HTTPException(
            status_code=422, detail=f"请求参数不合法：{errors}"
        ) from exc
    return TaskKind(submitted.kind), submitted.payload()


@app.websocket("/api/generations/{task_id}/events")
async def generation_events(ws: WebSocket, task_id: str, cursor: int = 0) -> None:
    """事件通道（WS）：连接后先回放 seq>cursor 缓存，再直播新事件直至 done。

    每条消息信封 {"seq": n, "event": {...}}，event 与 NDJSON 行同构；
    回放窗口在 per-task lock 下快照衔接，不丢不重；心跳依赖 uvicorn 协议层
    ping/pong（默认 ws_ping_interval），应用层不自制心跳；任务不存在 /
    已驱逐 → 接受后立即以 4404 关闭（不做鉴权：localhost sidecar）。
    """
    await ws.accept()
    entry = registry.get(task_id)
    if entry is None:
        await ws.close(code=4404, reason="任务不存在或已驱逐")
        return

    seq_sent = cursor  # 回放下界：cursor 之后的缓存才回放（不丢不重）
    done_sent = False
    while True:
        # 每轮在 per-task lock 内快照：新事件的追加同样持锁，衔接无缝
        async with registry.lock_for(task_id):
            pending = [(seq, event) for seq, event in entry.events if seq > seq_sent]
        for seq, event in pending:
            await _ws_send(ws, seq, event)
            seq_sent = seq
            if event.get("type") == "done":
                done_sent = True
        if done_sent or (
            not pending
            and entry.status is not TaskStatus.running
            and entry.done
        ):
            break
        await asyncio.sleep(0.05)
    await ws.close()


async def _ws_send(ws: WebSocket, seq: int, event: dict[str, Any]) -> None:
    """发送一条信封消息：连接断开时 send 抛异常，端点自然退出。"""
    await ws.send_json({"seq": seq, "event": event})


# _ndjson_line 为 /api/generate-plan/stream 与 /api/regenerate/stream 共用的
# 序列化 helper（regenerate 未弃用，Rust 重生成链仍在消费），故不随弃用端点打标。
def _ndjson_line(event: dict[str, Any]) -> str:
    """单个事件序列化为一行 NDJSON（ensure_ascii=False 保留中文原文）。"""
    return json.dumps(event, ensure_ascii=False) + "\n"


@app.get("/api/config", response_model=AgentSettingsModel)
async def get_config() -> AgentSettingsModel:
    """返回当前生效配置（API Key 原文：桌面单用户本机场景）。"""
    return AgentSettingsModel(**settings.public_dict())


@app.post("/api/config", response_model=AgentSettingsModel)
async def update_config(payload: AgentSettingsModel) -> AgentSettingsModel:
    """热更新配置：就地更新 settings 单例并重建 LLM 客户端，立即生效。

    日志刻意不记录 API Key；监听地址 / 端口不属于热更范围（进程生命周期属性）。
    """
    settings.update_runtime(payload.model_dump(by_alias=True, exclude_unset=True))
    reset_chat_model_cache()
    logger.info(
        "配置已热更新：provider=%s openaiModel=%s anthropicModel=%s mock=%s",
        settings.llm_provider,
        settings.openai_model,
        settings.anthropic_model,
        settings.allow_mock_llm,
    )
    return AgentSettingsModel(**settings.public_dict())


@app.get("/health", response_model=HealthResponse)
async def health() -> HealthResponse:
    """健康检查：供 Tauri 端启动时探测服务在线状态。"""
    return HealthResponse()


# ---------------------------------------------------------------------------
# 本地直跑入口：python -m app.main（生产式直跑；热重载开发请用 npm run dev / uvicorn --reload）
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    import uvicorn

    uvicorn.run(  # type: ignore[call-overload]
        "app.main:app",
        host=settings.host,
        port=settings.port,
        # dict 运行时完全合法；仅静态不满足 uvicorn 的 TypedDict 字面量检查。
        # ignore 须放在报错行（调用起始行）而非参数行
        # 不开 reload：热重载属于 dev.mjs 的 uvicorn --reload 职责，此处保持直跑语义
        log_config=uvicorn_log_config(),
    )
