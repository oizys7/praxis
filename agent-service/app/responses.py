"""生成图终态到统一响应包络的映射。"""

from __future__ import annotations

from typing import Any, Literal

from app.schemas import ErrorDetail, PlanResponse, StudyPlan


def _plan_result_status(plan: StudyPlan) -> Literal["plan", "partial", "error"]:
    """按章节正文判定交付成色：全有正文 → plan，部分 → partial，全空 → error。

    这是内容完整性口径，不是质量校验口径；质量门槛由 validators 决定是否回炉。
    """
    chapters = [chapter for phase in plan.phases for chapter in phase.chapters]
    filled_count = sum(bool((chapter.content_md or "").strip()) for chapter in chapters)
    if filled_count == len(chapters) and filled_count > 0:
        return "plan"
    if filled_count:
        return "partial"
    return "error"


def build_response(final_state: dict[str, Any]) -> PlanResponse:
    """将图终态组装为包络；该模块不依赖 FastAPI 端点或任务注册表。"""
    clarifications = final_state.get("clarifications")
    if clarifications:
        return PlanResponse(
            status="clarification",
            clarifications=list(clarifications),
        )

    plan = StudyPlan.model_validate(final_state["plan"])
    status = _plan_result_status(plan)
    if status == "error":
        return PlanResponse(
            status="error",
            plan=plan,
            error=ErrorDetail(
                code="all_chapters_failed",
                message="所有章节都未能生成，已保留大纲骨架，可逐章重新生成",
            ),
        )
    if status == "plan":
        return PlanResponse(status="plan", plan=plan)
    return PlanResponse(status="partial", plan=plan)
