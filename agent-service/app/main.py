"""FastAPI 入口：Praxis Agent 服务。

端点：
- POST /api/regenerate/stream：阶段/章节级重生成流式推送（phase/chapter/done 事件，
  供 Rust 侧 upsert 已入库的失败阶段/章节）
- POST /api/chat：研读页按章伴学问答（chatDelta/chatDone/chatError NDJSON 流式）
- GET  /api/config：返回当前生效配置（设置页读取）
- POST /api/config：热更新配置（设置页保存后由 Rust 推送，无需重启）
- GET  /health：健康检查（Tauri 端 ping_agent 探测在线状态）

协议约定：
- 请求 / 响应均为 camelCase JSON；
- 响应始终是统一包络 PlanResponse（status = plan | partial | clarification | error），
  即使发生业务错误也返回 200 + envelope，便于前端统一处理；
- 仅校验失败（422 类）与未捕获异常分别转换为 envelope 与 500 envelope。
"""

from __future__ import annotations

import asyncio
import json
import logging
from collections.abc import AsyncIterator
from typing import Any

from fastapi import FastAPI, HTTPException, Request, WebSocket
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from langchain_core.messages import HumanMessage, SystemMessage
from pydantic import ValidationError

from app import prompts, tasks
from app.config import settings
from app.errors import AgentServiceError, register_error_handlers
from app.llm import get_chat_model, has_llm_config, probe_llm, reset_chat_model_cache
from app.logging_config import setup_logging, uvicorn_log_config
from app.nodes import _NO_KEY_HINT
from app.schemas import (
    AgentSettingsModel,
    ChatRequest,
    GenerationSubmit,
    HealthResponse,
    RegenRequest,
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
    version="0.1.2",
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


@app.post("/api/regenerate/stream")
async def regenerate_stream(payload: RegenRequest) -> StreamingResponse:
    """阶段/章节级重生成流式端点（NDJSON，done 必为最后一行）。

    事件序列（application/x-ndjson，每行一个 JSON 对象）：
    - chapterIndex 缺省（全阶段）：{"type": "phase", "index": i, "phase": {…}} 1 条，
      phase 为 StudyPhase 的 camelCase dict（与后台生成任务的 phase 事件同构，
      Rust 侧直接复用 upsert_phase）；
    - chapterIndex 指定（单章）：{"type": "chapter", "index": c,
      "phaseIndex": i, "chapter": {…}} 1 条，chapter 为 Chapter 的 camelCase dict
      （by_alias，供 Rust 侧单章 upsert）；
    - {"type": "done", "response": {…统一包络…}}：终态，必为最后一行；
      成功时包络 status=plan（数据已由 phase/chapter 事件携带），失败时 status=error。

    越界 phaseIndex / chapterIndex 属业务失败：不抛 4xx，走 done(error) 包络
    （code=validation）；AgentServiceError 同样翻译为 error 终态，与
    _plan_event_dicts 事件管道的捕获模式一致；意外异常不在此捕获，仍由
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
# 研读页伴学问答（2026-11）：POST /api/chat，NDJSON 流式（token 级推送）
# ---------------------------------------------------------------------------


@app.post("/api/chat")
async def chat(payload: ChatRequest) -> StreamingResponse:
    """章节伴学聊天流式端点（NDJSON，chatDone / chatError 必为最后一行）。

    事件序列（application/x-ndjson，每行一个 JSON 对象）：
    - {"type": "chatDelta", "text": "…"}：回答文本增量（token 级，可多条）；
    - {"type": "chatDone", "message": {role: "assistant", content: "…"}}：
      正常终态，message 为完整回答（与增量拼接结果一致）；
    - {"type": "chatError", "code": "service|validation", "message": "…"}：
      失败终态（未配置 Key / 模型调用失败），必为最后一行。

    业务失败不抛 4xx（与生成链路的包络哲学一致）：走 chatError 事件收尾。
    请求体校验失败（422）仍由 FastAPI 默认处理——Rust 侧按非 2xx 归一。
    聊天历史持久化由 Rust 负责（chapter_chats 表）：本端点只依据请求携带的
    messages 上下文作答，成功后由调用方把本轮问答落库。
    """
    logger.info(
        "收到研读聊天请求：planId=%s phaseIndex=%s chapterIndex=%s history=%d",
        payload.plan_id,
        payload.phase_index,
        payload.chapter_index,
        len(payload.messages),
    )
    return StreamingResponse(
        _stream_chat_events(payload), media_type="application/x-ndjson"
    )


def _chat_history_text(messages: list) -> str:
    """把历史对话渲染为提示词文本段（无历史时给占位说明）。"""
    if not messages:
        return "（本章暂无历史对话）"
    role_labels = {"user": "学习者", "assistant": "助手"}
    return "\n".join(
        f"{role_labels.get(msg.role, msg.role)}：{msg.content}" for msg in messages
    )


def _chat_context(payload: ChatRequest) -> dict[str, str]:
    """聊天提示词变量：上下文缺省时给可读占位，正文为空按提示词第 5 条口径。"""
    content = payload.chapter_content_md.strip()
    return {
        "plan_title": payload.plan_title.strip() or "（未提供计划标题）",
        "plan_goal": payload.plan_goal.strip() or "（未提供学习目标）",
        "chapter_title": payload.chapter_title.strip() or "（未提供章节标题）",
        "chapter_content_md": content
        or "（本章正文尚未生成：仅依据计划上下文与章节标题作答，"
        "并提醒学习者先在正文区生成该章内容）",
        "history": _chat_history_text(payload.messages),
        "question": payload.question.strip(),
    }


async def _chat_reply_stream(payload: ChatRequest) -> AsyncIterator[str]:
    """逐段产出回答文本增量（mock / 真实 LLM 双路径）。

    mock 模式：确定性分片回复（验证流式链路，同输入同输出，不调 LLM）。
    真实路径：普通文本补全（无结构化输出）直接 astream，chunk.content 逐段产出。
    """
    if settings.allow_mock_llm:
        reply = (
            f"（离线演示回答）关于「{payload.chapter_title or '本章'}」，"
            f"针对你的提问「{payload.question[:40]}」，"
            "正文区域生成内容后，这里会基于该章正文给出扎根原文的解答。"
        )
        for index in range(0, len(reply), 16):
            yield reply[index : index + 16]
        return
    if not has_llm_config():
        raise AgentServiceError(_NO_KEY_HINT)

    template = prompts.load_prompt("chat")
    variables = _chat_context(payload)
    messages = [
        SystemMessage(content=template["system"]),
        HumanMessage(content=prompts.render(template["human"], **variables)),
    ]
    async for chunk in get_chat_model().astream(messages):
        content = chunk.content
        if isinstance(content, str) and content:
            yield content
        elif isinstance(content, list):
            # 多模态分块形态（本链路纯文本，防御性取文本块）
            for block in content:
                text = block.get("text") if isinstance(block, dict) else None
                if isinstance(text, str) and text:
                    yield text


async def _stream_chat_events(payload: ChatRequest) -> AsyncIterator[str]:
    """chat NDJSON 事件管道：chatDelta×N → chatDone；业务失败 → chatError 收尾。

    意外异常同样翻译为 chatError（流式响应内无法改状态码，终态事件是
    消费方判断成败的唯一凭借——与生成链 done(error) 同哲学）。
    """
    full_text_parts: list[str] = []
    try:
        async for text in _chat_reply_stream(payload):
            full_text_parts.append(text)
            yield _ndjson_line({"type": "chatDelta", "text": text})
    except AgentServiceError as exc:
        logger.warning("研读聊天业务失败：%s", exc)
        yield _ndjson_line(
            {"type": "chatError", "code": "service", "message": str(exc)}
        )
        return
    except Exception as exc:  # 流内异常必须落终态事件（ruff: E722 由 except Exception 明确）
        logger.exception("研读聊天意外失败")
        yield _ndjson_line(
            {"type": "chatError", "code": "internal", "message": f"服务内部错误：{exc}"}
        )
        return
    yield _ndjson_line(
        {
            "type": "chatDone",
            "message": {"role": "assistant", "content": "".join(full_text_parts)},
        }
    )


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
        raise HTTPException(
            status_code=422, detail=f"请求体不是合法 JSON：{exc}"
        ) from exc
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
            not pending and entry.status is not TaskStatus.running and entry.done
        ):
            break
        await asyncio.sleep(0.05)
    await ws.close()


async def _ws_send(ws: WebSocket, seq: int, event: dict[str, Any]) -> None:
    """发送一条信封消息：连接断开时 send 抛异常，端点自然退出。"""
    await ws.send_json({"seq": seq, "event": event})


# _ndjson_line 为 /api/regenerate/stream（Rust 重生成链在用）专用的序列化 helper。
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


@app.post("/api/llm-check")
async def llm_check() -> dict[str, object]:
    """生成前的 LLM 连通性探测（Rust 在本机生成/重生成前调用）。

    放在提交生成之前，是为了让「Key 无效 / 网络不通 / 模型名写错」这类问题
    立刻以一句可读的话告知用户，而不是等生成流跑到第一次模型调用才失败
    ——那时用户已经等了十几秒，且 generating 行已经建好。
    永远返回 HTTP 200，成败由 ok 字段表达。
    """
    ok, message = await probe_llm()
    logger.info("LLM 连通性探测 ok=%s provider=%s", ok, settings.llm_provider)
    return {"ok": ok, "message": message, "provider": settings.llm_provider}


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
