"""FastAPI 入口：Praxis Agent 服务。

端点：
- POST /api/generate-plan：生成学习计划（clarify → plan → review）
- GET  /health：健康检查（Tauri 端 ping_agent 探测在线状态）

协议约定：
- 请求 / 响应均为 camelCase JSON；
- 响应始终是统一包络 PlanResponse（status = plan | clarification | error），
  即使发生业务错误也返回 200 + envelope，便于前端统一处理；
- 仅校验失败（422 类）与未捕获异常分别转换为 envelope 与 500 envelope。
"""

from __future__ import annotations

import logging

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from app.config import settings
from app.graph import agent_graph
from app.nodes import AgentServiceError
from app.schemas import (
    ErrorDetail,
    HealthResponse,
    PlanRequest,
    PlanResponse,
    StudyPlan,
)

logging.basicConfig(
    level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s"
)
logger = logging.getLogger("praxis")

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
# 业务端点
# ---------------------------------------------------------------------------


@app.post(
    "/api/generate-plan", response_model=PlanResponse, response_model_by_alias=True
)
async def generate_plan(request: PlanRequest) -> PlanResponse:
    """生成学习计划：驱动 LangGraph 状态图并组装统一包络。"""
    logger.info(
        "收到计划请求：goal=%r level=%s weeklyHours=%s weeks=%s",
        request.goal,
        request.level.value,
        request.weekly_hours,
        request.weeks,
    )

    # LangGraph 执行：clarify → plan → review（mock / 真实 LLM 由配置决定）
    final_state = await agent_graph.ainvoke(
        {"request": request.model_dump(by_alias=True)}
    )

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
# 本地直跑入口：python -m app.main（等同 uvicorn app.main:app --reload）
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    import uvicorn

    uvicorn.run("app.main:app", host=settings.host, port=settings.port, reload=True)
