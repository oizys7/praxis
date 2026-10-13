"""regenerate 流式端点与 outline 事件扩展测试（mock 模式，httpx ASGITransport）。

用例名与任务简报逐字对应（test_outline_event_carries_phases /
test_regen_phase_stream / test_regen_chapter_stream /
test_regen_out_of_range）。协议约定：

- outline 事件除 phaseCount/title 外携带 overview + phases
  （[{name, weekRange, chapterTitles}]，数据直出自 PlanOutline）；
- /api/regenerate/stream：chapterIndex 缺省 → 单条 phase 事件；
  chapterIndex 指定 → 单条 chapter 事件（{index, phaseIndex, chapter}，
  chapter 为 by_alias camelCase）；
- done 必为最后一行，plan / error 终态统一走 done；
- 越界 phaseIndex / chapterIndex 不抛 4xx，走 done(error) 包络。
"""

import json
from typing import Any

import httpx
import pytest
from pydantic import ValidationError

from app import nodes
from app.config import settings
from app.errors import AgentServiceError
from app.main import app
from app.mock import build_mock_outline
from app.schemas import Chapter

# 合法请求：goal 足够长、background 非空——避开规则澄清（与 test_stream.py 同源）
VALID_REQUEST = {
    "goal": "学习 Rust 与系统编程",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["系统编程"],
    "background": "写过一些 C++",
}


async def _post_stream(path: str, payload: dict[str, Any]) -> list[dict[str, Any]]:
    """POST 流式端点并逐行解析 NDJSON 事件（每行一个 JSON 对象）。"""
    transport = httpx.ASGITransport(app=app)
    events: list[dict[str, Any]] = []
    async with (
        httpx.AsyncClient(transport=transport, base_url="http://test") as client,
        client.stream("POST", path, json=payload) as response,
    ):
        assert response.status_code == 200
        async for line in response.aiter_lines():
            if line.strip():
                events.append(json.loads(line))
    return events


@pytest.mark.asyncio
async def test_outline_event_carries_phases(monkeypatch):
    """outline 事件扩展载荷：phases[i].chapterTitles 非空，overview 到位。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    events = await _post_stream("/api/generate-plan/stream", VALID_REQUEST)

    first = events[0]
    assert first["type"] == "outline"
    phases = first["phases"]
    assert isinstance(phases, list) and len(phases) == first["phaseCount"]
    for phase in phases:
        assert phase["name"], "阶段名必须非空"
        assert phase["weekRange"], "周区间必须非空"
        assert phase["chapterTitles"], "章节标题列表必须非空"
    assert "overview" in first, "outline 事件必须携带整体概述"


@pytest.mark.asyncio
async def test_regen_phase_stream(monkeypatch):
    """全阶段重生成：mock 模式 [phase, done(status=plan)]，phase.chapters 非空。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)

    events = await _post_stream(
        "/api/regenerate/stream",
        {"request": VALID_REQUEST, "outline": outline, "phaseIndex": 0},
    )

    assert [event["type"] for event in events] == ["phase", "done"]
    phase_event = events[0]
    assert phase_event["index"] == 0, "phase 事件的 index 必须等于请求的 phaseIndex"
    assert phase_event["phase"]["chapters"], "重生成的阶段必须携带章节正文"
    done = events[-1]
    assert done["response"]["status"] == "plan"


@pytest.mark.asyncio
async def test_regen_chapter_stream(monkeypatch):
    """单章重生成：mock 模式 [chapter, done(status=plan)]，结构合法且 camelCase。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)
    chapter_index = 1

    events = await _post_stream(
        "/api/regenerate/stream",
        {
            "request": VALID_REQUEST,
            "outline": outline,
            "phaseIndex": 1,
            "chapterIndex": chapter_index,
        },
    )

    assert [event["type"] for event in events] == ["chapter", "done"]
    chapter_event = events[0]
    assert chapter_event["index"] == chapter_index
    assert chapter_event["phaseIndex"] == 1
    # by_alias camelCase dump：Pydantic 模型（snake_case 属性名）可直接回解析
    try:
        Chapter.model_validate(chapter_event["chapter"])
    except ValidationError as exc:  # pragma: no cover - 断言失败路径
        pytest.fail(f"chapter 载荷不满足 Chapter 模型：{exc}")
    chapter = chapter_event["chapter"]
    assert set(chapter) == {"title", "summary", "contentMd"}, "Chapter 唯一形状"
    content = chapter["contentMd"]
    assert len(content.strip()) >= 1000, "章节 markdown 正文不得少于 1000 字"
    assert "```mermaid" in content, "正文必须内联 mermaid 码块"
    assert "深入剖析" in content, "正文必须有一节深入剖析"
    done = events[-1]
    assert done["response"]["status"] == "plan"


@pytest.mark.asyncio
async def test_regen_service_error_yields_error_done(monkeypatch):
    """业务异常（AgentServiceError）不中断流：恰为一行 error 包络的 done。

    与 _stream_plan_events 的捕获模式一致（app/errors.py：code=service）；
    done 必为最后一行的承诺在 error 终态同样成立。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", False)

    async def _boom(*args, **kwargs):
        raise AgentServiceError("模型响应超时或网络异常，请稍后重试")

    # nodes 经模块级导入调用 invoke_structured，patch 模块命名空间即生效；
    # 同时钉死 has_llm_config 为 True，避免无 Key 机器上在 invoke 之前就被
    # _NO_KEY_HINT 短路——保证断言目标始终是被 patched 的 invoke 失败翻译
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)
    monkeypatch.setattr(nodes, "invoke_structured", _boom)

    events = await _post_stream(
        "/api/regenerate/stream",
        {
            "request": VALID_REQUEST,
            "outline": build_mock_outline(VALID_REQUEST),
            "phaseIndex": 0,
        },
    )

    assert [event["type"] for event in events] == ["done"]
    response = events[0]["response"]
    assert response["status"] == "error"
    assert response["error"]["code"] == "service"
    assert response["error"]["message"], "error 包络必须携带异常信息"


@pytest.mark.asyncio
async def test_regen_out_of_range(monkeypatch):
    """越界 phaseIndex / chapterIndex：不抛 4xx，走 done(status=error) 包络。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)

    # phaseIndex 越界：整流恰为一条 done 事件
    events = await _post_stream(
        "/api/regenerate/stream",
        {"request": VALID_REQUEST, "outline": outline, "phaseIndex": 99},
    )
    assert [event["type"] for event in events] == ["done"]
    response = events[0]["response"]
    assert response["status"] == "error"
    assert response["error"]["code"] == "validation"
    assert response["error"]["message"], "error 包络必须携带说明文本"

    # chapterIndex 越界：同样走 done(error) 包络
    events = await _post_stream(
        "/api/regenerate/stream",
        {
            "request": VALID_REQUEST,
            "outline": outline,
            "phaseIndex": 0,
            "chapterIndex": 99,
        },
    )
    assert [event["type"] for event in events] == ["done"]
    response = events[0]["response"]
    assert response["status"] == "error"
    assert response["error"]["code"] == "validation"
