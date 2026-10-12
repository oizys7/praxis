"""nodes.py 分层生成节点测试：mock 路径全覆盖 + 关键失败路径。

所有用例均不触网：mock 路径 monkeypatch settings.allow_mock_llm=True；
真实路径 monkeypatch invoke_structured / has_llm_config 控制行为。
核心用例名与实现计划简报逐字对应（test_outline_node_mock /
test_expand_phase_parallel_safe / test_expand_revision_keeps_old_draft_on_failure /
test_review_writes_pending）。
"""

from typing import Any

import pytest

from app import nodes
from app.config import settings
from app.errors import AgentServiceError
from app.mock import build_mock_outline, build_mock_phase
from app.schemas import ReviewOutput, StudyPhase

# camelCase 请求，形状等同 PlanRequest.model_dump(by_alias=True)
REQUEST = {
    "goal": "全栈 Web 开发",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["前端", "Python"],
    "background": "写过一些脚本",
}


def _state(**overrides: Any) -> dict[str, Any]:
    """构造节点测试用的最小 AgentState（request 默认注入，同图入口约定）。"""
    state: dict[str, Any] = {"request": REQUEST}
    state.update(overrides)
    return state


# ---------------------------------------------------------------------------
# outline_node：骨架规划
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_outline_node_mock(monkeypatch):
    """mock 模式：outline_node 返回大纲骨架，阶段数 ≥2。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    result = await nodes.outline_node(_state())

    assert set(result) == {"outline"}
    outline = result["outline"]
    assert outline["title"].strip(), "大纲标题非空"
    assert len(outline["phases"]) >= 2, "至少 2 个阶段"
    for phase in outline["phases"]:
        assert {"name", "weekRange", "objectives", "chapterTitles", "projectBrief"} <= (
            set(phase)
        )


@pytest.mark.asyncio
async def test_outline_node_without_key_raises(monkeypatch):
    """非 mock 且未配置 Key：抛业务错误（引导文案），而非裸异常。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: False)

    with pytest.raises(AgentServiceError):
        await nodes.outline_node(_state())


# ---------------------------------------------------------------------------
# expand_phase_node：并行阶段扩展（Send 目标节点）
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_phase_parallel_safe(monkeypatch):
    """Send 多实例并行安全：增量键集合恰为 {"phase_drafts"}，只写自己负责的索引。

    LangGraph 无 reducer 时状态整值覆盖，若节点返回多个键或别的键，
    并行实例会互相吞掉更新——此断言防该回归。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)

    for index, phase_outline in enumerate(outline["phases"]):
        payload = {
            "request": REQUEST,
            "phase_outline": phase_outline,
            "phase_index": index,
            "plan_context": nodes.build_plan_context(outline, index),
        }
        result = await nodes.expand_phase_node(payload)

        assert set(result) == {"phase_drafts"}, "顶层只允许 phase_drafts 一个键"
        assert set(result["phase_drafts"]) == {str(index)}, "只写本实例负责的索引"


@pytest.mark.asyncio
async def test_expand_phase_real_path_variables(monkeypatch):
    """真实路径：invoke_structured 收到 JSON 化骨架与首展占位评审意见。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)
    captured: dict[str, Any] = {}

    async def _stub(model_cls, prompt_key, variables):
        captured["model_cls"] = model_cls
        captured["prompt_key"] = prompt_key
        captured["variables"] = variables
        return StudyPhase(name="扩展阶段")

    monkeypatch.setattr(nodes, "invoke_structured", _stub)
    outline = build_mock_outline(REQUEST)

    result = await nodes.expand_phase_node(
        {
            "request": REQUEST,
            "phase_outline": outline["phases"][0],
            "phase_index": 0,
            "plan_context": "计划标题：X",
        }
    )

    assert captured["model_cls"] is nodes.StudyPhase
    assert captured["prompt_key"] == "phase"
    variables = captured["variables"]
    assert variables["plan_context"] == "计划标题：X"
    assert outline["phases"][0]["weekRange"] in variables["phase_outline"], (
        "骨架须 JSON 序列化后注入"
    )
    assert variables["review_notes"] == "（首次生成，无评审意见）", (
        "首展阶段注入占位评审意见"
    )
    assert set(result) == {"phase_drafts"}


# ---------------------------------------------------------------------------
# expand_revision_node：定向回炉
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_revision_keeps_old_draft_on_failure(monkeypatch):
    """重生成失败：旧草稿保留、追加中文备注、revised=True。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*args, **kwargs):
        raise AgentServiceError(
            "模型响应超时或网络异常，请稍后重试，或在设置页更换模型"
        )

    monkeypatch.setattr(nodes, "invoke_structured", _boom)

    old_draft = {"name": "旧阶段草稿"}
    state = _state(
        outline=build_mock_outline(REQUEST),
        phase_drafts={"1": old_draft},
        pending_phase_indexes=[1],
        review_notes=["第一轮评审意见"],
    )

    result = await nodes.expand_revision_node(state)

    assert result["revised"] is True
    assert result["phase_drafts"]["1"] == old_draft, "失败阶段必须保留旧草稿"
    assert result["review_notes"][0] == "第一轮评审意见", "旧评审意见保留"
    added = result["review_notes"][1:]
    assert len(added) == 1, "每个失败阶段恰好追加一条备注"
    assert "第 2 阶段重生成失败" in added[0] and "保留原稿" in added[0]


@pytest.mark.asyncio
async def test_expand_revision_mock_success(monkeypatch):
    """mock 模式回炉：重生成成功即覆盖旧草稿，旧备注保留。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    state = _state(
        outline=outline,
        phase_drafts={"0": {"name": "旧稿"}},
        pending_phase_indexes=[0],
        review_notes=["校验发现第 1 阶段字数不足"],
    )

    result = await nodes.expand_revision_node(state)

    assert result["revised"] is True
    assert result["phase_drafts"]["0"]["name"] == outline["phases"][0]["name"], (
        "成功后覆盖旧稿"
    )
    assert result["review_notes"] == ["校验发现第 1 阶段字数不足"]


