"""schemas.py 协议模型测试：camelCase 别名与字段结构。"""


def test_study_phase_camel_case():
    from app.schemas import StudyPhase
    data = StudyPhase(name="基础").model_dump(by_alias=True)
    assert "weekRange" in data and "chapters" in data and "theory" not in data


def test_chapter_markdown_shape():
    """Chapter 唯一形状：{title, summary, contentMd}，无结构化字段残留。"""
    from app.schemas import Chapter
    c = Chapter(title="t", summary="s", content_md="## 正文").model_dump(by_alias=True)
    assert set(c) == {"title", "summary", "contentMd"}
    assert c["contentMd"] == "## 正文"
    # 结构化字段随 markdown 化退役：Chapter 不再有 main_knowledge / deep_dives 字段
    assert not any("knowledge" in name for name in Chapter.model_fields)
    assert not any("dive" in name for name in Chapter.model_fields)
