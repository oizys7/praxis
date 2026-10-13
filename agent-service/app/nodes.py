"""LangGraph 节点实现：分层生成的编排逻辑。

生成流程分四步（骨架 → 并行展开 → 拼装校验 → 评审与定向回炉）：
- outline_node：先产出计划骨架（PlanOutline，只含阶段大纲，不写章节正文）；
- expand_phase_node：Send 并行逐阶段扩展为 StudyPhase 草稿（并发上限 3）；
- assemble_node：按索引排序拼装 StudyPlan，并跑程序化校验（app.validators）；
- review_node：结合校验问题做 LLM 评审，汇总出待回炉阶段索引，
  由 expand_revision_node 定向回炉（最多一次，条件边在图侧控制）。

节点只写业务编排逻辑（检查输入 → 调 LLM → 产出状态增量）。
LLM 调用细节（提示词渲染、结构化解析、解析重试、传输层异常翻译）
全部收敛在 app.llm.invoke_structured；异常类型定义见 app.errors。

约定：
- 每个节点接收状态（expand_phase_node 例外：收 Send payload dict），
  返回需要合并进状态的「增量 dict」；
- phase_drafts 采用「<阶段索引字符串> -> 阶段 camelCase dict」的整值
  存储：并行扩展时各实例只返回自己负责的键，增量必须携带完整 dict；
- 所有提示词文案均来自 prompts/*.yaml，代码中不硬编码；
- mock 模式 / 未配置 API Key 时有明确的降级路径，保证服务可用或报错清晰。
"""

from __future__ import annotations

import asyncio
import json
import logging
import operator
from typing import Annotated, Any, TypedDict

from app import prompts, validators
from app.config import settings
from app.errors import AgentServiceError
from app.llm import has_llm_config, invoke_structured
from app.mock import build_mock_chapter, build_mock_outline, build_mock_phase
from app.schemas import (
    Chapter,
    ClarificationOutput,
    PlanOutline,
    PlanRequest,
    ReviewOutput,
    StudyPhase,
)

logger = logging.getLogger("praxis.nodes")

# 单请求阶段扩展的并发上限：骨架一次拆出多阶段时，避免同时打开
# 过多 LLM 连接（首展 Send 多实例与定向回炉 gather 共用同一信号量）
_EXPAND_SEMAPHORE = asyncio.Semaphore(3)

# 未配置 Key 的统一引导文案（mock 判定之后、真实 LLM 调用之前抛出）
_NO_KEY_HINT = (
    "未配置 LLM API Key：请复制 agent-service/.env.example 为 .env 并填入 "
    "OPENAI_API_KEY（或 ANTHROPIC_API_KEY）；"
    "或设置 PRAXIS_ALLOW_MOCK_LLM=true 使用离线演示模式。"
)

# 首次展开阶段时注入 phase.yaml 的占位评审意见（模板据此判断「非回炉」）
_FIRST_EXPAND_REVIEW_NOTES = "（首次生成，无评审意见）"

# 程序化校验问题为空时注入 review.yaml 的占位说明
_NO_VALIDATION_ISSUES = "（无程序化校验问题）"


class AgentState(TypedDict, total=False):
    """LangGraph 状态：在节点之间流动的字段集合。

    total=False 表示所有键都可选（各节点只读/写自己关心的字段）。
    """

    request: dict[str, Any]  # PlanRequest 的 camelCase dict
    clarifications: list[str]  # clarify 产出：非空则流程终止，返回澄清问题
    outline: dict[str, Any]  # outline 产出：PlanOutline 的 camelCase dict（骨架）
    # 阶段草稿：阶段索引字符串 -> StudyPhase camelCase dict。
    # Annotated reducer（operator.or_，即 dict union）：并行 Send 多实例
    # 各写各的索引键，LangGraph 据此做增量合并而非整值覆盖
    phase_drafts: Annotated[dict[str, dict[str, Any]], operator.or_]
    plan: dict[str, Any]  # assemble 产出：StudyPlan 的 camelCase dict
    passed: bool  # review 产出：计划是否通过质量评审
    review_notes: list[str]  # review 产出：评审问题与回炉备注（回炉时作为重生成输入）
    suggestions: list[str]  # review 产出：改进建议（随响应包络返回给前端）
    revised: bool  # 是否已按评审意见修订过（保证最多回炉一次，防止死循环）
    validation_issues: list[str]  # assemble 产出：程序化校验问题描述列表
    pending_phase_indexes: list[int]  # 待定向回炉的阶段索引（0-based，升序去重）


