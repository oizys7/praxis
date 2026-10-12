"""mock.py 两级确定性生成测试：outline 骨架 + 单阶段扩展。

build_mock_outline / build_mock_phase 均为 camelCase dict 进出（与 schemas
模型 model_dump(by_alias=True) 形状一致，但 mock 不 import schemas），
拼出的整计划必须能通过 validate_plan 零 pending（裁决 2）。
"""

from app.mock import build_mock_outline, build_mock_phase, build_mock_plan
from app.validators import validate_plan

# camelCase 请求，形状等同 PlanRequest.model_dump(by_alias=True)
REQUEST = {
    "goal": "全栈 Web 开发",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["前端", "Python"],
    "background": "写过一些脚本",
}


def _collect_keys(obj) -> set:
    """递归收集 dict/list 中出现的所有键，用于断言无残留旧键（如 theory）。"""
    keys: set = set()
    if isinstance(obj, dict):
        for key, value in obj.items():
            keys.add(key)
            keys |= _collect_keys(value)
    elif isinstance(obj, list):
        for item in obj:
            keys |= _collect_keys(item)
    return keys


def _assemble_plan(request: dict) -> dict:
    """按两级接口拼装完整 StudyPlan dict（与 mock shim 同构，供校验器消费）。"""
    outline = build_mock_outline(request)
    return {
        "title": outline["title"],
        "overview": outline["overview"],
        "estimatedTotalHours": outline["estimatedTotalHours"],
        "phases": [
            build_mock_phase(request, phase_outline, index)
            for index, phase_outline in enumerate(outline["phases"])
        ],
        "resources": outline["resources"],
    }


# ---------------------------------------------------------------------------
# 步骤 1 用例：test_mock_outline_shape
# ---------------------------------------------------------------------------


def test_mock_outline_shape():
    outline = build_mock_outline(REQUEST)
    assert {"title", "overview", "estimatedTotalHours", "phases", "resources"} <= (
        set(outline)
    )
    assert outline["estimatedTotalHours"] == REQUEST["weeklyHours"] * REQUEST["weeks"]

    phases = outline["phases"]
    assert len(phases) == 3, "大纲应为 3 个阶段"
    for phase in phases:
        assert {"name", "weekRange", "objectives", "chapterTitles", "projectBrief"} <= (
            set(phase)
        )
        assert len(phase["chapterTitles"]) == 2, "每阶段章节标题恰好 2 条"
        assert phase["objectives"], "阶段目标非空"
        assert phase["projectBrief"].strip(), "项目一句话简介非空"

    # 难度递进：三阶段名称互不相同（基础 -> 进阶 -> 综合项目）
    names = [phase["name"] for phase in phases]
    assert len(set(names)) == 3
    # 周区间 3 条且均非空
    assert all(phase["weekRange"].strip() for phase in phases)
    # 资源形状：{title, type, note}
    for resource in outline["resources"]:
        assert {"title", "type", "note"} <= set(resource)


# ---------------------------------------------------------------------------
# 步骤 1 用例：test_mock_phase_passes_validator
# ---------------------------------------------------------------------------


