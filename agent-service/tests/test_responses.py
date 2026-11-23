"""生成终态包络：按正文交付事实区分完整、部分与全失败（研读改版口径）。

- 大纲-only 首展（regen_scope 缺省）：空正文是合法态，骨架拼装完成即 plan；
- 重生成链（regen_scope 存在）：沿用正文完整性三值口径 plan/partial/error。
"""

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
    """重生成链（regen_scope 存在）沿用正文完整性口径。"""
    scope = {"phase": 0, "chapters": None}
    assert build_response({"plan": _plan("正文"), "regen_scope": scope}).status == "plan"
    assert (
        build_response({"plan": _plan("正文", "  "), "regen_scope": scope}).status
        == "partial"
    )

    failed = build_response({"plan": _plan("", "\n"), "regen_scope": scope})
    assert failed.status == "error"
    assert failed.plan is not None, "全失败仍保留大纲骨架供用户重试"
    assert failed.error is not None
    assert failed.error.code == "all_chapters_failed"


def test_build_response_outline_only_completion_is_plan() -> None:
    """大纲-only 首展：全章空正文是合法态（研读页按需生成），终态为 plan。"""
    response = build_response({"plan": _plan("", "")})
    assert response.status == "plan"
    assert response.plan is not None
    assert response.error is None


def test_build_response_keeps_clarification_as_a_separate_terminal() -> None:
    response = build_response(
        {"clarifications": ["请补充已有基础"], "plan": _plan("正文")}
    )
    assert response.status == "clarification"
    assert response.clarifications == ["请补充已有基础"]
