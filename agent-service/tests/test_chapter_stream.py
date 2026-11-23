"""研读流式链路测试（2026-11 功能包1）。

- _JsonStringFieldStreamExtractor：工具参数 JSON 片段流的增量解码（纯函数级）；
- invoke_chapter_streamed：假模型注入的端到端（片段 → on_delta 增量 + 结构化返回）；
- /api/chat：mock 流式（chatDelta×N + chatDone）与未配置 Key 的 chatError 终态。
"""

import json
from types import SimpleNamespace
from typing import Any

import httpx
import pytest
from app.config import settings
from app.llm import (
    _JsonStringFieldStreamExtractor,
    invoke_chapter_streamed,
)
from app.main import app
from app.schemas import ChapterDraft


# ---------------------------------------------------------------------------
# 增量提取器（纯函数级）
# ---------------------------------------------------------------------------


def _feed_all(text: str, *, chunk_size: int = 3) -> list[str]:
    """把整段 JSON 按固定小片段喂给提取器，收集全部解码增量。"""
    out: list[str] = []
    extractor = _JsonStringFieldStreamExtractor("contentMd", out.append)
    for index in range(0, len(text), chunk_size):
        extractor.feed(text[index : index + chunk_size])
    return out


def test_extractor_streams_decoded_content_md() -> None:
    """常规 JSON：contentMd 值逐片段解码为增量，拼接等于原文；其他字段不外漏。"""
    payload = {
        "title": "所有权与借用",
        "summary": "概述",
        "contentMd": "## 所有权\n\n正文第一段。\n\n```mermaid\nflowchart TD\n```",
        "kind": "concept",
    }
    deltas = _feed_all(json.dumps(payload, ensure_ascii=False), chunk_size=2)
    assert "".join(deltas) == payload["contentMd"]
    assert all("title" not in delta for delta in deltas)


def test_extractor_handles_escapes_and_unicode() -> None:
    """转义与 \\uXXXX（含代理对）正确解码；引号不提前终止字符串。"""
    raw = r'{"contentMd": "行一\n行二\t缩进 \"引号\" \\ 反斜甲 \u4e00\u8f6c unicode \ud83d\ude00 表情"}'
    deltas = _feed_all(raw, chunk_size=5)
    joined = "".join(deltas)
    assert "行一\n行二\t缩进 \"引号\" \\ 反斜甲 一转 unicode \U0001f600 表情" == joined


def test_extractor_missing_field_yields_nothing() -> None:
    """字段从未出现：不产出任何增量（整段兜底路径的前提）。"""
    deltas = _feed_all('{"title": "只有标题"}', chunk_size=2)
    assert deltas == []


def test_extractor_stops_at_closing_quote() -> None:
    """值字符串闭合后的内容（含二次出现的同名键样文本）不再外漏。"""
    raw = r'{"contentMd": "正文", "tail": "\"contentMd\": \"伪造\""}'
    deltas = _feed_all(raw, chunk_size=1)
    assert "".join(deltas) == "正文"


def test_extractor_split_key_across_fragments() -> None:
    """键名 "contentMd" 被片段切开仍可定位（跨片段上下文）。"""
    raw = '{"contentMd": "跨片段键"}'
    deltas = _feed_all(raw, chunk_size=1)
    assert "".join(deltas) == "跨片段键"


# ---------------------------------------------------------------------------
# invoke_chapter_streamed（假模型端到端）
# ---------------------------------------------------------------------------


class _FakeBound:
    """bind_tools 返回替身：astream 逐个吐带 tool_call_chunks 的 chunk。"""

    def __init__(self, fragments: list[str]) -> None:
        self._fragments = fragments

    async def astream(self, _messages: Any):  # noqa: ANN401 —— 测试替身
        for fragment in self._fragments:
            yield SimpleNamespace(tool_call_chunks=[{"args": fragment}])


class _FakeModel:
    def __init__(self, fragments: list[str]) -> None:
        self._fragments = fragments

    def bind_tools(self, _schemas: Any) -> _FakeBound:  # noqa: ANN401
        return _FakeBound(self._fragments)