# ---------------------------------------------------------------------------
# assemble_node：拼装 + 程序化校验
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_assemble_node_sorts_and_validates(monkeypatch):
    """拼装：草稿按索引升序排列；mock 两级数据应零校验问题、零 pending。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts = {
        str(index): build_mock_phase(REQUEST, phase_outline, index)
        for index, phase_outline in enumerate(outline["phases"])
    }

    state = _state(
        outline=outline,
        phase_drafts={"2": drafts["2"], "0": drafts["0"], "1": drafts["1"]},  # 乱序
        review_notes=["备注一", "备注二"],
    )

    result = await nodes.assemble_node(state)

    assert set(result) == {"plan", "validation_issues", "pending_phase_indexes"}
    plan = result["plan"]
    assert [p["name"] for p in plan["phases"]] == [
        drafts[str(i)]["name"] for i in range(len(drafts))
    ], "阶段必须按索引升序拼装"
    assert plan["title"] == outline["title"]
    assert plan["overview"] == outline["overview"]
    assert plan["estimatedTotalHours"] == outline["estimatedTotalHours"]
    assert plan["resources"] == outline["resources"]
    assert plan["reviewNotes"] == "备注一\n备注二", "评审备注合并为字符串"
    assert result["validation_issues"] == [], "mock 计划应零校验问题"
    assert result["pending_phase_indexes"] == []


@pytest.mark.asyncio
async def test_assemble_node_reports_failed_indexes(monkeypatch):
    """拼装：校验不通过时把失败阶段索写入 pending_phase_indexes。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    broken_phase = build_mock_phase(REQUEST, outline["phases"][0], 0)
    broken_phase["project"] = None  # 破坏：缺阶段项目

    state = _state(
        outline=outline,
        phase_drafts={
            "0": broken_phase,
            "1": build_mock_phase(REQUEST, outline["phases"][1], 1),
        },
    )

    result = await nodes.assemble_node(state)

    assert result["validation_issues"], "应产出校验问题描述"
    assert result["pending_phase_indexes"] == [0], "失败索引指向被破坏的阶段"


# ---------------------------------------------------------------------------
# review_node：LLM 评审 + 定向回炉索引
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_review_writes_pending(monkeypatch):
    """LLM 评审给 failed_phase_indexes=[1] 时，增量写入 pending_phase_indexes=[1]。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _stub(model_cls, prompt_key, variables):
        assert model_cls is nodes.ReviewOutput
        assert prompt_key == "review"
        assert "phases" in variables["plan"], "待评审计划须 JSON 注入"
        assert variables["validation_issues"] == "（无程序化校验问题）", (
            "校验问题为空时传占位说明"
        )
        return ReviewOutput(
            passed=False,
            issues=["第 2 阶段项目验收标准不可测"],
            suggestions=["补充可量化验收标准"],
            failed_phase_indexes=[1],
        )

    monkeypatch.setattr(nodes, "invoke_structured", _stub)

    state = _state(
        plan={"phases": []},
        validation_issues=[],
        pending_phase_indexes=[],
    )

    result = await nodes.review_node(state)

    assert result["passed"] is False
    assert result["pending_phase_indexes"] == [1]
    assert result["review_notes"] == ["第 2 阶段项目验收标准不可测"]
    assert result["suggestions"] == ["补充可量化验收标准"]


@pytest.mark.asyncio
async def test_review_merges_pending_with_failed_indexes(monkeypatch):
    """校验失败与评审失败索引取并集（去重升序）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _stub(model_cls, prompt_key, variables):
        assert "字数不足" in variables["validation_issues"], "校验问题须注入评审提示词"
        return ReviewOutput(passed=False, failed_phase_indexes=[2])

    monkeypatch.setattr(nodes, "invoke_structured", _stub)

    state = _state(
        plan={"phases": []},
        validation_issues=["第 1 阶段深入内容字数不足"],
        pending_phase_indexes=[0, 2],
    )

    result = await nodes.review_node(state)

    assert result["pending_phase_indexes"] == [0, 2], "并集去重升序"


@pytest.mark.asyncio
async def test_review_mock_skips(monkeypatch):
    """mock / 无 Key：跳过评审直接放行（passed=True），全链路可用。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    result = await nodes.review_node(_state(plan={"phases": []}))

    assert result["passed"] is True
