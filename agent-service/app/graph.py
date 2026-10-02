"""LangGraph 状态图定义：clarify → plan → review。

流程：
- clarify 发现信息缺口 → END（返回澄清问题给用户）
- clarify 通过 → plan（LLM 生成计划）→ review（LLM 评审）
- review 未通过且尚未修订 → 回到 plan（带着评审意见重新规划）
- review 通过或已修订过一次 → END（返回计划，评审备注随计划返回）
"""

from __future__ import annotations

from typing import Any

from langgraph.graph import END, StateGraph

from app.nodes import AgentState, clarify_node, plan_node, review_node


def _route_after_clarify(state: dict[str, Any]) -> str:
    """clarify 之后的条件边：有澄清问题 → 结束；无 → 进入规划。"""
    return "end" if state.get("clarifications") else "plan"


def _route_after_review(state: dict[str, Any]) -> str:
    """review 之后的条件边：通过 / 已修订过 → 结束；否则带着意见回炉重规划。"""
    if state.get("passed") or state.get("revised"):
        return "end"
    return "plan"


def build_graph():
    """构建并编译 LangGraph 应用。"""
    graph = StateGraph(AgentState)

    graph.add_node("clarify", clarify_node)
    graph.add_node("plan", plan_node)
    graph.add_node("review", review_node)

    # 入口：clarify
    graph.set_entry_point("clarify")
    graph.add_conditional_edges(
        "clarify",
        _route_after_clarify,
        {"end": END, "plan": "plan"},
    )

    # plan 之后固定进入 review
    graph.add_edge("plan", "review")

    # review 之后：通过/已修订 → END；否则回 plan（最多回炉一次）
    graph.add_conditional_edges(
        "review",
        _route_after_review,
        {"end": END, "plan": "plan"},
    )

    return graph.compile()


# 模块级编译好的图，供 FastAPI 直接使用
agent_graph = build_graph()
