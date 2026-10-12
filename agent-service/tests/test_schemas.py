"""schemas.py 协议模型测试：camelCase 别名与字段结构。"""


def test_study_phase_camel_case():
    from app.schemas import StudyPhase
    data = StudyPhase(name="基础").model_dump(by_alias=True)
    assert "weekRange" in data and "chapters" in data and "theory" not in data

def test_chapter_alias():
    from app.schemas import Chapter
    c = Chapter(title="t").model_dump(by_alias=True)
    assert set(c) == {"title", "summary", "mainKnowledge", "deepDives"}
