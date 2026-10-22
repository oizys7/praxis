"""nodes.py 章级扇出节点测试：mock 路径全覆盖 + 关键失败路径。

所有用例均不触网：mock 路径 monkeypatch settings.allow_mock_llm=True；
真实路径 monkeypatch invoke_structured / has_llm_config 控制行为。
核心用例名与实现计划简报逐字对应（test_expand_chapter_parallel_safe /
test_expand_revision_keeps_old_draft_on_failure / test_review_writes_pending /
test_assemble_extras_missing_retries_once）。
"""

from typing import Any, cast

import pytest

from app import nodes
from app.config import settings
from app.errors import AgentServiceError
from app.mock import (
    build_mock_chapter,
    build_mock_extras,
    build_mock_outline,
)
from app.nodes import AgentState
from app.schemas import (
    Chapter,
    PhaseExtrasOutput,
    PhaseProject,
    PlanRequest,
    ReviewOutput,
)

# camelCase 请求，形状等同 PlanRequest.model_dump(by_alias=True)
REQUEST = {
    "goal": "全栈 Web 开发",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["前端", "Python"],
    "background": "写过一些脚本",
}


def _state(**overrides: Any) -> AgentState:
    """构造节点测试用的最小 AgentState（request 默认注入，同图入口约定）。"""
    state: dict[str, Any] = {"request": REQUEST}
    state.update(overrides)
    return cast(AgentState, state)


def _drafts_and_extras(
    outline: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """用确定性 mock 填满章草稿复合键与阶段轻装（测试共用基底）。"""
    drafts: dict[str, Any] = {}
    extras: dict[str, Any] = {}
    for index, phase_outline in enumerate(outline["phases"]):
        for chapter_index in range(len(phase_outline["chapterTitles"])):
            drafts[f"{index}:{chapter_index}"] = build_mock_chapter(
                REQUEST, phase_outline, chapter_index
            )
        extras[str(index)] = build_mock_extras(REQUEST, phase_outline, index)
    return drafts, extras


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
        assert len(phase["chapterTitles"]) >= 1, "每阶段至少 1 个章题（章级扇出前提）"


@pytest.mark.asyncio
async def test_outline_node_without_key_raises(monkeypatch):
    """非 mock 且未配置 Key：抛业务错误（引导文案），而非裸异常。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: False)

    with pytest.raises(AgentServiceError):
        await nodes.outline_node(_state())


# ---------------------------------------------------------------------------
# expand_chapter_node：章级扇出（Send 目标节点）
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_chapter_parallel_safe(monkeypatch):
    """章级 Send 多实例并行安全：增量键集合恰为 {"chapter_drafts"}，只写自己的复合键。

    LangGraph 用 reducer 合并行实例增量，若节点返回别的键或写别人的键，
    并行实例会互相吞掉更新——此断言防该回归。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)

    for index, phase_outline in enumerate(outline["phases"]):
        for chapter_index in range(len(phase_outline["chapterTitles"])):
            payload = {
                "request": REQUEST,
                "phase_outline": phase_outline,
                "phase_index": index,
                "chapter_index": chapter_index,
                "plan_context": nodes.build_plan_context(outline, index),
            }
            result = await nodes.expand_chapter_node(payload)

            assert set(result) == {"chapter_drafts"}, "顶层只允许 chapter_drafts 一个键"
            assert set(result["chapter_drafts"]) == {f"{index}:{chapter_index}"}, (
                "只写本实例负责的复合键"
            )
            chapter = result["chapter_drafts"][f"{index}:{chapter_index}"]
            assert chapter["title"] == phase_outline["chapterTitles"][chapter_index], (
                "章题与骨架逐字一致"
            )