@pytest.mark.asyncio
async def test_invoke_chapter_streamed_emits_deltas_and_parses(monkeypatch) -> None:
    """片段流 → on_delta 收到解码增量；最终返回完整 ChapterDraft。"""
    draft = ChapterDraft(
        title="研读流式章",
        summary="概述",
        content_md="## 正文\n\n" + "长" * 300,
        kind="concept",
    )
    raw = json.dumps(draft.model_dump(mode="json", by_alias=True), ensure_ascii=False)
    fragments = [raw[i : i + 9] for i in range(0, len(raw), 9)]
    monkeypatch.setattr(
        "app.llm.get_chat_model", lambda: _FakeModel(fragments)
    )

    deltas: list[str] = []

    def _collect(text: str) -> None:
        deltas.append(text)

    result = await invoke_chapter_streamed(
        ChapterDraft, "chapter", {"goal": "测试"}, on_delta=_collect
    )

    assert result.title == "研读流式章"
    assert result.content_md == draft.content_md
    assert "".join(deltas) == draft.content_md, "增量拼接 == 最终 contentMd"


@pytest.mark.asyncio
async def test_invoke_chapter_streamed_retries_with_feedback(monkeypatch) -> None:
    """第一轮空工具调用：走反馈重试链，第二轮成功且增量独立成流。"""
    good = ChapterDraft(title="重试成功", content_md="正文", kind="concept")
    good_raw = json.dumps(
        good.model_dump(mode="json", by_alias=True), ensure_ascii=False
    )
    rounds: list[list[str]] = [
        [],  # 第一轮：无任何 tool_call_chunks（空工具调用）
        [good_raw[i : i + 12] for i in range(0, len(good_raw), 12)],
    ]
    state = {"round": -1}

    class _Bound:
        async def astream(self, _messages: Any):  # noqa: ANN401
            state["round"] += 1
            fragments = rounds[min(state["round"], len(rounds) - 1)]
            for fragment in fragments:
                yield SimpleNamespace(tool_call_chunks=[{"args": fragment}])

    class _Model:
        def bind_tools(self, _schemas: Any) -> _Bound:  # noqa: ANN401
            return _Bound()

    monkeypatch.setattr("app.llm.get_chat_model", lambda: _Model())

    deltas: list[str] = []

    def _collect(text: str) -> None:
        deltas.append(text)

    result = await invoke_chapter_streamed(
        ChapterDraft, "chapter", {}, on_delta=_collect
    )

    assert result.title == "重试成功"
    assert "".join(deltas) == "正文", "重试轮的增量独立成流（无第一轮残留）"


# ---------------------------------------------------------------------------
# /api/chat（mock 流式 + 错误终态）
# ---------------------------------------------------------------------------


async def _post_chat(payload: dict[str, Any]) -> list[dict[str, Any]]:
    transport = httpx.ASGITransport(app=app)
    events: list[dict[str, Any]] = []
    async with (
        httpx.AsyncClient(transport=transport, base_url="http://test") as client,
        client.stream("POST", "/api/chat", json=payload) as response,
    ):
        assert response.status_code == 200
        async for line in response.aiter_lines():
            if line.strip():
                events.append(json.loads(line))
    return events


@pytest.mark.asyncio
async def test_chat_mock_stream_ends_with_done(monkeypatch) -> None:
    """mock 模式：chatDelta 多条 + chatDone 收尾，完整回答 == 增量拼接。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)

    events = await _post_chat(
        {
            "planId": "p-1",
            "phaseIndex": 0,
            "chapterIndex": 1,
            "planTitle": "Rust 入门",
            "planGoal": "三个月写出异步服务",
            "chapterTitle": "所有权与借用",
            "chapterContentMd": "## 所有权\n\n正文…",
            "messages": [{"role": "user", "content": "上一问"}],
            "question": "所有权和借用有什么区别？",
        }
    )

    types = [event["type"] for event in events]
    assert types[0] == "chatDelta" and types[-1] == "chatDone"
    assert set(types[:-1]) == {"chatDelta"}, "终态前全是增量事件"
    deltas = "".join(event["text"] for event in events[:-1])
    done = events[-1]
    assert done["message"]["role"] == "assistant"
    assert done["message"]["content"] == deltas, "chatDone 完整回答与增量拼接一致"


@pytest.mark.asyncio
async def test_chat_without_key_yields_chat_error(monkeypatch) -> None:
    """未配置 Key 且非 mock：chatError 业务终态（200 NDJSON，不抛 4xx）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr("app.main.has_llm_config", lambda: False)

    events = await _post_chat(
        {
            "question": "这一章讲什么？",
        }
    )

    assert [event["type"] for event in events] == ["chatError"]
    assert events[0]["code"] == "service"
    assert "API Key" in events[0]["message"]
