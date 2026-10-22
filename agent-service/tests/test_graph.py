"""graph.py 章级扇出装配全链路测试（mock 模式，不触网）。

用例名与实现计划简报逐字对应（test_full_flow_mock_returns_plan /
test_chapter_rework_only_regenerates_failed_chapter /
test_structural_rework_redoes_all_chapters_and_extras / test_rework_terminates）。
review stub 经 build_graph(review=...) 注入，验证章级定向回炉拓扑：

- 首展哨兵：包装 expand_chapter_node / expand_phase_wrap_node——给首展章
  summary 打哨兵前缀，同时破坏「1:1」章正文触发章级失败；
- 回炉哨兵：包装 nodes._expand_chapter_draft / nodes._expand_phase_extras
  （带计数）——回炉轮直接调用这两个函数，是否含哨兵/计数即可区分
  「保留首展稿」与「被回炉重扩覆盖」，并验证「只重扩失败章」。
"""

from typing import Any

import app.graph as graph_module
import app.nodes as nodes_module
import pytest
from app.config import settings
from app.graph import agent_graph, build_graph
from app.nodes import AgentState
from app.schemas import Chapter

# camelCase 请求，形状等同 PlanRequest.model_dump(by_alias=True)
REQUEST = {
    "goal": "全栈 Web 开发",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["前端", "Python"],
    "background": "写过一些脚本",
}

# 首展哨兵：打在章 summary 前缀上，回炉重扩（确定性 mock）产物无此前缀
_SENTINEL = "[首展哨兵]"
# 回炉哨兵：回炉重扩产物的前缀（与首展哨兵互斥可辨）
_REWORK_SENTINEL = "[回炉哨兵]"


def _patch_sentinel_expand(
    monkeypatch: pytest.MonkeyPatch, *, break_key: str | None = "1:1"
) -> None:
    """包装 expand_chapter_node：给首展章 summary 打首展哨兵前缀。

    break_key 非空时把该章正文改成「太短」——assemble 会把它标进
    failed_chapter_keys，触发章级定向回炉（缺省破坏 1:1）。
    同时给 expand_phase_wrap_node 的 assignments 打首展哨兵标记。
    """
    original_chapter = graph_module.expand_chapter_node
    original_wrap = graph_module.expand_phase_wrap_node

    async def _chapter_with_sentinel(payload: dict[str, Any]) -> dict[str, Any]:
        result = await original_chapter(payload)
        stamped: dict[str, Any] = {}
        for key, chapter in result["chapter_drafts"].items():
            chapter = dict(chapter)
            chapter["summary"] = f"{_SENTINEL}{chapter.get('summary', '')}"
            if break_key is not None and key == break_key:
                chapter["contentMd"] = "太短"  # 破坏：触发章级失败
            stamped[key] = chapter
        return {"chapter_drafts": stamped}

    async def _wrap_with_sentinel(payload: dict[str, Any]) -> dict[str, Any]:
        result = await original_wrap(payload)
        stamped: dict[str, Any] = {}
        for key, extras in result["phase_extras"].items():
            extras = dict(extras)
            extras["assignments"] = [f"{_SENTINEL}{a}" for a in extras["assignments"]]
            stamped[key] = extras
        return {"phase_extras": stamped}

    # build_graph 调用时才取 add_node 引用的函数对象，先 patch 即生效
    monkeypatch.setattr(graph_module, "expand_chapter_node", _chapter_with_sentinel)
    monkeypatch.setattr(graph_module, "expand_phase_wrap_node", _wrap_with_sentinel)


