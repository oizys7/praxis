"""LangGraph 状态图定义：分层生成拓扑（骨架 → 并行扩展 → 拼装 → 评审/定向回炉）。

流程：
- clarify 发现信息缺口 → END（返回澄清问题给用户）
- clarify 通过 → outline（骨架规划）→ 条件边 Send 并行分发 expand_phase
  （每阶段一个实例，payload 携带 request / phase_outline / phase_index / plan_context）
- expand_phase 各实例只写自己的索引键 → assemble（按索引拼装 + 程序化校验）
- assemble 未修订 → review（LLM 评审）：
  - 有待回炉阶段 → Send expand_revision 定向重生成 → 回到 assemble
  - 无待回炉阶段 → END
- assemble 已修订过 → END（图侧条件边保证最多回炉一次，防死循环）
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
    expand_phase_node,
    expand_revision_node,
    outline_node,
    review_node,
)

# 评审节点签名（真实 review_node 与测试注入的 stub 共用）
ReviewNode = Callable[[AgentState], Awaitable[dict[str, Any]]]


def _route_after_clarify(state: dict[str, Any]) -> str:
    """clarify 之后的条件边：有澄清问题 → 结束；无 → 骨架规划。"""
    return "end" if state.get("clarifications") else "outline"


def _fan_out_phases(state: dict[str, Any]) -> list[Send]:
    """outline 之后的条件边：按骨架逐阶段分发 Send（并行扩展）。

    payload 与 expand_phase_node 的入参约定一一对应；
    plan_context 由 build_plan_context 从大纲构造（相邻阶段名 / 标题 / 概述）。
    """
    outline = state.get("outline") or {}
    return [
        Send(
            "expand_phase",
            {
                "request": state.get("request"),
                "phase_outline": phase,
                "phase_index": index,
                "plan_context": build_plan_context(outline, index),
            },
        )
        for index, phase in enumerate(outline.get("phases") or [])
    ]


def _route_after_assemble(state: dict[str, Any]) -> str:
    """assemble 之后的条件边：已修订过（回炉后重拼）→ 结束；否则进入评审。"""
    return "end" if state.get("revised") else "review"


def _route_after_review(state: dict[str, Any]) -> list[Send] | str:
    """review 之后的条件边：有待回炉阶段 → Send 定向回炉；否则结束。

    Send 携带全量 state 快照：expand_revision_node 需要
    pending_phase_indexes / phase_drafts / review_notes / request / outline。
    """
    if not state.get("pending_phase_indexes"):
        return END
    return [Send("expand_revision", dict(state))]


def build_graph(review: ReviewNode | None = None):
    """构建并编译 LangGraph 应用。

    review 参数可注入评审节点替身（测试用验证回炉路由），缺省用真实 review_node。
    """
    graph = StateGraph(AgentState)

    graph.add_node("clarify", clarify_node)
    graph.add_node("outline", outline_node)
    # Send 目标节点收 payload dict 而非全局状态，LangGraph 的节点类型签名
    # 无法表达该约定（官方支持模式），此处静态检查报参不匹配属误报
    graph.add_node("expand_phase", expand_phase_node)  # type: ignore[arg-type]
    graph.add_node("assemble", assemble_node)
    # 注入替身与真实节点联合类型同样无法满足严格的节点签名检查
    graph.add_node("review", review or review_node)  # type: ignore[arg-type]
    graph.add_node("expand_revision", expand_revision_node)

    # 入口：clarify
    graph.set_entry_point("clarify")
    graph.add_conditional_edges(
        "clarify",
        _route_after_clarify,
        {"end": END, "outline": "outline"},
    )

    # outline 之后 Send 并行分发到 expand_phase
    graph.add_conditional_edges("outline", _fan_out_phases)

    # 并行扩展的各实例完成后统一进入拼装
    graph.add_edge("expand_phase", "assemble")

    # assemble：未修订 → review；已修订过（回炉后重拼）→ END
    graph.add_conditional_edges(
        "assemble",
        _route_after_assemble,
        {"review": "review", "end": END},
    )

    # review：有待回炉阶段 → Send expand_revision；无 → END
    graph.add_conditional_edges("review", _route_after_review)

    # 定向回炉完成后回到 assemble（此时 revised=True，走 END 分支）
    graph.add_edge("expand_revision", "assemble")

    return graph.compile()


# 模块级编译好的图，供 FastAPI 直接使用
agent_graph = build_graph()