@pytest.mark.asyncio
async def test_expand_chapter_real_path_variables(monkeypatch):
    """真实路径：invoke_structured 收到 Chapter 提示词与首展占位评审意见。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)
    captured: dict[str, Any] = {}

    async def _stub(model_cls, prompt_key, variables):
        captured["model_cls"] = model_cls
        captured["prompt_key"] = prompt_key
        captured["variables"] = variables
        return Chapter(title="核心概念入门", summary="概述", content_md="# 正文")

    monkeypatch.setattr(nodes, "invoke_structured", _stub)
    outline = build_mock_outline(REQUEST)
    phase_outline = outline["phases"][0]

    result = await nodes.expand_chapter_node(
        {
            "request": REQUEST,
            "phase_outline": phase_outline,
            "phase_index": 0,
            "chapter_index": 0,
            "plan_context": "计划标题：X",
        }
    )

    assert captured["model_cls"] is nodes.Chapter
    assert captured["prompt_key"] == "chapter"
    variables = captured["variables"]
    assert variables["chapter_title"] == phase_outline["chapterTitles"][0]
    assert variables["plan_context"] == "计划标题：X"
    assert variables["review_notes"] == "（首次生成，无评审意见）", (
        "首展章注入占位评审意见"
    )
    assert variables["phase_name"] == phase_outline["name"]
    # 兄弟章排除自身
    assert (
        variables["sibling_chapter_titles"] == f"- {phase_outline['chapterTitles'][1]}"
    )
    assert set(result) == {"chapter_drafts"}
    # 章型以大纲为准强制回写：大纲未给 chapterKinds → 兜底 concept
    assert result["chapter_drafts"] == {
        "0:0": {
            "title": "核心概念入门",
            "summary": "概述",
            "contentMd": "# 正文",
            "kind": "concept",
            "keyTerms": [],
        }
    }


@pytest.mark.asyncio
async def test_expand_chapter_without_key_raises(monkeypatch):
    """非 mock 且未配置 Key：章实例抛业务错误（首展失败即 raise 的失败策略）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: False)

    with pytest.raises(AgentServiceError):
        await nodes.expand_chapter_node(
            {
                "request": REQUEST,
                "phase_outline": build_mock_outline(REQUEST)["phases"][0],
                "phase_index": 0,
                "chapter_index": 0,
                "plan_context": "",
            }
        )


# ---------------------------------------------------------------------------
# expand_phase_wrap_node：阶段轻装（Send 目标节点，轻量调用）
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_phase_wrap_parallel_safe(monkeypatch):
    """wrap 多实例并行安全：增量键集合恰为 {"phase_extras"}，只写本阶段索引。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)

    for index, phase_outline in enumerate(outline["phases"]):
        payload = {
            "request": REQUEST,
            "phase_outline": phase_outline,
            "phase_index": index,
        }
        result = await nodes.expand_phase_wrap_node(payload)

        assert set(result) == {"phase_extras"}, "顶层只允许 phase_extras 一个键"
        assert set(result["phase_extras"]) == {str(index)}, "只写本实例负责的阶段索引"
        extras = result["phase_extras"][str(index)]
        assert set(extras) == {"practice", "assignments", "project"}
        assert extras["project"]["acceptanceCriteria"], "验收标准非空"


@pytest.mark.asyncio
async def test_expand_phase_wrap_real_path_variables(monkeypatch):
    """真实路径：invoke_structured 收到轻量提示词 PhaseExtrasOutputienne 产出三键。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)
    captured: dict[str, Any] = {}

    async def _stub(model_cls, prompt_key, variables):
        captured["model_cls"] = model_cls
        captured["prompt_key"] = prompt_key
        captured["variables"] = variables
        return PhaseExtrasOutput(
            practice=["练"],
            assignments=["作业"],
            project=PhaseProject(name="项目名"),
        )

    monkeypatch.setattr(nodes, "invoke_structured", _stub)
    outline = build_mock_outline(REQUEST)

    result = await nodes.expand_phase_wrap_node(
        {"request": REQUEST, "phase_outline": outline["phases"][1], "phase_index": 1}
    )

    assert captured["model_cls"] is nodes.PhaseExtrasOutput
    assert captured["prompt_key"] == "phase_extras"
    variables = captured["variables"]
    assert outline["phases"][1]["name"] in variables["phase_outline"], (
        "骨架须 JSON 序列化后注入"
    )
    assert variables["weekly_hours"] == "10" and variables["weeks"] == "12"
    # 学习者画像必须进入提示词（此前只注入骨架与学时，练习/项目完全不针对性）
    assert {"goal", "level", "interests", "background"} <= set(variables)
    assert variables["goal"] == REQUEST["goal"]
    assert set(result) == {"phase_extras"}
    assert result["phase_extras"]["1"]["practice"] == ["练"]