def test_mock_phase_passes_validator():
    outline = build_mock_outline(REQUEST)
    weekly_hours = REQUEST["weeklyHours"]
    weeks = REQUEST["weeks"]
    for index, phase_outline in enumerate(outline["phases"]):
        phase = build_mock_phase(REQUEST, phase_outline, index)

        # 章节内容契约：deepDives ≥600 字、concepts ≥3、knowledgePoints ≥4、
        # diagrams 含一段 flowchart TD 起头的 mermaid
        assert len(phase["chapters"]) == 2, "每阶段 2 个章节"
        for chapter in phase["chapters"]:
            assert chapter["title"] in phase_outline["chapterTitles"]
            assert chapter["deepDives"], "每章至少 1 篇深入内容"
            for dive in chapter["deepDives"]:
                assert len(dive["content"].strip()) >= 600, (
                    f"深入内容应 ≥600 字，实际 {len(dive['content'].strip())} 字"
                )
            knowledge = chapter["mainKnowledge"]
            assert len(knowledge["concepts"]) >= 3
            assert len(knowledge["knowledgePoints"]) >= 4
            mermaid_sources = [d["mermaid"] for d in knowledge["diagrams"]]
            assert any(
                source.lstrip().startswith("flowchart TD")
                for source in mermaid_sources
            ), "至少一段 flowchart TD 起头的 mermaid"

        # 单阶段放入最小计划容器，必须过校验器零 pending
        plan = {
            "title": outline["title"],
            "overview": outline["overview"],
            "estimatedTotalHours": weekly_hours * weeks,
            "phases": [phase],
            "resources": outline["resources"],
        }
        issues, pending = validate_plan(plan, weekly_hours, weeks)
        assert issues == [], f"index={index} 单阶段应零 issue，实际：{issues}"
        assert pending == [], f"index={index} 单阶段应零 pending，实际：{pending}"


# ---------------------------------------------------------------------------
# 补充用例：整计划零 pending（裁决 2 核心断言）
# ---------------------------------------------------------------------------


def test_full_plan_zero_pending():
    plan = _assemble_plan(REQUEST)
    issues, pending = validate_plan(
        plan, REQUEST["weeklyHours"], REQUEST["weeks"]
    )
    assert issues == [], f"整计划应零 issue，实际：{issues}"
    assert pending == []


# ---------------------------------------------------------------------------
# 补充用例：三套项目模板字段完整性（含 index ≥3 循环取模板的防御）
# ---------------------------------------------------------------------------


def test_project_templates_complete():
    outline = build_mock_outline(REQUEST)
    project_names = []
    for index in range(4):  # 0/1/2 三套模板 + 3 验证循环取模板
        phase_outline = outline["phases"][index % 3]
        project = build_mock_phase(REQUEST, phase_outline, index)["project"]

        assert (project["name"] or "").strip(), "项目名称非空"
        assert (project["description"] or "").strip(), "项目描述非空"
        assert (project["background"] or "").strip(), "项目背景（动机场景）非空"
        assert len(project["steps"]) >= 3, "至少 3 条可执行步骤"
        assert len(project["acceptanceCriteria"]) >= 2, "至少 2 条验收标准"
        for criterion in project["acceptanceCriteria"]:
            assert "了解" not in criterion and "熟悉" not in criterion, (
                f"验收标准须可验证，禁「了解/熟悉」：{criterion}"
            )
        assert isinstance(project["estimatedHours"], (int, float)), "estimatedHours 为数值"
        assert project["estimatedHours"] > 0, "estimatedHours 必须 > 0"
        project_names.append(project["name"])

    assert project_names[3] == project_names[0], "index=3 应循环取模板 0"


# ---------------------------------------------------------------------------
# 补充用例：旧入口 shim 与确定性
# ---------------------------------------------------------------------------


def test_build_mock_plan_shim():
    """旧 build_mock_plan 保留导出：拼出新形状 StudyPlan dict，无 theory 键。"""
    plan = build_mock_plan(REQUEST)
    assert plan["estimatedTotalHours"] == REQUEST["weeklyHours"] * REQUEST["weeks"]
    assert len(plan["phases"]) == 3
    assert {"title", "overview", "estimatedTotalHours", "phases", "resources"} <= (
        set(plan)
    )
    keys = _collect_keys(plan)
    assert "theory" not in keys, "新形状不应残留旧 theory 键"
    assert "chapters" in keys and "deepDives" in keys, "应含 chapters/deepDives 新键"


def test_mock_generation_deterministic():
    """同输入两次生成结果完全一致（确定性契约）。"""
    assert build_mock_outline(REQUEST) == build_mock_outline(REQUEST)
    assert _assemble_plan(REQUEST) == _assemble_plan(REQUEST)
