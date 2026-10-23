"""mock.py 两级确定性生成测试：outline 骨架 + markdown 章节。

build_mock_outline / build_mock_phase 均为 camelCase dict 进出（与 schemas
模型 model_dump(by_alias=True) 形状一致，但 mock 不 import schemas）；
每章唯一形状 {title, summary, contentMd}，contentMd 为确定性 markdown
（章节与「深入剖析」节均 ≥1000 字、内联 mermaid 码块，同输入同输出）。
章节校验规则的「零 pending」契约随 validators 重写（任务 19）同步对齐。
"""

from app.mock import (
    build_mock_chapter,
    build_mock_extras,
    build_mock_outline,
    build_mock_phase,
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

_MIN_CHARS = 1000


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
        assert len(phase["chapterTitles"]) == 2, "每阶段章节标题恻好 2 条"
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


def _assert_markdown_chapter(chapter: dict, phase_outline: dict) -> None:
    """单章 markdown 形状断言：唯一三字段 + 字数 + 码块 + 深入剖析节。"""
    assert set(chapter) == { "title", "summary", "contentMd", "kind", "keyTerms", "visualizations", "exercises" }, (
        f"Chapter 键集 {{title, summary, contentMd, kind, keyTerms, visualizations, exercises}}，实际：{set(chapter)}"
    )
    assert chapter["kind"] in {"concept", "practice", "project", "review"}
    assert chapter["keyTerms"], "mock 章节须带术语，供术语表汇总"
    assert chapter["title"] in phase_outline["chapterTitles"]
    assert chapter["summary"].strip()

    content = chapter["contentMd"]
    assert len(content.strip()) >= _MIN_CHARS, (
        f"章节正文应 ≥{_MIN_CHARS} 字，实际 {len(content.strip())} 字"
    )
    # 标题节 + 知识点段落（### 段落式讲解）
    assert content.lstrip().startswith("## "), "正文应以章节标题开头"
    assert "### " in content, "知识点必须段落式讲解（### 小节）"
    # 图示：恰好一个 mermaid 码块，非空正文且 flowchart 起头
    blocks = [seg for seg in content.split("```") if seg.startswith("mermaid")]
    assert len(blocks) == 1, "正文应恰好内联一个 mermaid 码块"
    source = blocks[0][len("mermaid") :].strip()
    assert source.startswith("flowchart "), "码块应为 flowchart 起头"
    # 深入剖析节：单独托底 ≥1000 字（极限输入下也达标）
    dive = content.split("深入剖析")[-1]
    assert len(dive.strip()) >= _MIN_CHARS, (
        f"深入剖析节应 ≥{_MIN_CHARS} 字，实际 {len(dive.strip())} 字"
    )


def test_mock_phase_chapters_markdown_shape():
    outline = build_mock_outline(REQUEST)
    for index, phase_outline in enumerate(outline["phases"]):
        phase = build_mock_phase(REQUEST, phase_outline, index)

        assert len(phase["chapters"]) == 2, "每阶段 2 个章节"
        for chapter in phase["chapters"]:
            _assert_markdown_chapter(chapter, phase_outline)


def test_mock_chapter_single_chapter_markdown_shape():
    """单章重生成路径：与全阶段产出的同章内容一致（复用同一构造器）。"""
    outline = build_mock_outline(REQUEST)
    phase_outline = outline["phases"][2]
    whole = build_mock_phase(REQUEST, phase_outline, 2)["chapters"][1]
    single = build_mock_chapter(REQUEST, phase_outline, 1)
    assert single == whole, "单章重生成与整阶段展开产出必须一致"


def test_mock_markdown_extreme_inputs_still_long():
    """极限输入（极短 goal / 无兴趣无基础 / 极少周数）字数仍达标。"""
    extreme = {"goal": "abc", "weeklyHours": 1, "weeks": 1}
    outline = build_mock_outline(extreme)
    phase = build_mock_phase(extreme, outline["phases"][0], 0)
    for chapter in phase["chapters"]:
        assert len(chapter["contentMd"].strip()) >= _MIN_CHARS


def test_mock_extras_compose_phase():
    """轻装重组一致性：build_mock_phase 的三键与 build_mock_extras 完全一致。"""
    outline = build_mock_outline(REQUEST)
    for index, phase_outline in enumerate(outline["phases"]):
        extras = build_mock_extras(REQUEST, phase_outline, index)
        assert set(extras) == {"practice", "assignments", "project"}
        assert extras["project"]["acceptanceCriteria"], "验收标准非空"
        phase = build_mock_phase(REQUEST, phase_outline, index)
        for key, value in extras.items():
            assert phase[key] == value, f"带装轻装重组应一致：{key}"


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
        assert isinstance(project["estimatedHours"], (int, float)), (
            "estimatedHours 为数值"
        )
        assert project["estimatedHours"] > 0, "estimatedHours 必须 > 0"
        project_names.append(project["name"])

    assert project_names[3] == project_names[0], "index=3 应循环取模板 0"


# ---------------------------------------------------------------------------
# 补充用例：确定性
# ---------------------------------------------------------------------------


def test_mock_generation_deterministic():
    """同输入两次生成结果完全一致（确定性契约，含 markdown 正文）。"""
    assert build_mock_outline(REQUEST) == build_mock_outline(REQUEST)
    outline = build_mock_outline(REQUEST)
    phases = [
        build_mock_phase(REQUEST, phase_outline, index)
        for index, phase_outline in enumerate(outline["phases"])
    ]
    assert phases == [
        build_mock_phase(REQUEST, phase_outline, index)
        for index, phase_outline in enumerate(outline["phases"])
    ]
    assert build_mock_chapter(REQUEST, outline["phases"][0], 0) == build_mock_chapter(
        REQUEST, outline["phases"][0], 0
    )


def test_mock_outline_chapter_kinds_align_with_titles():
    """章型走并行数组：必须与 chapterTitles 等长同序，且取值合法。"""
    request = {
        "goal": "掌握 Web 前端",
        "level": "beginner",
        "weeklyHours": 10,
        "weeks": 12,
        "interests": [],
        "background": "",
    }
    outline = build_mock_outline(request)
    for phase in outline["phases"]:
        assert len(phase["chapterKinds"]) == len(phase["chapterTitles"]), (
            "chapterKinds 必须与 chapterTitles 等长，否则章型会错位"
        )
        assert set(phase["chapterKinds"]) <= {
            "concept",
            "practice",
            "project",
            "review",
        }
