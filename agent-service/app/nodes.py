"""LangGraph 节点实现：clarify（澄清）→ plan（规划）→ review（评审）。

约定：
- 每个节点接收状态 dict，返回需要合并进状态的"增量 dict"；
- 所有提示词文案均来自 prompts/*.yaml，代码中不硬编码；
- LLM 输出通过 with_structured_output 强制为 Pydantic 模型（JSON）；
- mock 模式 / 未配置 API Key 时有明确的降级路径，保证服务可用或报错清晰。
"""

from __future__ import annotations

import json
import logging
from typing import Any, TypedDict, cast

from langchain_core.messages import HumanMessage, SystemMessage
from pydantic import ValidationError

from app import prompts
from app.config import settings
from app.llm import get_chat_model, has_llm_config
from app.mock import build_mock_plan
from app.schemas import (
    ClarificationOutput,
    PlanRequest,
    ReviewOutput,
    StudyPlan,
)


class AgentServiceError(RuntimeError):
    """业务错误：会被 FastAPI 转成 status=error 的统一包络返回给前端。"""


class AgentState(TypedDict, total=False):
    """LangGraph 状态：在节点之间流动的字段集合。

    total=False 表示所有键都可选（各节点只读/写自己关心的字段）。
    """

    request: dict[str, Any]  # PlanRequest 的 camelCase dict
    clarifications: list[str]  # clarify 产出：非空则流程终止，返回澄清问题
    plan: dict[str, Any]  # plan 产出：StudyPlan 的 camelCase dict
    passed: bool  # review 产出：计划是否通过质量评审
    review_notes: str  # review 产出：问题与建议（回炉时作为重新规划的输入）
    revised: bool  # 是否已按评审意见修订过（保证最多回炉一次，防止死循环）


def _request_of(state: AgentState) -> PlanRequest:
    """从状态中恢复 PlanRequest（camelCase dict -> 模型）。

    注：入口固定注入 request，运行时必然存在；用 .get() 仅为满足
    total=False TypedDict 的静态检查。
    """
    return PlanRequest.model_validate(state.get("request") or {})


# ---------------------------------------------------------------------------
# 节点 1：clarify —— 检查输入是否完整，不完整则产出澄清问题
# ---------------------------------------------------------------------------


def _detect_issues(req: PlanRequest) -> list[tuple[str, str]]:
    """基于规则的完整性检查，返回 (问题代码, 描述) 列表。

    问题代码对应 clarify.yaml 中的 fallback_questions 键，
    供 LLM 不可用时的降级文案使用。
    """
    issues: list[tuple[str, str]] = []

    if len(req.goal.strip()) < 4:
        issues.append(
            ("goal_too_short", f"学习目标「{req.goal}」过于简短，难以据此规划")
        )
    if not 1 <= req.weekly_hours <= 80:
        issues.append(
            (
                "weekly_hours_out_of_range",
                f"每周可用小时数 {req.weekly_hours} 超出合理范围（1-80）",
            )
        )
    if not 1 <= req.weeks <= 52:
        issues.append(
            ("weeks_out_of_range", f"期望周数 {req.weeks} 超出合理范围（1-52）")
        )
    if not req.background.strip():
        issues.append(("background_missing", "未描述已有基础，无法校准学习起点"))

    return issues


logger = logging.getLogger("praxis.nodes")


async def clarify_node(state: AgentState) -> dict[str, Any]:
    """澄清节点：规则检查发现信息缺口时，让 LLM 生成友好的澄清问题。

    降级策略：mock 模式或 LLM 未配置/调用失败时，直接使用
    clarify.yaml 中 fallback_questions 的内置文案。
    """
    req = _request_of(state)
    issues = _detect_issues(req)

    # 输入完整 → 空列表，条件边将放行到 plan 节点
    if not issues:
        return {"clarifications": []}
    logger.info("检测到 %d 个信息缺口，触发澄清", len(issues))

    fallback = prompts.fallback_questions("clarify")
    fallback_answers = [fallback.get(code, desc) for code, desc in issues]

    # mock / 未配置 Key：跳过 LLM，直接用降级文案
    if settings.allow_mock_llm or not has_llm_config():
        return {"clarifications": fallback_answers}

    # 正常路径：LLM 生成更自然的澄清问题
    try:
        template = prompts.load_prompt("clarify")
        structured = get_chat_model().with_structured_output(
            ClarificationOutput, method="function_calling"
        )
        # ainvoke 静态签名宽泛（dict | BaseModel），运行时必为 ClarificationOutput
        output = cast(
            ClarificationOutput,
            await structured.ainvoke(
                [
                    SystemMessage(content=template["system"]),
                    HumanMessage(
                        content=prompts.render(
                            template["human"],
                            request=json.dumps(
                                state.get("request", {}), ensure_ascii=False
                            ),
                            issues="\n".join(f"- {desc}" for _, desc in issues),
                        )
                    ),
                ]
            ),
        )
        questions = [q.strip() for q in output.questions if q.strip()]
        # LLM 返回为空时仍回退到内置文案
        return {"clarifications": questions or fallback_answers}
    except Exception:  # noqa: BLE001 —— 澄清是辅助能力，任何失败都不应阻断主流程
        return {"clarifications": fallback_answers}