def _request_of(state: AgentState) -> PlanRequest:
    """从状态中恢复 PlanRequest（camelCase dict -> 模型）。

    注：入口固定注入 request，运行时必然存在；用 .get() 仅为满足
    total=False TypedDict 的静态检查。
    """
    return PlanRequest.model_validate(state.get("request") or {})


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
            (
                "weeks_out_of_range",
                f"期望周数 {req.weeks} 超出合理范围（1-52）",
            )
        )
    if not req.background.strip():
        issues.append(("background_missing", "未描述已有基础，无法校准学习起点"))

    return issues


# ---------------------------------------------------------------------------
# 节点 1：clarify —— 检查输入是否完整，不完整则产出澄清问题
# ---------------------------------------------------------------------------


async def clarify_node(state: AgentState) -> dict[str, Any]:
    """澄清节点：规则检查发现信息缺口时，让 LLM 生成友好的澄清问题。

    降级策略：mock 模式、未配置 Key 或 LLM 调用失败时，直接使用
    clarify.yaml 中 fallback_questions 的内置文案——澄清是辅助能力，
    任何失败都不应阻断主流程。
    """
    req = _request_of(state)
    issues = _detect_issues(req)

    # 输入完整 → 空列表，条件边将放行到 plan 节点
    if not issues:
        return {"clarifications": []}
    logger.info("检测到 %d 个信息缺口，触发澄清", len(issues))

    fallback = prompts.fallback_questions("clarify")
    fallback_answers = [fallback.get(code, desc) for code, desc in issues]

    if settings.allow_mock_llm or not has_llm_config():
        return {"clarifications": fallback_answers}

    try:
        output = await invoke_structured(
            ClarificationOutput,
            "clarify",
            {
                "request": json.dumps(state.get("request", {}), ensure_ascii=False),
                "issues": "\n".join(f"- {desc}" for _, desc in issues),
            },
        )
        questions = [q.strip() for q in output.questions if q.strip()]
        # LLM 返回为空时仍回退到内置文案
        return {"clarifications": questions or fallback_answers}
    except Exception:  # 澄清是辅助能力：任何失败都降级，不阻断主流程
        logger.warning("澄清问题生成失败，使用内置文案", exc_info=True)
        return {"clarifications": fallback_answers}


# ---------------------------------------------------------------------------
# 节点 2：outline —— 只产出计划骨架（阶段大纲，不写章节正文）
# ---------------------------------------------------------------------------


async def outline_node(state: AgentState) -> dict[str, Any]:
    """骨架规划节点：用 outline.yaml 产出 PlanOutline（计划标题 / 阶段大纲 / 资源）。

    骨架只描述每个阶段的目标、周区间、章节标题与项目简介，
    章节正文由后续 expand_phase_node 逐阶段展开。
    """
    req = _request_of(state)

    if settings.allow_mock_llm:
        return {"outline": build_mock_outline(state.get("request", {}))}
    if not has_llm_config():
        raise AgentServiceError(_NO_KEY_HINT)

    outline = await invoke_structured(
        PlanOutline,
        "outline",
        {
            "goal": req.goal,
            "level": req.level.value,
            "weekly_hours": str(req.weekly_hours),
            "weeks": str(req.weeks),
            "interests": "、".join(req.interests) if req.interests else "未填写",
            "background": req.background or "未填写",
        },
    )
    # 以 camelCase JSON 形式存入状态（与后续节点及响应包络直接对齐）
    return {"outline": outline.model_dump(mode="json", by_alias=True)}