# ---------------------------------------------------------------------------
# expand_revision_node：章级定向回炉
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_revision_keeps_old_draft_on_failure(monkeypatch):
    """章重生成失败：增量不含该键（旧稿保旧）、追加中文备注、revised=True。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*args, **kwargs):
        raise AgentServiceError(
            "模型响应超时或网络异常，请稍后重试，或在设置页更换模型"
        )

    monkeypatch.setattr(nodes, "invoke_structured", _boom)
    outline = build_mock_outline(REQUEST)
    broken_break, _ = _drafts_and_extras(outline)

    state = _state(
        outline=outline,
        chapter_drafts=broken_break,
        phase_extras=build_mock_extras(REQUEST, outline["phases"][0], 0)
        and {"0": build_mock_extras(REQUEST, outline["phases"][0], 0)},
        pending_phase_indexes=[0],
        failed_chapter_keys=["0:0"],
        review_notes=["第一轮评审意见"],
    )

    result = await nodes.expand_revision_node(state)

    assert result["revised"] is True
    assert result["chapter_drafts"] == {}, "失败章不写增量：状态不发键即保旧稿"
    assert result["phase_extras"] == {}, "章级失败路径不动轻装"
    assert result["review_notes"][0] == "第一轮评审意见", "旧评审意见保留"
    added = result["review_notes"][1:]
    assert len(added) == 1, "每个失败章恰好追加一条备注"
    assert "「核心概念入门」章重生成失败" in added[0] and "保留原稿" in added[0]


@pytest.mark.asyncio
async def test_expand_revision_mock_success(monkeypatch):
    """mock 模式回炉：成功章覆盖旧键，未点名章不写增量，旧备注保留。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    chapter_drafts = {
        f"0:{c}": build_mock_chapter(REQUEST, outline["phases"][0], c) for c in range(2)
    }
    chapter_drafts["0:1"]["contentMd"] = "太短"  # 待回炉的短章旧稿
    state = _state(
        outline=outline,
        chapter_drafts=chapter_drafts,
        phase_extras={"0": build_mock_extras(REQUEST, outline["phases"][0], 0)},
        pending_phase_indexes=[0],
        failed_chapter_keys=["0:1"],
        review_notes=["校验发现第 1 阶段第 2 章字数不足"],
    )

    result = await nodes.expand_revision_node(state)

    assert result["revised"] is True
    assert set(result["chapter_drafts"]) == {"0:1"}, "只重扩失败章"
    assert result["chapter_drafts"]["0:1"]["contentMd"] != "太短", (
        "成功后以确定性 mock 覆盖旧稿"
    )
    assert result["phase_extras"] == {}, "章级失败路径不动轻装"
    assert result["review_notes"] == ["校验发现第 1 阶段第 2 章字数不足"]


@pytest.mark.asyncio
async def test_expand_revision_structural_redoes_all_chapters_and_extras(monkeypatch):
    """结构性回炉（无失败章）：该阶段全部章重扩 + 轻装（extras）重做。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)

    state = _state(
        outline=outline,
        chapter_drafts=drafts,
        phase_extras=extras,
        pending_phase_indexes=[0],
        failed_chapter_keys=[],  # 无失败章 → 结构性回炉
        review_notes=["第 1 阶段项目验收标准不可测"],
    )

    result = await nodes.expand_revision_node(state)

    assert result["revised"] is True
    phase_titles = outline["phases"][0]["chapterTitles"]
    assert set(result["chapter_drafts"]) == {
        f"0:{c}" for c in range(len(phase_titles))
    }, "全部章重扩"
    assert set(result["phase_extras"]) == {"0"}, "结构回炉连带轻装重做"


# ---------------------------------------------------------------------------
# assemble_node：章级重组 + 程序化校验
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_assemble_node_sorts_and_validates(monkeypatch):
    """章级重组：复合键乱序也能按 (阶段序, 章序) 重组；mock 数据零校验问题。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    reversed_drafts = dict(reversed(list(drafts.items())))  # 键序打乱注入

    state = _state(
        outline=outline,
        chapter_drafts=reversed_drafts,
        phase_extras=extras,
        review_notes=["备注一", "备注二"],
    )

    result = await nodes.assemble_node(state)

    assert set(result) == {
        "plan",
        "validation_issues",
        "pending_phase_indexes",
        "failed_chapter_keys",
        "phase_extras",
    }
    plan = result["plan"]
    assert len(plan["phases"]) == len(outline["phases"]), "阶段按大纲序重组"
    for index, phase in enumerate(plan["phases"]):
        assert phase["name"] == outline["phases"][index]["name"], "阶段名与大纲一致"
        titles = outline["phases"][index]["chapterTitles"]
        assert [c["title"] for c in phase["chapters"]] == list(titles), (
            "章节按章索引升序重组"
        )
        assert phase["practice"] and phase["project"], "轻装（practice/project）合入"
    assert plan["title"] == outline["title"]
    assert plan["overview"] == outline["overview"]
    assert plan["estimatedTotalHours"] == outline["estimatedTotalHours"]
    assert plan["resources"] == outline["resources"]
    assert plan["reviewNotes"] == "备注一\n备注二", "评审备注合并为字符串"
    assert result["validation_issues"] == [], "mock 计划应零校验问题"
    assert result["pending_phase_indexes"] == []
    assert result["failed_chapter_keys"] == []
    assert result["phase_extras"] == {}, "extras 齐备时不产生补放增量"


