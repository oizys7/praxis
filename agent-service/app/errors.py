"""业务错误定义与其 HTTP 呈现：全链路统一的异常处理出口。

- AgentServiceError：任何「可预期的业务失败」（未配置 Key、模型响应超时、
  输出无法解析等）都应抛出本类型，而不是让底层 SDK 异常裸奔成 500；
  类型定义零框架依赖，nodes / llm 等任意层可安全引用；
- register_error_handlers(app)：把异常统一转成 200/500 + status=error 的
  PlanResponse 包络，由 main.py 启动时注册一次。前端因此只需要处理
  一种错误形态。
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    # 仅供类型标注；运行时 import 延迟到 register_error_handlers 内，
    # 保持本模块类型定义部分的零框架依赖
    from fastapi import FastAPI

logger = logging.getLogger("praxis.errors")


class AgentServiceError(RuntimeError):
    """业务错误：会被 register_error_handlers 转成 status=error 的包络返回。"""


def register_error_handlers(app: FastAPI) -> None:
    """注册全局异常处理器：任何异常都转成统一包络（main.py 启动时调用一次）。"""
    from fastapi import Request
    from fastapi.exceptions import RequestValidationError
    from fastapi.responses import JSONResponse

    from app.schemas import ErrorDetail, PlanResponse

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
        return JSONResponse(
            status_code=200, content=_error_envelope("service", str(exc))
        )

    @app.exception_handler(RequestValidationError)
    async def validation_error_handler(
        _: Request, exc: RequestValidationError
    ) -> JSONResponse:
        """请求参数校验失败：200 + envelope，列出前 3 条错误。"""
        errors = exc.errors()[:3]
        message = f"请求参数不合法：{errors}"
        logger.warning("参数校验失败：%s", message)
        return JSONResponse(
            status_code=200, content=_error_envelope("validation", message)
        )

    @app.exception_handler(Exception)
    async def unexpected_error_handler(_: Request, exc: Exception) -> JSONResponse:
        """未捕获异常：500 + envelope，同时记录完整堆栈。"""
        logger.exception("未处理异常：%s", exc)
        return JSONResponse(
            status_code=500,
            content=_error_envelope("internal", f"服务内部错误：{exc}"),
        )