def _patch_rework_expand(monkeypatch: pytest.MonkeyPatch) -> dict[str, int]:
    """包装 nodes._expand_chapter_draft / _expand_phase_extras（回炉轮识别）。

    首展与回炉轮共用同一底层函数（本函数拦截的是模块全局查找），
    以 review_notes 是否为占位文案区分：首展恒用占位文案，回炉轮
    注入真实评审意见。仅对回炉轮计数并打回炉哨兵前缀。
    """
    calls = {"chapter": 0, "extras": 0}
    original_chapter = nodes_module._expand_chapter_draft
    original_extras = nodes_module._expand_phase_extras

    def _is_rework(kwargs: dict[str, Any]) -> bool:
        return (
            kwargs.get("review_notes") is not None
            and kwargs["review_notes"] != nodes_module._FIRST_EXPAND_REVIEW_NOTES
        )

    async def _chapter_with_rework_sentinel(
        *args: Any, **kwargs: Any
    ) -> tuple[dict[str, Any], dict[str, Any]]:
        chapter, trace = await original_chapter(*args, **kwargs)
        if not _is_rework(kwargs):
            return chapter, trace
        calls["chapter"] += 1
        return (
            {**chapter, "summary": f"{_REWORK_SENTINEL}{chapter.get('summary', '')}"},
            trace,
        )

    async def _extras_with_rework_sentinel(*args: Any, **kwargs: Any) -> dict[str, Any]:
        extras = await original_extras(*args, **kwargs)
        if not _is_rework(kwargs):
            return extras
        calls["extras"] += 1
        return {
            **extras,
            "assignments": [f"{_REWORK_SENTINEL}{a}" for a in extras["assignments"]],
        }

    monkeypatch.setattr(
        nodes_module, "_expand_chapter_draft", _chapter_with_rework_sentinel
    )
    monkeypatch.setattr(
        nodes_module, "_expand_phase_extras", _extras_with_rework_sentinel
    )
    return calls


# ---------------------------------------------------------------------------
# 全链路：mock 模式走完整章级扇出拓扑
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_full_flow_mock_returns_plan(monkeypatch):
    """mock 模式全链路：clarify → outline → 章级并行扩展+warp → 拼装 → 评审放行 → END。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    result = await agent_graph.ainvoke({"request": REQUEST})

    plan = result["plan"]
    assert plan["phases"], "计划至少包含一个阶段"
    assert plan["phases"][0]["chapters"], "首个阶段至少包含一章"
    assert plan["phases"][0]["project"], "首阶段的轻装项目合入重组"
    assert result["passed"] is True, "mock 评审应直接放行"
    # 章级扇出完整：每章草稿复合键齐备（"阶段:章"）
    chapter_keys = set(result["chapter_drafts"])
    assert chapter_keys == {
        f"{p}:{c}"
        for p, phase in enumerate(plan["phases"])
        for c in range(len(phase["chapters"]))
    }


# ---------------------------------------------------------------------------
# 章级定向回炉：仅失败章被重扩，未点名章保留首展稿
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_chapter_rework_only_regenerates_failed_chapter(monkeypatch):
    """assemble 标定失败章「1:1」：回炉后仅该章被重扩，其余章保留首展稿。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    _patch_sentinel_expand(monkeypatch)
    calls = _patch_rework_expand(monkeypatch)

    async def _failing_review(state: AgentState) -> dict[str, Any]:
        # 回炉前的状态自检：失败章与哨兵必须已就位
        assert state.get("failed_chapter_keys") == ["1:1"]
        drafts = state.get("chapter_drafts") or {}
        assert drafts["1:1"]["contentMd"] == "太短"
        assert drafts["1:0"]["summary"].startswith(_SENTINEL)
        return {
            "passed": False,
            "review_notes": ["第 2 阶段第 2 章正文不足，需要重写"],
            "suggestions": [],
            "pending_phase_indexes": [1],
        }

    graph = build_graph(review=_failing_review)

    result = await graph.ainvoke({"request": REQUEST})

    assert result["revised"] is True, "章级定向回炉后应标记已修订"
    assert calls["chapter"] == 1, "只重扩失败章（恰 1 次）"
    assert calls["extras"] == 0, "章级失败路径不重做轻装"
    # 失败章被重扩覆盖：回炉哨兵出现、短正文被替换
    assert result["chapter_drafts"]["1:1"]["summary"].startswith(_REWORK_SENTINEL)
    assert result["chapter_drafts"]["1:1"]["contentMd"] != "太短"
    # 未失败章保留首展稿（首展哨兵仍在）
    assert result["chapter_drafts"]["1:0"]["summary"].startswith(_SENTINEL)
    assert result["chapter_drafts"]["0:0"]["summary"].startswith(_SENTINEL)
    assert result["phase_extras"], "首展轻装全保留（未标记缺失）"
    assert not any(
        a.startswith(_REWORK_SENTINEL)
        for extras in result["phase_extras"].values()
        for a in extras["assignments"]
    ), "轻装未被回炉重做"
    # 重组采用重扩后的新稿
    assert result["plan"]["phases"][1]["chapters"][1]["contentMd"], (
        "回炉后拼装应采用失败章的新稿"
    )