@pytest.mark.asyncio
async def test_assemble_node_reports_failed_chapter_and_phase(monkeypatch):
    """失败章标定 + 阶段罚项：短章写入 failed_chapter_keys，所在阶段进 pending。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    drafts["1:1"] = {**drafts["1:1"], "contentMd": "太短"}  # 破坏：第 2 阶段第 2 章短

    state = _state(outline=outline, chapter_drafts=drafts, phase_extras=extras)

    result = await nodes.assemble_node(state)

    assert result["failed_chapter_keys"] == ["1:1"], "仅标定被破坏的章"
    assert result["pending_phase_indexes"] == [1], "失败章所在阶段进定向回炉索引"
    assert result["validation_issues"], "应产出校验问题描述"


@pytest.mark.asyncio
async def test_assemble_extras_retry_failure_still_flags_phase(monkeypatch):
    """wrap 补放也失败：不阻断拼装，缺失项目仍由 validators 罚项进 pending。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    async def _boom(*args, **kwargs):
        raise AgentServiceError("补放失败")

    monkeypatch.setattr(nodes, "_expand_phase_extras", _boom)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    del extras["0"]  # 破坏：第 1 阶段轻装缺失且补放失败

    result = await nodes.assemble_node(
        _state(outline=outline, chapter_drafts=drafts, phase_extras=dict(extras))
    )

    assert result["phase_extras"] == {}, "补放失败不产增量"
    assert result["pending_phase_indexes"] == [0], "缺项目仍记入阶段罚项"
    assert result["failed_chapter_keys"] == [], "章完好，仅轻装罚项"


@pytest.mark.asyncio
async def test_assemble_missing_chapter_becomes_placeholder_and_pending(monkeypatch):
    """章实例缺失：占位空章不阻断拼装，标定失败章 + 阶段罚项供回炉重建。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    del drafts["2:1"]  # 破坏：第 3 阶段第 2 章实例缺失

    state = _state(outline=outline, chapter_drafts=drafts, phase_extras=extras)

    result = await nodes.assemble_node(state)

    missing = result["plan"]["phases"][2]["chapters"][1]
    assert missing["contentMd"] == "", "缺失实例以空章占位"
    assert result["failed_chapter_keys"] == ["2:1"]
    assert result["pending_phase_indexes"] == [2]


@pytest.mark.asyncio
async def test_assemble_extras_missing_retries_once(monkeypatch):
    """wrap 缺失重试：assemble 当场补产一次 extras 增量，防轻调用丢失。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    del extras["1"]  # 破坏：第 2 阶段轻装缺失

    state = _state(outline=outline, chapter_drafts=drafts, phase_extras=dict(extras))

    result = await nodes.assemble_node(state)

    retried = result["phase_extras"]
    assert set(retried) == {"1"}, "补放增量只含缺失阶段"
    assert retried["1"]["project"]["name"].strip(), "补放产物含合法项目"
    assert (
        result["plan"]["phases"][1]["project"]["name"]
        == (retried["1"]["project"]["name"])
    ), "补放结果直接合入计划"
    assert result["pending_phase_indexes"] == [], "补放成功后无阶段罚项"