# ---------------------------------------------------------------------------
# 阶段扩展共用逻辑：单阶段草稿生成（首展与定向回炉两条路径共用）
# ---------------------------------------------------------------------------


def build_plan_context(outline: dict[str, Any], phase_index: int) -> str:
    """从大纲构造阶段扩展用的计划上下文（标题 / 概述 / 相邻阶段名称）。

    首展构造 Send payload 与定向回炉重建上下文共用本函数，
    保证两条路径注入给 LLM 的上下文形状一致。
    """
    phases = outline.get("phases") or []

    def _phase_name(index: int) -> str:
        if 0 <= index < len(phases):
            return (phases[index] or {}).get("name") or f"第 {index + 1} 阶段"
        return "（无）"

    return (
        f"计划标题：{outline.get('title', '')}\n"
        f"整体概述：{outline.get('overview', '')}\n"
        f"上一阶段：{_phase_name(phase_index - 1)}\n"
        f"下一阶段：{_phase_name(phase_index + 1)}"
    )


async def _expand_phase_draft(
    request: dict[str, Any],
    phase_outline: dict[str, Any],
    phase_index: int,
    *,
    plan_context: str,
    review_notes: str,
) -> dict[str, Any]:
    """扩展单个阶段草稿（mock / 真实 LLM 双路径），返回 StudyPhase camelCase dict。

    并发闸在此收口：首展（Send 多实例）与定向回炉（gather）共用
    _EXPAND_SEMAPHORE，同一请求最多 3 个阶段同时调用 LLM。
    """
    if settings.allow_mock_llm:
        return build_mock_phase(request, phase_outline, phase_index)
    if not has_llm_config():
        raise AgentServiceError(_NO_KEY_HINT)

    async with _EXPAND_SEMAPHORE:
        phase = await invoke_structured(
            StudyPhase,
            "phase",
            {
                "plan_context": plan_context,
                "phase_outline": json.dumps(phase_outline, ensure_ascii=False),
                "review_notes": review_notes,
            },
        )
    return phase.model_dump(mode="json", by_alias=True)


# ---------------------------------------------------------------------------
# 端点级重生成封装：POST /api/regenerate/stream 两条路径直接复用本节函数
# （build_plan_context / Semaphore / 解析错误反馈重试与图内扩展完全同源）
# ---------------------------------------------------------------------------


def _phase_outline_at(outline: dict[str, Any], phase_index: int) -> dict[str, Any]:
    """取指定阶段的大纲 dict（越界与空大纲由端点层先行校验，这里纯取值）。"""
    return (outline.get("phases") or [])[phase_index]


async def regenerate_phase(
    request: dict[str, Any],
    outline: dict[str, Any],
    phase_index: int,
) -> dict[str, Any]:
    """全阶段重生成（regen 端点 chapterIndex 缺省路径）：返回 StudyPhase camelCase dict。

    复用 _expand_phase_draft：同用 _EXPAND_SEMAPHORE 并发闸与解析重试；
    评审意见注入占位文案（重生成无评审误会，phase.yaml 据此跳过意见落实）。
    """
    phase_outline = _phase_outline_at(outline, phase_index)
    return await _expand_phase_draft(
        request,
        phase_outline,
        phase_index,
        plan_context=build_plan_context(outline, phase_index),
        review_notes=_FIRST_EXPAND_REVIEW_NOTES,
    )