# ---------------------------------------------------------------------------
# 结构性回炉：无失败章 → 该阶段全部章 + 轻装连带重做
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_structural_rework_redoes_all_chapters_and_extras(monkeypatch):
    """评审点名阶段 0（章完好）：回炉重扩全部章并连带重做轻装。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    _patch_sentinel_expand(monkeypatch, break_key=None)  # 无失败章
    calls = _patch_rework_expand(monkeypatch)

    async def _failing_review(state: AgentState) -> dict[str, Any]:
        # 自检：该阶段章完好（failed_chapter_keys 为空）→ 结构性回炉
        assert state.get("failed_chapter_keys") == []
        extras = state.get("phase_extras") or {}
        assert extras["0"]["assignments"][0].startswith(_SENTINEL)
        return {
            "passed": False,
            "review_notes": ["第 1 阶段项目验收标准不可测"],
            "suggestions": [],
            "pending_phase_indexes": [0],
        }

    graph = build_graph(review=_failing_review)

    result = await graph.ainvoke({"request": REQUEST})

    assert result["revised"] is True
    assert calls["chapter"] == 2, "该阶段全部章重扩"
    assert calls["extras"] == 1, "结构回炉连带轻装重做（恰一次）"
    # 阶段 0 全部章 + 轻装被回炉覆盖；阶段 1 保留首展稿
    assert result["chapter_drafts"]["0:0"]["summary"].startswith(_REWORK_SENTINEL)
    assert result["chapter_drafts"]["0:1"]["summary"].startswith(_REWORK_SENTINEL)
    assert result["chapter_drafts"]["1:0"]["summary"].startswith(_SENTINEL)
    assert result["phase_extras"]["0"]["assignments"][0].startswith(_REWORK_SENTINEL)
    assert all(
        result["plan"]["phases"][0]["chapters"][i]["summary"].startswith(
            _REWORK_SENTINEL
        )
        for i in range(2)
    ), "重组采用回炉新稿"
    assert result["plan"]["phases"][1]["chapters"][0]["summary"].startswith(
        _SENTINEL
    ), "未点名阶段保留首展稿"


# ---------------------------------------------------------------------------
# 防死循环：评审永远不通过也只回炉一次，流程正常终止
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_rework_terminates(monkeypatch):
    """stub 永远 passed=False：回炉一次后终止，不死循环。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    _patch_sentinel_expand(monkeypatch)
    calls = _patch_rework_expand(monkeypatch)

    calls_review = {"review": 0}

    async def _always_failing_review(state: AgentState) -> dict[str, Any]:
        calls_review["review"] += 1
        return {
            "passed": False,
            "review_notes": ["始终不通过"],
            "suggestions": [],
            "pending_phase_indexes": [0, 1, 2],
        }

    graph = build_graph(review=_always_failing_review)

    result = await graph.ainvoke({"request": REQUEST})

    assert result["revised"] is True
    assert calls_review["review"] == 1, "回炉后不再进评审（图侧保证最多回炉一次）"
    # 回炉轮：阶段 0/2 结构性（各 2 章 + 1 轻），阶段 1 只重扩失败章 1:1
    assert calls["chapter"] == 5
    assert calls["extras"] == 2


# ---------------------------------------------------------------------------
# 章级扇出 payload 形状自检（哨兵包装的入参完整性）
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_fan_out_payload_shape(monkeypatch):
    """首展 Send payload 携带章索引与计划上下文（章级扇出契约）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    seen: list[dict[str, Any]] = []

    async def _capturing_chapter(payload: dict[str, Any]) -> dict[str, Any]:
        seen.append(dict(payload))
        chapter = Chapter(
            title=payload["phase_outline"]["chapterTitles"][payload["chapter_index"]]
        )
        return {
            "chapter_drafts": {
                f"{payload['phase_index']}:{payload['chapter_index']}": chapter.model_dump(
                    mode="json", by_alias=True
                )
            }
        }

    monkeypatch.setattr(graph_module, "expand_chapter_node", _capturing_chapter)

    # agent_graph 是导入期预编译图（不随 patch 变化），需经 build_graph 重编译
    graph = build_graph()
    await graph.ainvoke({"request": REQUEST})

    assert seen, "章实例应被分发"
    stage_titles = set()
    for payload in seen:
        assert {
            "request",
            "phase_outline",
            "phase_index",
            "chapter_index",
            "plan_context",
        } <= set(payload)
        assert payload["plan_context"], "计划上下文非空"
        stage_titles.add((payload["phase_index"], payload["chapter_index"]))
    assert len(stage_titles) == len(seen), "各章 payload 唯一"