@pytest.mark.asyncio
async def test_assemble_extras_retry_real_path_uses_light_prompt(monkeypatch):
    """真实路径补放：走 phase_extras 轻提示词，增量形状与 mock 一致。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)
    calls: list[tuple[Any, str]] = []

    async def _stub(model_cls, prompt_key, variables):
        calls.append((model_cls, prompt_key))
        return PhaseExtrasOutput(
            practice=["练"],
            assignments=["作业"],
            project=PhaseProject(name="补放项目"),
        )

    monkeypatch.setattr(nodes, "invoke_structured", _stub)
    outline = build_mock_outline(REQUEST)
    drafts, extras = _drafts_and_extras(outline)
    del extras["0"]

    result = await nodes.assemble_node(
        _state(outline=outline, chapter_drafts=drafts, phase_extras=dict(extras))
    )

    assert calls == [(nodes.PhaseExtrasOutput, "phase_extras")], "恰一次轻量补放"
    assert result["phase_extras"]["0"]["project"]["name"] == "补放项目"
    assert result["plan"]["phases"][0]["project"]["name"] == "补放项目"


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
        # 评审输入是结构摘要（不含正文全文）：6 万字全量正文会撑爆上下文
        assert "计划标题" in variables["plan_summary"], "待评审计划须注入结构摘要"
        assert "contentMd" not in variables["plan_summary"], "摘要不得含正文全文"
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


# ---------------------------------------------------------------------------
# clarify 判据：轻量模糊度检查（反馈③用户裁定，2026-10-15）
# ---------------------------------------------------------------------------


def _req_with_goal(goal: str) -> PlanRequest:
    """围绕 _detect_issues 构造仅 goal 不同的 PlanRequest。"""
    return PlanRequest.model_validate({**REQUEST, "goal": goal})


def test_clarify_broad_goal_prefix_form():
    """「学/做 + 主题」纯前缀形态（如 学Python）触发 goal_too_broad。"""
    issues = dict(nodes._detect_issues(_req_with_goal("学Python")))
    assert "goal_too_broad" in issues
    assert "goal_too_short" not in issues


def test_clarify_specific_goal_passes():
    """带场景/产出的详细目标不受模糊度判据影响，仍直接放行。"""
    issues = dict(
        nodes._detect_issues(_req_with_goal("自学 Python 爬虫并做出数据抓取系统"))
    )
    assert "goal_too_broad" not in issues
    assert "goal_too_short" not in issues


def test_clarify_extra_short_goal_only_short():
    """3 字以下目标只走 goal_too_short；模糊度判据不重复报。"""
    issues = dict(nodes._detect_issues(_req_with_goal("学英语")))
    assert "goal_too_short" in issues
    assert "goal_too_broad" not in issues


# ---------------------------------------------------------------------------
# 首展容错（2026-10-21）：单章/单阶段失败不再炸断整条流
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_expand_chapter_llm_failure_returns_empty_delta(monkeypatch):
    """单章偶发失败：返回空增量，交给 assemble 兜底重扩（不写键、不抛）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*_args, **_kwargs):
        raise AgentServiceError("模型响应超时，请稍后重试")

    monkeypatch.setattr(nodes, "_expand_chapter_draft", _boom)
    outline = build_mock_outline(REQUEST)

    result = await nodes.expand_chapter_node(
        {
            "request": REQUEST,
            "phase_outline": outline["phases"][0],
            "phase_index": 0,
            "chapter_index": 1,
            "plan_context": "",
        }
    )

    assert result == {}, "失败不写 chapter_drafts 键，避免覆盖并行实例的成果"


@pytest.mark.asyncio
async def test_expand_phase_wrap_llm_failure_returns_empty_delta(monkeypatch):
    """阶段收尾失败：返回空增量，assemble 会当场补产一次。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*_args, **_kwargs):
        raise AgentServiceError("被限流")

    monkeypatch.setattr(nodes, "_expand_phase_extras", _boom)
    outline = build_mock_outline(REQUEST)

    result = await nodes.expand_phase_wrap_node(
        {"request": REQUEST, "phase_outline": outline["phases"][0], "phase_index": 0}
    )

    assert result == {}


@pytest.mark.asyncio
async def test_review_llm_failure_passes_with_note(monkeypatch):
    """评审是辅助能力：调用失败按放行处理，保留 validators 给出的待修阶段。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*_args, **_kwargs):
        raise AgentServiceError("评审超时")

    monkeypatch.setattr(nodes, "invoke_structured", _boom)

    result = await nodes.review_node(
        {"plan": {"title": "t"}, "validation_issues": [], "pending_phase_indexes": [2]}
    )

    assert result["passed"] is True
    assert result["pending_phase_indexes"] == [2]
    assert result["suggestions"] == []
    assert "评审未执行" in result["review_notes"][0]