async def regenerate_chapter(
    request: dict[str, Any],
    outline: dict[str, Any],
    phase_index: int,
    chapter_index: int,
) -> dict[str, Any]:
    """单章重生成（regen 端点 chapterIndex 指定路径）：返回 Chapter camelCase dict。

    上下文取自阶段大纲：兄弟章标题排除自身（参考而非必写，空列表合法传空）；
    结构合法性由 Pydantic Chapter 模型兜底，不跑整计划 validators。
    """
    phase_outline = _phase_outline_at(outline, phase_index)
    chapter_titles = phase_outline.get("chapterTitles") or []
    chapter_title = chapter_titles[chapter_index]
    siblings = [t for i, t in enumerate(chapter_titles) if i != chapter_index]

    if settings.allow_mock_llm:
        return build_mock_chapter(request, phase_outline, chapter_index)
    if not has_llm_config():
        raise AgentServiceError(_NO_KEY_HINT)

    req = PlanRequest.model_validate(request)
    async with _EXPAND_SEMAPHORE:
        chapter = await invoke_structured(
            Chapter,
            "chapter",
            {
                "goal": req.goal,
                "background": req.background or "未填写",
                "interests": "、".join(req.interests) if req.interests else "未填写",
                "weekly_hours": str(req.weekly_hours),
                "weeks": str(req.weeks),
                "phase_name": phase_outline.get("name", ""),
                "phase_week_range": phase_outline.get("weekRange", ""),
                "phase_objectives": (
                    "\n".join(f"- {obj}" for obj in phase_outline.get("objectives") or [])
                    or "（无）"
                ),
                "chapter_title": chapter_title,
                "sibling_chapter_titles": "\n".join(
                    f"- {t}" for t in siblings
                ),
                "project_brief": phase_outline.get("projectBrief", "") or "（无）",
            },
        )
    return chapter.model_dump(mode="json", by_alias=True)


# ---------------------------------------------------------------------------
# 节点 3：expand_phase —— Send 目标节点，并行扩展单个阶段
# ---------------------------------------------------------------------------


async def expand_phase_node(payload: dict[str, Any]) -> dict[str, Any]:
    """阶段扩展节点：把 payload 指定的单个阶段骨架扩展为完整正文。

    由图侧经 Send API 分发，收到的是 payload dict（含 request /
    phase_outline / phase_index / plan_context）而非全局状态。
    返回增量只含自己负责的索引键——多实例并行时各写各键，互不覆盖。
    """
    request: dict[str, Any] = payload["request"]
    phase_outline: dict[str, Any] = payload["phase_outline"]
    phase_index: int = payload["phase_index"]

    draft = await _expand_phase_draft(
        request,
        phase_outline,
        phase_index,
        plan_context=payload["plan_context"],
        # 首展不是回炉：注入占位意见，phase.yaml 据此跳过意见落实
        review_notes=_FIRST_EXPAND_REVIEW_NOTES,
    )
    return {"phase_drafts": {str(phase_index): draft}}


# ---------------------------------------------------------------------------
# 节点 4：expand_revision —— 定向回炉，只重生成未通过的阶段
# ---------------------------------------------------------------------------


async def expand_revision_node(state: AgentState) -> dict[str, Any]:
    """定向回炉节点：仅重生成 pending_phase_indexes 中的阶段。

    与整计划回炉不同：gather 收集异常而不传播，单个阶段失败不拖垮
    整轮——失败阶段保留旧草稿，并把原因追加进 review_notes。
    """
    req = _request_of(state)
    request = req.model_dump(by_alias=True)
    outline = state.get("outline") or {}
    drafts: dict[str, dict[str, Any]] = dict(state.get("phase_drafts") or {})
    pending: list[int] = state.get("pending_phase_indexes") or []
    notes: list[str] = list(state.get("review_notes") or [])
    # 注入给重生成调用的评审意见（失败备注在 gather 之后才追加，互不污染）
    review_notes_text = "\n".join(notes) or _FIRST_EXPAND_REVIEW_NOTES

    async def _redo(index: int) -> tuple[int, dict[str, Any]]:
        phase_outline = (outline.get("phases") or [])[index]
        draft = await _expand_phase_draft(
            request,
            phase_outline,
            index,
            plan_context=build_plan_context(outline, index),
            review_notes=review_notes_text,
        )
        return index, draft

    results = await asyncio.gather(
        *(_redo(index) for index in pending), return_exceptions=True
    )

    for index, result in zip(pending, results, strict=True):  # gather 保序且一一对应
        if isinstance(result, BaseException):
            notes.append(f"第 {index + 1} 阶段重生成失败，保留原稿：{result}")
            continue
        _, draft = result
        drafts[str(index)] = draft

    return {"phase_drafts": drafts, "revised": True, "review_notes": notes}


