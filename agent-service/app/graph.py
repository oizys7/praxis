"""LangGraph 状态图定义：章级扇出拓扑（骨架 → 章级并行扩展 → 拼装 → 评审/章级回炉）。

流程：
- intake 分流：state 带 regen_scope（重生成）→ 直接按作用域扇出，跳过
  clarify/outline；否则 → clarify
- clarify 发现信息缺口 → END（返回澄清问题给用户）
- clarify 通过 → outline（骨架规划）→ 条件边 Send 双路分发：
  - expand_chapter：每章一个实例（每章 1 次 LLM 调用，一次只写一章正文）
  - expand_phase_wrap：每阶段一个实例（轻量调用，只产 practice/assignments/project）
- 两路实例各写各键 → assemble（按 (阶段索引, 章索引) 重组 + 程序化校验）
- assemble 未修订 → review（LLM 评审）：
  - 有待回炉阶段 → Send expand_revision 章级定向回炉（只重扩失败章；
    结构性问题回炉该阶段全部章）→ 回到 assemble
  - 无待回炉阶段 → END
- assemble 已修订过 → END（图侧条件边保证最多回炉一次，防死循环）

章级扇出的动机：一次吐整阶段（2-6 章长 markdown）是 LLM 解析失败/截断
重灾区，按章单调用保证质量；outlines 已产出每章标题，天然支持章级 Send 扇出。
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any

from langgraph.graph import END, StateGraph
from langgraph.types import Send

from app.nodes import (
    AgentState,
    assemble_node,
    build_plan_context,
    clarify_node,
    expand_chapter_node,
    expand_phase_wrap_node,
    expand_revision_node,
    outline_node,
    review_node,
)

# 评审节点签名（真实 review_node 与测试注入的 stub 共用）
ReviewNode = Callable[[AgentState], Awaitable[dict[str, Any]]]


def _route_after_clarify(state: dict[str, Any]) -> str:
    """clarify 之后的条件边：有澄清问题 → 结束；无 → 骨架规划。"""
    return "end" if state.get("clarifications") else "outline"


def _fan_out_chapters(state: dict[str, Any]) -> list[Send]:
    """扇出条件边：按骨架章级分发 Send（并行扩展）。

    双路分发：每章一个 expand_chapter 实例（payload 携带 request /
    phase_outline / phase_index / chapter_index / plan_context），
    加每阶段一个 expand_phase_wrap 实例（轻量收尾内容，不含章节正文）。
    plan_context 由 build_plan_context 从大纲构造（相邻阶段名 / 标题 / 概述）。

    重生成（state.regen_scope 存在）时按作用域过滤：只发目标阶段的 Send，
    单章重生成只发那一个 expand_chapter、且不发 wrap（不动阶段轻装）。
    """
    outline = state.get("outline") or {}
    phases = outline.get("phases") or []
    request = state.get("request")
    scope = state.get("regen_scope") or {}
    scope_phase = scope.get("phase")
    scope_chapters_raw = scope.get("chapters")
    scope_chapters = (
        [int(c) for c in scope_chapters_raw] if scope_chapters_raw is not None else None
    )
    phase_indexes: list[int] = (
        [int(scope_phase)] if scope_phase is not None else list(range(len(phases)))
    )

    sends: list[Send] = []
    for index in phase_indexes:
        if not 0 <= index < len(phases):
            continue
        phase = phases[index]
        plan_context = build_plan_context(outline, index)
        titles = phase.get("chapterTitles") or []
        chapter_indexes = (
            [c for c in scope_chapters if 0 <= c < len(titles)]
            if scope_chapters is not None
            else list(range(len(titles)))
        )
        for chapter_index in chapter_indexes:
            sends.append(
                Send(
                    "expand_chapter",
                    {
                        "request": request,
                        "phase_outline": phase,
                        "phase_index": index,
                        "chapter_index": chapter_index,
                        "plan_context": plan_context,
                        # 重生成链需要「本章开始生成」信号做逐章动画；
                        # 首次生成不需要（Rust 侧对 chapterStart 会忽略）
                        "emit_chapter_start": scope_phase is not None,
                    },
                )
            )
        # 阶段轻装（practice / assignments / project）与章节正文并行生成。
        # 单章重生成不发：只修那一章，不动阶段收尾内容。
        if scope_chapters is None:
            sends.append(
                Send(
                    "expand_phase_wrap",
                    {
                        "request": request,
                        "phase_outline": phase,
                        "phase_index": index,
                    },
                )
            )
    return sends


async def _intake_node(state: dict[str, Any]) -> dict[str, Any]:
    """入口占位节点：只做路由，不产生任何状态。

    存在的唯一理由是让「重生成」能从入口就扇出——LangGraph 的条件边必须
    挂在一个节点上，而重生成不需要 clarify/outline 这两步。
    """
    return {}


def _route_after_intake(state: dict[str, Any]) -> list[Send] | str:
    """入口条件边：重生成直接扇出到目标范围；首次生成先走 clarify。"""
    if state.get("regen_scope"):
        return _fan_out_chapters(state)
    return "clarify"


def _route_after_assemble(state: dict[str, Any]) -> str:
    """assemble 之后的条件边：已修订过（回炉后重拼）→ 结束；否则进入评审。"""
    return "end" if state.get("revised") else "review"


def _route_after_review(state: dict[str, Any]) -> list[Send] | str:
    """review 之后的条件边：有待回炉阶段 → Send 章级定向回炉；否则结束。

    Send 携带全量 state 快照：expand_revision_node 需要
    pending_phase_indexes / failed_chapter_keys / chapter_drafts / phase_extras /
    review_notes / request / outline。
    """
    if not state.get("pending_phase_indexes"):
        return END
    return [Send("expand_revision", dict(state))]


def build_graph(review: ReviewNode | None = None):
    """构建并编译 LangGraph 应用。

    review 参数可注入评审节点替身（测试用验证回炉路由），缺省用真实 review_node。
    """
    graph = StateGraph(AgentState)

    graph.add_node("intake", _intake_node)  # type: ignore[arg-type]
    graph.add_node("clarify", clarify_node)
    graph.add_node("outline", outline_node)
    # Send 目标节点收 payload dict 而非全局状态，LangGraph 的节点类型签名
    # 无法表达该约定（官方支持模式），此处静态检查报参不匹配属误报
    graph.add_node("expand_chapter", expand_chapter_node)  # type: ignore[arg-type]
    graph.add_node("expand_phase_wrap", expand_phase_wrap_node)  # type: ignore[arg-type]
    graph.add_node("assemble", assemble_node)
    # 注入替身与真实节点联合类型同样无法满足严格的节点签名检查
    graph.add_node("review", review or review_node)  # type: ignore[arg-type]
    graph.add_node("expand_revision", expand_revision_node)

    # 入口：intake 分流——重生成直接扇出，首次生成先 clarify
    graph.set_entry_point("intake")
    graph.add_conditional_edges("intake", _route_after_intake)
    graph.add_conditional_edges(
        "clarify",
        _route_after_clarify,
        {"end": END, "outline": "outline"},
    )

    # outline 之后 Send 双路并行分发到 expand_chapter + expand_phase_wrap
    graph.add_conditional_edges("outline", _fan_out_chapters)

    # 并行扩展的各实例完成后统一进入拼装
    graph.add_edge("expand_chapter", "assemble")
    graph.add_edge("expand_phase_wrap", "assemble")

    # assemble：未修订 → review；已修订过（回炉后重拼）→ END
    graph.add_conditional_edges(
        "assemble",
        _route_after_assemble,
        {"review": "review", "end": END},
    )

    # review：有待回炉阶段 → Send expand_revision；无 → END
    graph.add_conditional_edges("review", _route_after_review)

    # 章级定向回炉完成后回到 assemble（此时 revised=True，走 END 分支）
    graph.add_edge("expand_revision", "assemble")

    return graph.compile()


# 模块级编译好的图，供 FastAPI 直接使用
agent_graph = build_graph()