@pytest.mark.asyncio
async def test_expand_chapter_unexpected_error_also_isolated(monkeypatch):
    """非业务异常（SDK 漏网 / 属性错误等）同样只让本章失败，不炸断整条流。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*_args, **_kwargs):
        raise RuntimeError("with_structured_output 返回了意外形状")

    monkeypatch.setattr(nodes, "_expand_chapter_draft", _boom)
    outline = build_mock_outline(REQUEST)

    result = await nodes.expand_chapter_node(
        {
            "request": REQUEST,
            "phase_outline": outline["phases"][0],
            "phase_index": 0,
            "chapter_index": 0,
            "plan_context": "",
        }
    )

    assert result == {}, "任何异常都不写键、不外抛，其余章节照常生成"


@pytest.mark.asyncio
async def test_review_unexpected_error_also_passes(monkeypatch):
    """评审遇到非业务异常同样放行（不能因评审意外把整份计划废掉）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    async def _boom(*_args, **_kwargs):
        raise RuntimeError("unexpected")

    monkeypatch.setattr(nodes, "invoke_structured", _boom)

    result = await nodes.review_node(
        {"plan": {"title": "t"}, "validation_issues": [], "pending_phase_indexes": []}
    )

    assert result["passed"] is True
    assert "评审未执行" in result["review_notes"][0]


# ---------------------------------------------------------------------------
# 章型 / 术语表 / 评审摘要（2026-10-21 kind 与 review 缩输入）
# ---------------------------------------------------------------------------


def test_plan_summary_excludes_full_content():
    """评审输入是结构摘要：给出章节结构与字数，但绝不包含正文全文。"""
    plan = {
        "title": "计划",
        "overview": "概述",
        "estimatedTotalHours": 120,
        "phases": [
            {
                "name": "第一阶段",
                "weekRange": "第 1-4 周",
                "objectives": ["目标一"],
                "chapters": [
                    {
                        "title": "变量与类型",
                        "summary": "讲清类型系统",
                        "kind": "concept",
                        "keyTerms": ["静态类型"],
                        "contentMd": "### 学习目标\n\n" + "正" * 1200,
                    }
                ],
                "project": {"name": "小工具"},
            }
        ],
        "resources": [{"title": "某本书"}],
    }
    summary = nodes._plan_summary(plan)
    assert "变量与类型" in summary
    assert "[concept]" in summary
    assert "字；小节：" in summary, "摘要须标注正文长度与小节命中情况"
    assert "静态类型" in summary
    assert "正" * 100 not in summary, "摘要不得含正文全文"


def test_build_glossary_dedupes_and_keeps_first_chapter():
    """术语表由 keyTerms 去重汇总：同名术语只留首次出现的章节。"""
    phases = [
        {
            "chapters": [
                {"title": "第一章", "keyTerms": ["闭包", "作用域"]},
                {"title": "第二章", "keyTerms": ["闭包", "柯里化"]},
            ]
        }
    ]
    glossary = nodes._build_glossary(phases)
    assert [entry["term"] for entry in glossary] == ["闭包", "作用域", "柯里化"]
    assert glossary[0]["chapterTitle"] == "第一章", "重复术语保留首次出现的出处"


def test_build_glossary_respects_limit():
    """术语表上限防止超长计划把响应体撑爆。"""
    phases = [{"chapters": [{"title": "章", "keyTerms": [f"术语{i}" for i in range(100)]}]}]
    assert len(nodes._build_glossary(phases, limit=10)) == 10


def test_chapter_kind_falls_back_to_concept():
    """章型缺失/越界/非法值一律兜底 concept（并行数组的对齐风险吸收点）。"""
    assert nodes._chapter_kind({}, 0) == "concept"
    assert nodes._chapter_kind({"chapterKinds": ["practice"]}, 3) == "concept"
    assert nodes._chapter_kind({"chapterKinds": ["乱写"]}, 0) == "concept"
    assert nodes._chapter_kind({"chapterKinds": ["project"]}, 0) == "project"
