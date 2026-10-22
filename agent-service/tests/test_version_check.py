"""版本探测与检索降级路径的单测：命中、丢弃、全失败、无 provider 不变。"""

from __future__ import annotations

from typing import Self

import pytest
from app import search, version_check
from app.version_check import detect_versions


class _Resp:
    def __init__(self, status_code: int, data: dict | None = None) -> None:
        self.status_code = status_code
        self._data = data or {}

    def json(self) -> dict:
        return self._data


class _FakeClient:
    """按 url 查表返回响应；表里没有的 url 一律 404（模拟"查不到"）。"""

    def __init__(self, table: dict[str, _Resp]) -> None:
        self.table = table
        self.calls: list[str] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc: object) -> bool:
        return False

    async def get(self, url: str) -> _Resp:
        self.calls.append(url)
        return self.table.get(url, _Resp(404))


def _patch(monkeypatch: pytest.MonkeyPatch, client: _FakeClient) -> None:
    monkeypatch.setattr(version_check.httpx, "AsyncClient", lambda **_: client)


@pytest.mark.asyncio
async def test_detect_versions_reads_each_ecosystem(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """四个生态各取对应字段的版本号。"""
    client = _FakeClient(
        {
            "https://crates.io/api/v1/crates/serde": _Resp(
                200, {"crate": {"max_stable_version": "1.0.215"}}
            ),
            "https://registry.npmjs.org/vite/latest": _Resp(200, {"version": "6.0.3"}),
            "https://pypi.org/pypi/fastapi/json": _Resp(
                200, {"info": {"version": "0.115.6"}}
            ),
            "https://api.github.com/repos/tokio-rs/tokio/releases/latest": _Resp(
                200, {"tag_name": "v1.42.0"}
            ),
        }
    )
    _patch(monkeypatch, client)

    facts = await detect_versions(
        [
            {"name": "serde", "ecosystem": "crate"},
            {"name": "vite", "ecosystem": "npm"},
            {"name": "fastapi", "ecosystem": "pypi"},
            {"name": "tokio-rs/tokio", "ecosystem": "github"},
        ]
    )

    assert [f["latestVersion"] for f in facts] == [
        "1.0.215",
        "6.0.3",
        "0.115.6",
        "1.42.0",
    ]


@pytest.mark.asyncio
async def test_unknown_packages_are_dropped(monkeypatch: pytest.MonkeyPatch) -> None:
    """查不到的库一律丢弃——这就是幻觉过滤器，编造的库名没有版本。"""
    client = _FakeClient(
        {"https://crates.io/api/v1/crates/serde": _Resp(
            200, {"crate": {"max_stable_version": "1.0.215"}}
        )}
    )
    _patch(monkeypatch, client)

    facts = await detect_versions(
        [
            {"name": "serde", "ecosystem": "crate"},
            {"name": "totally-made-up-crate", "ecosystem": "crate"},
        ]
    )

    assert [f["name"] for f in facts] == ["serde"]


@pytest.mark.asyncio
async def test_unsupported_ecosystem_is_ignored(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """未知生态不参与探测，也不发请求。"""
    client = _FakeClient({})
    _patch(monkeypatch, client)

    facts = await detect_versions([{"name": "x", "ecosystem": "maven"}])

    assert facts == []
    assert client.calls == []


@pytest.mark.asyncio
async def test_empty_input_sends_no_request(monkeypatch: pytest.MonkeyPatch) -> None:
    client = _FakeClient({})
    _patch(monkeypatch, client)

    assert await detect_versions([]) == []
    assert await detect_versions([{"name": "", "ecosystem": "crate"}]) == []
    assert client.calls == []


@pytest.mark.asyncio
async def test_all_failures_degrade_silently(monkeypatch: pytest.MonkeyPatch) -> None:
    """全失败只返回空列表，不抛异常（生成链路照常继续）。"""
    _patch(monkeypatch, _FakeClient({}))
    assert await detect_versions([{"name": "a", "ecosystem": "npm"}]) == []


def test_search_provider_absent_by_default(monkeypatch: pytest.MonkeyPatch) -> None:
    """未配置 / 配置了但未接入实现 → 一律 None，生成行为毫无变化。"""
    from app.config import settings

    monkeypatch.setattr(settings, "search_provider", "")
    assert search.current_provider() is None

    monkeypatch.setattr(settings, "search_provider", "tavily")
    assert search.current_provider() is None


@pytest.mark.asyncio
async def test_research_snippets_empty_without_provider(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """没有 provider 时知识点检索返回空，不发起任何网络调用。"""
    from app import nodes

    monkeypatch.setattr(search, "current_provider", lambda: None)
    assert await nodes._research_snippets("事件循环 宏任务") == []
