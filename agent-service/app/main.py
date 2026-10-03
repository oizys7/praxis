"""FastAPI 入口：Praxis Agent 服务。

端点：
- POST /api/generate-plan：生成学习计划（clarify → plan → review）
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

import logging

import httpx
from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from app.config import settings
from app.graph import agent_graph
from app.llm import reset_chat_model_cache
from app.logging_config import setup_logging, uvicorn_log_config
from app.nodes import AgentServiceError
from app.schemas import (
    AgentSettingsModel,
    ErrorDetail,
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


@app.post(
    "/api/generate-plan", response_model=PlanResponse, response_model_by_alias=True
)
async def generate_plan(
    request: PlanRequest, http_request: Request
) -> PlanResponse:
    """生成学习计划：驱动 LangGraph 状态图并组装统一包络。"""
    logger.info(
        "收到计划请求：goal=%r level=%s weeklyHours=%s weeks=%s",
        request.goal,
        request.level.value,
        request.weekly_hours,
        request.weeks,
    )

    # LangGraph 执行：clarify → plan → review（mock / 真实 LLM 由配置决定）
    # LLM 超时 / 网络错误是可预期的业务失败，归一为 200 + status=error 包络（不裸 500）
    try:
        final_state = await agent_graph.ainvoke(
            {"request": request.model_dump(by_alias=True)}
        )
    except AgentServiceError:
        raise  # 业务错误已有统一 handler
    except (httpx.TimeoutException, TimeoutError) as exc:
        logger.error(
            "LLM 响应超时 req_id=%s",
            http_request.headers.get("x-request-id", "-"),
            exc_info=exc,
        )
        raise AgentServiceError(
            "模型响应超时，请稍后重试，或在设置页更换更快的模型"
        ) from exc
    except httpx.HTTPError as exc:
        logger.error(
            "LLM 网络错误 req_id=%s",
            http_request.headers.get("x-request-id", "-"),
            exc_info=exc,
        )
        raise AgentServiceError(
            "无法连接模型服务，请检查网络连接与 API Key"
        ) from exc

    # 路径一：clarify 节点判定信息不完整，返回澄清问题
    if final_state.get("clarifications"):
        logger.info("需要澄清：%d 个问题", len(final_state["clarifications"]))
        return PlanResponse(
            status="clarification",
            clarifications=list(final_state["clarifications"]),
        )

    # 路径二：正常产出计划
    plan = StudyPlan.model_validate(final_state["plan"])
    logger.info("计划生成成功：%s（%d 个阶段）", plan.title, len(plan.phases))
    return PlanResponse(status="plan", plan=plan)


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
# 异常处理：统一转成 PlanResponse 包络
# ---------------------------------------------------------------------------


def _error_envelope(code: str, message: str) -> dict:
    """构造包络 dict（camelCase），供 JSONResponse 使用。"""
    return PlanResponse(
        status="error",
        error=ErrorDetail(code=code, message=message),
    ).model_dump(by_alias=True)


@app.exception_handler(AgentServiceError)
async def agent_service_error_handler(
    _: Request, exc: AgentServiceError
) -> JSONResponse:
    """业务错误（如未配置 API Key）：200 + envelope，前端展示友好提示。"""
    logger.warning("业务错误：%s", exc)
    return JSONResponse(status_code=200, content=_error_envelope("service", str(exc)))


@app.exception_handler(RequestValidationError)
async def validation_error_handler(
    _: Request, exc: RequestValidationError
) -> JSONResponse:
    """请求参数校验失败：200 + envelope，列出前 3 条错误。"""
    errors = exc.errors()[:3]
    message = f"请求参数不合法：{errors}"
    logger.warning("参数校验失败：%s", message)
    return JSONResponse(status_code=200, content=_error_envelope("validation", message))


@app.exception_handler(Exception)
async def unexpected_error_handler(_: Request, exc: Exception) -> JSONResponse:
    """未捕获异常：500 + envelope，同时记录完整堆栈。"""
    logger.exception("未处理异常：%s", exc)
    return JSONResponse(
        status_code=500,
        content=_error_envelope("internal", f"服务内部错误：{exc}"),
    )


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
