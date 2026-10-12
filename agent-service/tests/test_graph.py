"""graph.py 分层装配全链路测试（mock 模式，不触网）。

用例名与实现计划简报逐字对应（test_full_flow_mock_returns_plan /
test_review_rework_route / test_rework_terminates）。
review stub 经 build_graph(review=...) 注入，验证定向回炉拓扑：
expand_phase 后给首展草稿打哨兵标记——回炉重生成（确定性 mock）不带哨兵，
最终草稿是否含哨兵即可区分「保留首展稿」与「被重生成数据覆盖」。
"""

from typing import Any

import pytest

import app.graph as graph_module
from app.config import settings
from app.graph import agent_graph, build_graph
from app.nodes import AgentState

# camelCase 请求，形状等同 PlanRequest.model_dump(by_alias=True)
REQUEST = {
    "goal": "全栈 Web 开发",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["前端", "Python"],
    "background": "写过一些脚本",
}

# 首展哨兵：打在阶段名前缀上，重生成后的确定性 mock 数据不含此前缀
_SENTINEL = "[首展哨兵]"


def _patch_sentinel_expand(monkeypatch: pytest.MonkeyPatch) -> None:
    """包装 expand_phase_node：给阶段 0 与 1 的首展草稿名打上哨兵前缀。

    patch 的是 graph 模块命名空间——build_graph 在调用时才取
    add_node 所引用的函数对象，故在 build_graph 之前 patch 即生效。
    """
    original_expand = graph_module.expand_phase_node

    async def _expand_with_sentinel(payload: dict[str, Any]) -> dict[str, Any]:
        result = await original_expand(payload)
        index = payload["phase_index"]
        if index in (0, 1):
            draft = dict(result["phase_drafts"][str(index)])
            draft["name"] = f"{_SENTINEL}{draft['name']}"
            result = {"phase_drafts": {str(index): draft}}
        return result

    monkeypatch.setattr(graph_module, "expand_phase_node", _expand_with_sentinel)


# ---------------------------------------------------------------------------
# 全链路：mock 模式走完整分层生成拓扑
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_full_flow_mock_returns_plan(monkeypatch):
    """mock 模式全链路：clarify → outline → 并行扩展 → 拼装 → 评审放行 → END。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    result = await agent_graph.ainvoke({"request": REQUEST})

    plan = result["plan"]
    assert plan["phases"], "计划至少包含一个阶段"
    assert plan["phases"][0]["chapters"], "首个阶段至少包含一章"
    assert result["passed"] is True, "mock 评审应直接放行"


# ---------------------------------------------------------------------------
# 定向回炉：评审失败阶段被重生成数据覆盖，未点名阶段保留首展稿
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_review_rework_route(monkeypatch):
    """评审只点名阶段 1：回炉后阶段 1 被重生成覆盖，阶段 0 保留首展稿。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    _patch_sentinel_expand(monkeypatch)

    async def _failing_review(state: AgentState) -> dict[str, Any]:
        # 回炉前的状态自检：哨兵必须仍在（证明首展包装已生效）
        name = (state.get("phase_drafts") or {}).get("1", {}).get("name", "")
        assert name.startswith(_SENTINEL)
        return {
            "passed": False,
            "review_notes": ["第 2 阶段深度不足，需要重写"],
            "suggestions": ["增加对照实验与自测清单"],
            "pending_phase_indexes": [1],
        }

    graph = build_graph(review=_failing_review)

    result = await graph.ainvoke({"request": REQUEST})

    assert result["revised"] is True, "定向回炉后应标记已修订"
    assert not result["phase_drafts"]["1"]["name"].startswith(_SENTINEL), (
        "阶段 1 旧稿应被重生成数据覆盖"
    )
    assert result["phase_drafts"]["0"]["name"].startswith(_SENTINEL), (
        "阶段 0 未被点名，应保留首展稿（定向回炉而非全量重做）"
    )
    assert not result["plan"]["phases"][1]["name"].startswith(_SENTINEL), (
        "回炉后拼装应采用阶段 1 的新稿"
    )


# ---------------------------------------------------------------------------
# 防死循环：评审永远不通过也只回炉一次，流程正常终止
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_rework_terminates(monkeypatch):
    """stub 永远 passed=False：回炉一次后终止，不死循环。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    calls = {"review": 0}

    async def _always_failing_review(state: AgentState) -> dict[str, Any]:
        calls["review"] += 1
        return {
            "passed": False,
            "review_notes": ["始终不通过"],
            "suggestions": [],
            "pending_phase_indexes": [0, 1, 2],
        }

    graph = build_graph(review=_always_failing_review)

    result = await graph.ainvoke({"request": REQUEST})

    assert result["revised"] is True
    assert calls["review"] == 1, "回炉后不再进评审（图侧保证最多回炉一次）"
