"""知识点覆盖率校验：正向命中、低覆盖判失败、缺省跳过、mock 对齐。"""

from __future__ import annotations

from app import validators
from app.mock import build_mock_chapter, build_mock_outline

# 正文换行一律用 chr(10) 拼接，避免任何反斜杠字面量（本项目 heredoc 会折叠它们）
NL = chr(10)


def _content(sections: list[str]) -> str:
    """构造一段能过其余校验的正文（practice 章型，无 mermaid 块）。"""
    parts = [
        "## 章标题",
        # 带两个具体名词（接口名 + 版本号）：具体名词密度是硬校验，
        # 全是「内容」重复的正文会被判泛泛而谈，那样测的就不是覆盖率了
        "### 学习目标" + NL + NL + "能独立完成一个最小示例（`Client.run` 配 `max_retries`，v1.2.3，日志看 RUST_LOG）。",
        "### 前置知识" + NL + NL + "无。",
    ]
    parts.extend("## " + name + NL + NL + "内容" * 200 for name in sections)
    if not sections:
        # 无知识点小节时也要过 1000 字硬底线，否则测的是「正文过短」而非覆盖率
        parts.append("## 基础内容" + NL + NL + "内容" * 600)
    parts.append("### 动手练习" + NL + NL + "照做一遍并记录结果。")
    parts.append(
        "### 自检清单"
        + NL
        + NL
        + "1. 能复述主线。2. 能解释每一步。3. 能指出一个误用。"
    )
    return NL.join(parts)


def _plan(sections: list[str]) -> dict:
    return {
        "title": "计划",
        "overview": "",
        "estimatedTotalHours": 40,
        "phases": [
            {
                "name": "阶段",
                "weekRange": "第 1-4 周",
                "objectives": ["目标"],
                "chapters": [
                    {
                        "title": "章标题",
                        "summary": "",
                        "contentMd": _content(sections),
                        "kind": "practice",
                        "keyTerms": [],
                    }
                ],
                "practice": [],
                "assignments": [],
                "project": {
                    "name": "项目",
                    "description": "",
                    "background": "",
                    "steps": ["第一步", "第二步"],
                    "estimatedHours": 8,
                    "acceptanceCriteria": ["能跑通", "能解释"],
                },
            }
        ],
        "resources": [],
        "reviewNotes": "",
    }


_POINTS = ["事件循环调度", "宏任务队列", "微任务优先级"]


def test_coverage_passes_when_sections_match_points() -> None:
    """三个知识点都有对应小节 → 不报覆盖率问题、不判失败。"""
    issues, _failed, failed_chapters = validators.validate_plan(
        _plan(_POINTS), 10, 4, {(0, 0): _POINTS}
    )
    assert not [i for i in issues if "知识点覆盖率" in i]
    assert failed_chapters == []


def test_coverage_fails_when_most_points_missed() -> None:
    """只覆盖 1/3（33% < 60%）→ 报覆盖率不足并判该章失败。"""
    issues, _failed, failed_chapters = validators.validate_plan(
        _plan(["事件循环调度"]), 10, 4, {(0, 0): _POINTS}
    )
    coverage = [i for i in issues if "知识点覆盖率" in i]
    assert coverage, issues
    assert "33%" in coverage[0]
    assert failed_chapters == ["0:0"]


def test_coverage_skipped_without_points() -> None:
    """不传知识点（旧路径 / mock 无大纲）→ 完全跳过覆盖率校验。"""
    issues, _failed, failed_chapters = validators.validate_plan(_plan([]), 10, 4)
    assert not [i for i in issues if "知识点覆盖率" in i]
    assert failed_chapters == []


def test_extra_sections_only_warn_when_multiple() -> None:
    """单个过渡小节不提示；两个以上与知识点对不上的小节才记 issue。"""
    issues, _failed, _fc = validators.validate_plan(
        _plan(list(_POINTS)), 10, 4, {(0, 0): _POINTS}
    )
    assert not [i for i in issues if "对不上" in i]

    issues2, _failed2, _fc2 = validators.validate_plan(
        _plan([*_POINTS, "扩展阅读甲", "扩展阅读乙"]), 10, 4, {(0, 0): _POINTS}
    )
    assert [i for i in issues2 if "对不上" in i], issues2


def test_mock_outline_points_align_with_titles() -> None:
    """mock 并行数组与 chapterTitles 等长同序，且每章都不是空的。"""
    outline = build_mock_outline({"goal": "学 Rust", "weeks": 8, "weeklyHours": 10})
    for phase in outline["phases"]:
        titles = phase["chapterTitles"]
        points = phase["chapterKnowledgePoints"]
        assert len(points) == len(titles)
        assert all(len(group) >= 1 for group in points)


def test_mock_chapter_sections_cover_its_points() -> None:
    """mock 正文的小节标题与知识点一字不差 → 覆盖率校验必然命中。"""
    request = {"goal": "学 Rust", "weeks": 8, "weeklyHours": 10}
    outline = build_mock_outline(request)
    phase = outline["phases"][0]
    chapter = build_mock_chapter(request, phase, 0)
    for point in phase["chapterKnowledgePoints"][0]:
        assert "## " + point in chapter["contentMd"]
