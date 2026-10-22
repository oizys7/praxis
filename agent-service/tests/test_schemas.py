"""schemas.py 协议模型测试：camelCase 别名与字段结构。"""


def test_study_phase_camel_case():
    from app.schemas import StudyPhase

    data = StudyPhase(name="基础").model_dump(by_alias=True)
    assert "weekRange" in data and "chapters" in data and "theory" not in data


def test_chapter_markdown_shape():
    """Chapter 唯一形状：{title, summary, contentMd}，无结构化字段残留。"""
    from app.schemas import Chapter

    c = Chapter(title="t", summary="s", content_md="## 正文").model_dump(by_alias=True)
    assert set(c) == {"title", "summary", "contentMd", "kind", "keyTerms"}
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
    from pydantic import ValidationError

    from app.schemas import PlanRequest

    for bad in ([0, 1], [1, 8], [-1], [2, 7, 9]):
        with pytest.raises(ValidationError):
            PlanRequest(goal="学 Rust", study_days=bad)


def test_plan_request_study_days_rejects_empty():
    """空数组拒绝：至少保留一个学习日。"""
    import pytest
    from pydantic import ValidationError

    from app.schemas import PlanRequest

    with pytest.raises(ValidationError):
        PlanRequest(goal="学 Rust", study_days=[])


def test_plan_request_study_days_accepts_null_as_default():
    """显式 null（Rust 侧 Option 缺省直传）回退全周默认，而非校验失败。"""
    from app.schemas import PlanRequest

    req = PlanRequest.model_validate({"goal": "学 Rust", "studyDays": None})
    assert req.study_days == [1, 2, 3, 4, 5, 6, 7]
