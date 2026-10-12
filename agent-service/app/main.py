"""FastAPI 入口：Praxis Agent 服务。

端点：
- POST /api/generate-plan：生成学习计划（分层拓扑：outline → 并行扩展 → 拼装 → 评审）
- POST /api/generate-plan/stream：同上，NDJSON 流式推送（outline/phase/done 事件，
  供 Rust 侧逐阶段入库；done 必为最后一行）
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

import json
import logging
from collections.abc import AsyncIterator
from typing import Any, cast

from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse

from app.config import settings
from app.errors import register_error_handlers
from app.graph import agent_graph
from app.llm import reset_chat_model_cache
from app.logging_config import setup_logging, uvicorn_log_config
from app.schemas import (
    AgentSettingsModel,
    HealthResponse,
    PlanRequest,
    PlanResponse,
    StudyPlan,
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
    非流式端点由全局 exception_handler 转成 200 + status=error 包络。
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
async def generate_plan(request: PlanRequest) -> PlanResponse:
    """生成学习计划：驱动 LangGraph 状态图并组装统一包络（一次性返回）。"""
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
async def generate_plan_stream(request: PlanRequest) -> StreamingResponse:
    """流式生成学习计划：NDJSON 逐行推送事件，每阶段完成即到达。

    事件序列（application/x-ndjson，每行一个 JSON 对象）：
    - {"type": "outline", "phaseCount": N, "title": "…"}：骨架到达时推 1 条；
    - {"type": "phase", "index": i, "phase": {…}}：每阶段完成推 1 条，
      可能乱序，回炉重生成时同 index 再次推送（覆盖语义）；
    - {"type": "done", "response": {…统一包络…}}：终态，必为最后一行
      （clarification 终态同样走 done，此时无 outline / phase 事件）。

    生成器不做 try/except 吞错：异常直接抛出由 Starlette 中断流，
    传输层错误由消费方处理；请求校验失败（422）发生在流开始前，无需处理。
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


async def _stream_plan_events(request: PlanRequest) -> AsyncIterator[str]:
    """驱动图流式执行，把节点增量翻译为 NDJSON 事件行。"""
    # langgraph v1 astream 的 Sequence stream_mode 重载未建模 (mode, chunk)
    # 元组返回形状，cast 对齐运行时实际产出（与 graph.py 的签名误报处理同理）
    stream = cast(
        AsyncIterator[tuple[str, Any]],
        agent_graph.astream(
            {"request": request.model_dump(by_alias=True)},
            stream_mode=["updates", "values"],
        ),
    )
    final_state: dict[str, Any] = {}
    async for mode, chunk in stream:
        if mode == "updates":
            for node, update in chunk.items():
                # 图终态哨兵（如 __end__）的增量可能非 dict，直接跳过
                if not isinstance(update, dict):
                    continue
                # outline 节点增量到达：推 1 条 outline 事件
                if node == "outline" and "outline" in update:
                    outline = update["outline"]
                    yield _ndjson_line(
                        {
                            "type": "outline",
                            "phaseCount": len(outline.get("phases") or []),
                            "title": outline.get("title", ""),
                        }
                    )
                # 并行扩展 / 定向回炉的增量到达：逐阶段推 phase 事件
                # （expand_revision 携带全量草稿，未点名阶段会重复推送，
                # 消费方按 index 覆盖即可，语义不变）
                elif node in ("expand_phase", "expand_revision") and (
                    "phase_drafts" in update
                ):
                    for index, phase in update["phase_drafts"].items():
                        yield _ndjson_line(
                            {"type": "phase", "index": int(index), "phase": phase}
                        )
        else:  # values：整图状态快照，记录最新一份作为终态
            final_state = chunk

    # 终态包络：mode="json" 先落成 JSON 安全标量（枚举 / UUID 安全），
    # clarification / error 终态同样走 done
    response = build_response(final_state)
    yield _ndjson_line(
        {"type": "done", "response": response.model_dump(mode="json", by_alias=True)}
    )


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
