"""schemas.py 协议模型测试：camelCase 别名与字段结构。"""


def test_study_phase_camel_case():
    from app.schemas import StudyPhase

    data = StudyPhase(name="基础").model_dump(by_alias=True)
    assert "weekRange" in data and "chapters" in data and "theory" not in data


def test_chapter_markdown_shape():
    """Chapter 唯一形状：{title, summary, contentMd}，无结构化字段残留。"""
    from app.schemas import Chapter

    c = Chapter(title="t", summary="s", content_md="## 正文").model_dump(by_alias=True)
    assert set(c) == { "title", "summary", "contentMd", "kind", "keyTerms", "visualizations", "exercises" }
    assert c["kind"] == "concept", "章型缺省为 concept"
    assert c["keyTerms"] == []
    assert c["contentMd"] == "## 正文"
    # 结构化字段随 markdown 化退役：Chapter 不再有 main_knowledge / deep_dives 字段
    assert not any("knowledge" in name for name in Chapter.model_fields)
    assert not any("dive" in name for name in Chapter.model_fields)


# ---------------------------------------------------------------------------
# PlanRequest.studyDays：每周学习日（1=周一..7=周日），全周默认（Task E）
# ---------------------------------------------------------------------------


def test_plan_request_study_days_defaults_full_week():
    """缺省 studyDays 归一为全周 [1..7]；wire 别名恒 camelCase studyDays。"""
    from app.schemas import PlanRequest

    req = PlanRequest(goal="学 Rust")
    assert req.study_days == [1, 2, 3, 4, 5, 6, 7]
    wire = req.model_dump(by_alias=True)
    assert wire["studyDays"] == [1, 2, 3, 4, 5, 6, 7]


def test_plan_request_study_days_dedupes_and_sorts():
    """重复项去重、乱序归一为升序（归一化而非拒绝重复）。"""
    from app.schemas import PlanRequest

    req = PlanRequest(goal="学 Rust", study_days=[5, 1, 5, 3])
    assert req.study_days == [1, 3, 5]


def test_plan_request_study_days_rejects_out_of_range():
    """取值越界（0 / 8 / 负数）一律 422 拒绝。"""
    import pytest
    from app.schemas import PlanRequest
    from pydantic import ValidationError

    for bad in ([0, 1], [1, 8], [-1], [2, 7, 9]):
        with pytest.raises(ValidationError):
            PlanRequest(goal="学 Rust", study_days=bad)


def test_plan_request_study_days_rejects_empty():
    """空数组拒绝：至少保留一个学习日。"""
    import pytest
    from app.schemas import PlanRequest
    from pydantic import ValidationError

    with pytest.raises(ValidationError):
        PlanRequest(goal="学 Rust", study_days=[])


def test_plan_request_study_days_accepts_null_as_default():
    """显式 null（Rust 侧 Option 缺省直传）回退全周默认，而非校验失败。"""
    from app.schemas import PlanRequest

    req = PlanRequest.model_validate({"goal": "学 Rust", "studyDays": None})
    assert req.study_days == [1, 2, 3, 4, 5, 6, 7]

def test_chapter_knowledge_points_tolerates_flattened_output() -> None:
    """GLM 把嵌套数组退化成扁平 list[str] 时不得判废整份大纲。

    实测：8 阶段 × 5 章一次报出 40 条 "Input should be a valid list"，
    整次生成连同这次 LLM 调用一起报废。一章一个长字符串仍是有效信息。
    """
    from app.schemas import PhaseOutline as _PhaseOutline

    phase = _PhaseOutline.model_validate(
        {
            "name": "阶段一",
            "weekRange": "第1-2周",
            "objectives": ["目标"],
            "chapterTitles": ["甲", "乙", "丙"],
            "chapterKnowledgePoints": ["知识点A、知识点B", "知识点C", None],
        }
    )
    assert phase.chapter_knowledge_points == [
        ["知识点A、知识点B"],
        ["知识点C"],
        [],
    ]


def test_chapter_knowledge_points_accepts_objects_and_preserves_nested() -> None:
    """对象条目按已知键展平；正常的两层结构原样保留（不破坏正确输出）。"""
    from app.schemas import PhaseOutline as _PhaseOutline

    obj = _PhaseOutline.model_validate(
        {
            "name": "阶段一",
            "weekRange": "第1-2周",
            "objectives": [],
            "chapterTitles": ["甲"],
            "chapterKnowledgePoints": [{"point": "机制", "detail": "细节"}],
        }
    )
    assert obj.chapter_knowledge_points == [["机制", "细节"]]

    nested = _PhaseOutline.model_validate(
        {
            "name": "阶段一",
            "weekRange": "第1-2周",
            "objectives": [],
            "chapterTitles": ["甲"],
            "chapterKnowledgePoints": [["机制", "细节"]],
        }
    )
    assert nested.chapter_knowledge_points == [["机制", "细节"]]
