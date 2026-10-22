"""检索提供方的单测：配置分支、请求形状、结果映射与失败路径。"""

from __future__ import annotations

from typing import Self

import pytest

from app import search
from app.search import TavilyProvider


class _Resp:
    def __init__(self, status_code: int, data: dict | None = None) -> None:
        self.status_code = status_code
        self._data = data or {}

    def json(self) -> dict:
        return self._data


class _FakeClient:
    def __init__(self, response: _Resp) -> None:
        self.response = response
        self.calls: list[dict] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc: object) -> bool:
        return False

    async def post(self, url: str, *, headers: dict, json: dict) -> _Resp:
        self.calls.append({"url": url, "headers": headers, "json": json})
        return self.response


def _patch(monkeypatch: pytest.MonkeyPatch, client: _FakeClient) -> None:
    monkeypatch.setattr(search.httpx, "AsyncClient", lambda **_: client)


def _settings(monkeypatch: pytest.MonkeyPatch, provider: str, key: str) -> None:
    from app.config import settings

    monkeypatch.setattr(settings, "search_provider", provider)
    monkeypatch.setattr(settings, "search_api_key", key)


def test_provider_none_when_unconfigured(monkeypatch: pytest.MonkeyPatch) -> None:
    """留空 = 不启用，生成行为与没有本模块完全一致。"""
    _settings(monkeypatch, "", "")
    assert search.current_provider() is None


def test_provider_none_without_api_key(monkeypatch: pytest.MonkeyPatch) -> None:
    """选了 tavily 却没填 key：按未启用处理，而不是每章一次必然 401。"""
    _settings(monkeypatch, "tavily", "")
    assert search.current_provider() is None


def test_provider_none_for_unknown_name(monkeypatch: pytest.MonkeyPatch) -> None:
    _settings(monkeypatch, "brave", "whatever")
    assert search.current_provider() is None


def test_provider_built_when_configured(monkeypatch: pytest.MonkeyPatch) -> None:
    _settings(monkeypatch, "Tavily", "  tvly-test  ")
    provider = search.current_provider()
    assert isinstance(provider, TavilyProvider)


@pytest.mark.asyncio
async def test_search_request_shape_and_mapping(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """请求带 Bearer 认证、只用 basic 深度；结果映射成 {title,url,snippet}。"""
    client = _FakeClient(
        _Resp(
            200,
            {
                "results": [
                    {
                        "title": "Rust Book",
                        "url": "https://doc.rust-lang.org/book/",
                        "content": "Ownership is Rust's most unique feature.",
                        "score": 0.98,
                        "raw_content": "<html>…</html>",
                    }
                ]
            },
        )
    )
    _patch(monkeypatch, client)

    items = await TavilyProvider("tvly-test").search("rust ownership", limit=3)

    call = client.calls[0]
    assert call["url"] == "https://api.tavily.com/search"
    assert call["headers"]["Authorization"] == "Bearer tvly-test"
    assert call["json"] == {
        "query": "rust ownership",
        "max_results": 3,
        "search_depth": "basic",
    }
    assert items == [
        {
            "title": "Rust Book",
            "url": "https://doc.rust-lang.org/book/",
            "snippet": "Ownership is Rust's most unique feature.",
        }
    ]


@pytest.mark.asyncio
async def test_result_without_content_is_skipped(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """没有正文片段的条目对「注入素材」无用，跳过。"""
    client = _FakeClient(
        _Resp(200, {"results": [{"title": "空", "url": "https://a"}, {"title": "T", "url": "https://b", "content": "正文"}]})
    )
    _patch(monkeypatch, client)

    items = await TavilyProvider("k").search("q")

    assert [i["url"] for i in items] == ["https://b"]


@pytest.mark.asyncio
async def test_limit_is_clamped(monkeypatch: pytest.MonkeyPatch) -> None:
    client = _FakeClient(_Resp(200, {"results": []}))
    _patch(monkeypatch, client)

    await TavilyProvider("k").search("q", limit=999)
    assert client.calls[0]["json"]["max_results"] == 20


@pytest.mark.asyncio
async def test_http_error_raises(monkeypatch: pytest.MonkeyPatch) -> None:
    """非 2xx 上抛，由调用方记 warning 后继续生成（不是静默空结果）。"""
    _patch(monkeypatch, _FakeClient(_Resp(429)))

    with pytest.raises(RuntimeError, match="429"):
        await TavilyProvider("k").search("q")


@pytest.mark.asyncio
async def test_research_snippets_empty_without_provider(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from app import nodes

    monkeypatch.setattr(search, "current_provider", lambda: None)
    assert await nodes._research_snippets("事件循环 宏任务") == []


@pytest.mark.asyncio
async def test_research_snippets_degrades_on_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """检索抛异常只降级为空素材，章节照常生成。"""
    from app import nodes

    class _Boom:
        async def search(self, query: str, *, limit: int = 3) -> list[dict[str, str]]:
            raise RuntimeError("boom")

    monkeypatch.setattr(search, "current_provider", lambda: _Boom())
    assert await nodes._research_snippets("q") == []