# ---------------------------------------------------------------------------
# 节点 5：assemble —— 拼装完整计划 + 程序化校验
# ---------------------------------------------------------------------------


async def assemble_node(state: AgentState) -> dict[str, Any]:
    """拼装节点：阶段草稿按索引升序拼成 StudyPlan，并跑程序化校验。

    本节点不调用 LLM；校验不通过的阶段索引写入 pending_phase_indexes，
    与评审失败索引一起构成定向回炉的唯一输入。
    """
    req = _request_of(state)
    outline = state.get("outline") or {}
    drafts: dict[str, dict[str, Any]] = state.get("phase_drafts") or {}

    phases = [drafts[key] for key in sorted(drafts, key=int)]
    plan = {
        "title": outline.get("title", ""),
        "overview": outline.get("overview", ""),
        "estimatedTotalHours": outline.get("estimatedTotalHours", 0),
        "phases": phases,
        "resources": outline.get("resources") or [],
        "reviewNotes": "\n".join(state.get("review_notes") or []),
    }

    issues, failed_indexes = validators.validate_plan(plan, req.weekly_hours, req.weeks)
    logger.info(
        "拼装完成 phases=%d 校验问题=%d 待回炉阶段=%s",
        len(phases),
        len(issues),
        failed_indexes,
    )
    return {
        "plan": plan,
        "validation_issues": issues,
        "pending_phase_indexes": failed_indexes,
    }


# ---------------------------------------------------------------------------
# 节点 6：review —— LLM 评审 + 汇总定向回炉索引
# ---------------------------------------------------------------------------


async def review_node(state: AgentState) -> dict[str, Any]:
    """评审节点：结合程序化校验结果，从结构完整 / 主要知识 / 可测性 /
    时间可行 / 难度递进 / 方向契合等维度审查。

    LLM 判定失败的阶段索引与校验失败索引取并集后写入
    pending_phase_indexes（定向回炉的唯一入口）。
    mock / 未配置 Key：跳过评审直接放行，保证全链路可用。
    """
    plan = state.get("plan") or {}
    validation_issues: list[str] = state.get("validation_issues") or []

    if settings.allow_mock_llm:
        return {"passed": True, "review_notes": ["（Mock 模式：未执行真实 LLM 评审）"]}
    if not has_llm_config():
        return {"passed": True, "review_notes": ["（未配置 API Key：跳过评审）"]}

    output = await invoke_structured(
        ReviewOutput,
        "review",
        {
            "plan": json.dumps(plan, ensure_ascii=False),
            "validation_issues": (
                "\n".join(f"- {issue}" for issue in validation_issues)
                if validation_issues
                else _NO_VALIDATION_ISSUES
            ),
        },
    )

    logger.info(
        "评审完成 passed=%s issues=%d suggestions=%d failed_phases=%s",
        output.passed,
        len(output.issues),
        len(output.suggestions),
        output.failed_phase_indexes,
    )
    merged_pending = sorted(
        set(state.get("pending_phase_indexes") or []) | set(output.failed_phase_indexes)
    )
    return {
        "passed": output.passed,
        "review_notes": [issue for issue in output.issues if issue.strip()],
        "suggestions": list(output.suggestions),
        "pending_phase_indexes": merged_pending,
    }
