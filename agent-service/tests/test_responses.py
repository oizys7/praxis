"""生成终态包络：按正文交付事实区分完整、部分与全失败。"""

from app.responses import build_response


def _plan(*contents: str) -> dict:
    return {
        "title": "Rust 学习路线",
        "phases": [
            {
                "name": "阶段一",
                "chapters": [
                    {"title": f"第 {index + 1} 章", "contentMd": content}
                    for index, content in enumerate(contents)
                ],
            }
        ],
    }


def test_build_response_reports_complete_partial_and_failed_content() -> None:
    assert build_response({"plan": _plan("正文")}).status == "plan"
    assert build_response({"plan": _plan("正文", "  ")}).status == "partial"

    failed = build_response({"plan": _plan("", "\n")})
    assert failed.status == "error"
    assert failed.plan is not None, "全失败仍保留大纲骨架供用户重试"
    assert failed.error is not None
    assert failed.error.code == "all_chapters_failed"


def test_build_response_keeps_clarification_as_a_separate_terminal() -> None:
    response = build_response(
        {"clarifications": ["请补充已有基础"], "plan": _plan("正文")}
    )
    assert response.status == "clarification"
    assert response.clarifications == ["请补充已有基础"]