# ---------------------------------------------------------------------------
# 节点 2：plan —— 调用 LLM 生成结构化学习计划
# ---------------------------------------------------------------------------


async def plan_node(state: AgentState) -> dict[str, Any]:
    """规划节点：用 plan.yaml 提示词 + 结构化输出生成 StudyPlan。

    若状态中带有 review_notes（说明是评审回炉），会将其注入提示词，
    要求 LLM 逐条落实评审意见，并标记 revised=True。
    """
    req = _request_of(state)
    review_notes: str = state.get("review_notes", "")
    revised = bool(review_notes)

    # mock 模式：确定性计划，不调用 LLM
    if settings.allow_mock_llm:
        return {"plan": build_mock_plan(state.get("request", {})), "revised": revised}

    if not has_llm_config():
        raise AgentServiceError(
            "未配置 LLM API Key：请复制 agent-service/.env.example 为 .env 并填入 "
            "OPENAI_API_KEY（或 ANTHROPIC_API_KEY）；"
            "或设置 PRAXIS_ALLOW_MOCK_LLM=true 使用离线演示模式。"
        )

    template = prompts.load_prompt("plan")
    structured = get_chat_model().with_structured_output(
        StudyPlan, method="function_calling"
    )
    messages = [
        SystemMessage(content=template["system"]),
        HumanMessage(
            content=prompts.render(
                template["human"],
                goal=req.goal,
                level=req.level.value,
                weekly_hours=str(req.weekly_hours),
                weeks=str(req.weeks),
                interests="、".join(req.interests) if req.interests else "未填写",
                background=req.background or "未填写",
                review_notes=review_notes or "（首次生成，无评审意见）",
            )
        ),
    ]

    # glm-5.3 等思考模型对大嵌套 schema 的工具参数偶发拼接错乱
    # （字段丢失 / 类型漂移），解析失败自动重试一次
    plan: StudyPlan | None = None
    for attempt in range(2):
        try:
            # ainvoke 静态签名宽泛（dict | BaseModel），运行时必为 StudyPlan
            plan = cast(StudyPlan, await structured.ainvoke(messages))
            break
        except ValidationError:
            logger.warning(
                "计划结构化输出校验失败（第 %d 次），%s",
                attempt + 1,
                "重试" if attempt == 0 else "放弃本次生成",
            )

    # 回炉场景的兑底：重试仍失败时回退到上一轮已成功解析的计划，
    # 避免「评审回炉 → 生成损坏 → 整个请求 500」的链路
    if plan is None:
        previous = state.get("plan")
        if revised and previous:
            logger.warning("回炉生成失败，回退到上一轮计划")
            return {"plan": previous, "revised": True}
        raise AgentServiceError(
            "学习计划生成失败：模型返回内容无法解析为结构化计划，请重试。"
        )

    # 以 camelCase JSON 形式存入状态（与响应包络直接对齐）
    return {"plan": plan.model_dump(mode="json", by_alias=True), "revised": revised}


# ---------------------------------------------------------------------------
# 节点 3：review —— 第二次 LLM 调用，评审计划质量
# ---------------------------------------------------------------------------


async def review_node(state: AgentState) -> dict[str, Any]:
    """评审节点：从结构完整性 / 验收标准可测性 / 时间可行性 / 难度递进四个维度审查。

    未通过时条件边会把流程送回 plan 节点（最多一次）。
    """
    # mock / 未配置 Key：跳过评审直接放行
    if settings.allow_mock_llm:
        return {"passed": True, "review_notes": "（Mock 模式：未执行真实 LLM 评审）"}
    if not has_llm_config():
        return {"passed": True, "review_notes": "（未配置 API Key：跳过评审）"}

    template = prompts.load_prompt("review")
    structured = get_chat_model().with_structured_output(
        ReviewOutput, method="function_calling"
    )
    # ainvoke 静态签名宽泛（dict | BaseModel），运行时必为 ReviewOutput
    output = cast(
        ReviewOutput,
        await structured.ainvoke(
            [
                SystemMessage(content=template["system"]),
                HumanMessage(
                    content=prompts.render(
                        template["human"],
                        plan=json.dumps(
                            state.get("plan", {}), ensure_ascii=False, indent=2
                        ),
                    )
                ),
            ]
        ),
    )

    logger.info(
        "评审完成 passed=%s issues=%d suggestions=%d",
        output.passed,
        len(output.issues),
        len(output.suggestions),
    )
    notes = "\n".join([*output.issues, *output.suggestions]).strip()
    return {
        "passed": output.passed,
        "review_notes": notes or "计划评审通过，未发现问题。",
    }
