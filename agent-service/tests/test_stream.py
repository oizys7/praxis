"""main.py NDJSON 流式端点测试（mock 模式，httpx ASGITransport 不走网络）。

用例名与实现计划简报逐字对应（test_stream_event_sequence /
test_stream_clarification）。事件协议章级扇出式：outline → chapter×M
（可乱序）→ phase×N（阶段重组）→ done；done 必为最后一行，
clarification 终态同样只走 done（此时无 outline / phase / chapter 事件）。
"""

import json
from typing import Any

import httpx
import pytest

from app import nodes
from app.config import settings
from app.errors import AgentServiceError
from app.main import app

# 合法请求：goal 足够长、background 非空——避开规则澄清，直达分层生成
VALID_REQUEST = {
    "goal": "学习 Rust 与系统编程",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["系统编程"],
    "background": "写过一些 C++",
}


async def _post_stream(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """POST 流式端点并逐行解析 NDJSON 事件（每行一个 JSON 对象）。"""
    transport = httpx.ASGITransport(app=app)
    events: list[dict[str, Any]] = []
    async with (
        httpx.AsyncClient(transport=transport, base_url="http://test") as client,
        client.stream("POST", "/api/generate-plan/stream", json=payload) as response,
    ):
        assert response.status_code == 200
        async for line in response.aiter_lines():
            if line.strip():
                events.append(json.loads(line))
    return events


@pytest.mark.asyncio
async def test_stream_event_sequence(monkeypatch):
    """合法请求：outline → chapter×M → phase×N（重组含全章）→ done=plan。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    events = await _post_stream(VALID_REQUEST)

    assert events, "流必须至少产出一个事件"
    # 首条：outline 事件，阶段数与标题齐备
    first = events[0]
    assert first["type"] == "outline"
    assert first["phaseCount"] >= 2
    assert first["title"], "outline 事件必须携带计划标题"
    phase_count = first["phaseCount"]

    # 中段：chapter 事件（每章一条），与重生成链 chapter 事件同构
    chapter_events = [event for event in events if event["type"] == "chapter"]
    expected_chapters = sum(
        len(phase["chapterTitles"]) for phase in first["phases"]
    )
    assert len(chapter_events) == expected_chapters, "每章恰好一条 chapter 事件"
    for event in chapter_events:
        assert set(event) == {"type", "index", "phaseIndex", "chapter"}, (
            "chapter 事件键集与 WS 重生成单章链同构"
        )
        assert 0 <= event["phaseIndex"] < phase_count, "phaseIndex 落在大纲范围内"
        assert event["chapter"]["title"] in first["phases"][event["phaseIndex"]][
            "chapterTitles"
        ], "章题与骨架一致"
        assert len(event["chapter"]["contentMd"].strip()) >= 1000, "章正文过硬底线"
    type_seq = [event["type"] for event in events]
    assert type_seq.index("phase") > max(
        i for i, t in enumerate(type_seq) if t == "chapter"
    ), "chapter 事件应全部先于重组 phase 事件"

    # 重组段：phase 事件，index 落在大纲范围内，章节非空
    phase_events = [event for event in events if event["type"] == "phase"]
    assert len(phase_events) == phase_count, "每阶段恰好一条重组 phase 事件"
    for event in phase_events:
        assert 0 <= event["index"] < phase_count, "phase index 必须落在大纲范围内"
        assert event["phase"]["chapters"], "每个阶段的章节列表必须非空"

    # 末条：done 事件，包络为 plan 终态，且全流无其他事件类型
    done = events[-1]
    assert done["type"] == "done"
    assert done["response"]["status"] == "plan"
    assert done["response"]["plan"], "plan 终态必须携带计划体"
    assert {event["type"] for event in events} == {
        "outline",
        "chapter",
        "phase",
        "done",
    }


@pytest.mark.asyncio
async def test_stream_business_error_yields_error_done(monkeypatch):
    """业务异常（AgentServiceError）不中断流：恰为一行 error 包络的 done。

    与非流式通道全局 handler 语义对齐（app/errors.py：code=service，
    message 为异常文本）；done 必为最后一行的承诺在 error 终态同样成立。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", False)

    async def _boom(*args, **kwargs):
        raise AgentServiceError("模型响应超时或网络异常，请稍后重试，或在设置页更换模型")

    # outline_node 经 nodes 模块级导入调用 invoke_structured，patch 模块命名空间即生效
    monkeypatch.setattr(nodes, "invoke_structured", _boom)

    events = await _post_stream(VALID_REQUEST)

    assert [event["type"] for event in events] == ["done"], (
        "业务失败不得截断流，必须恰有一条 done 事件收尾"
    )
    response = events[0]["response"]
    assert response["status"] == "error"
    assert response["error"]["code"] == "service"
    assert response["error"]["message"], "error 包络必须携带异常信息"


@pytest.mark.asyncio
async def test_stream_clarification(monkeypatch):
    """短目标 "ab"（<4 字符）触发规则澄清：无 outline/phase 事件，done=clarification。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    events = await _post_stream({**VALID_REQUEST, "goal": "ab"})

    assert [event["type"] for event in events] == ["done"], (
        "澄清终态只应有一条 done 事件"
    )
    response = events[0]["response"]
    assert response["status"] == "clarification"
    assert response["clarifications"], "clarification 终态必须携带澄清问题"
