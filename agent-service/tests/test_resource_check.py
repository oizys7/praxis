"""资源可达性检查的单测：两档判定、fail-open、空地址不发请求。"""

from __future__ import annotations

import pytest

from app import resource_check
from app.resource_check import REACHABLE, UNVERIFIED, verify_resources


class _FakeResponse:
    def __init__(self, status_code: int) -> None:
        self.status_code = status_code


class _FakeClient:
    """替身：按 url 查表返回状态码，可指定抛异常的 url；记录被调用的地址。"""

    def __init__(self, status_by_url: dict[str, int], *, boom: set[str] | None = None):
        self.status_by_url = status_by_url
        self.boom = boom or set()
        self.head_calls: list[str] = []
        self.get_calls: list[str] = []

    async def __aenter__(self) -> "_FakeClient":
        return self

    async def __aexit__(self, *exc: object) -> bool:
        return False

    async def head(self, url: str) -> _FakeResponse:
        self.head_calls.append(url)
        if url in self.boom:
            raise RuntimeError(f"connect failed: {url}")
        return _FakeResponse(self.status_by_url.get(url, 200))

    async def get(self, url: str) -> _FakeResponse:
        # 真站点常见形态：HEAD 被拒（405）但 GET 正常，故退回 GET 一律 200
        self.get_calls.append(url)
        return _FakeResponse(200)


def _patch(monkeypatch: pytest.MonkeyPatch, client: _FakeClient) -> None:
    monkeypatch.setattr(resource_check.httpx, "AsyncClient", lambda **_: client)


@pytest.mark.asyncio
async def test_reachable_urls_marked_reachable(monkeypatch: pytest.MonkeyPatch) -> None:
    """2xx/3xx 一律记 reachable。"""
    resources = [
        {"title": "A", "url": "https://ok.example/a"},
        {"title": "B", "url": "https://ok.example/b"},
    ]
    client = _FakeClient({})
    _patch(monkeypatch, client)

    out = await verify_resources(resources)

    assert [r["verification"] for r in out] == [REACHABLE, REACHABLE]
    assert client.head_calls == ["https://ok.example/a", "https://ok.example/b"]


@pytest.mark.asyncio
async def test_broken_and_missing_urls_stay_unverified(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """404 与连接异常都退回 unverified，且不抛出——检查失败绝不能中断生成。"""
    resources = [
        {"title": "404", "url": "https://ok.example/gone"},
        {"title": "boom", "url": "https://ok.example/boom"},
        {"title": "no-url", "url": ""},
    ]
    client = _FakeClient({"https://ok.example/gone": 404}, boom={"https://ok.example/boom"})
    _patch(monkeypatch, client)

    out = await verify_resources(resources)

    assert [r["verification"] for r in out] == [UNVERIFIED, UNVERIFIED, UNVERIFIED]


@pytest.mark.asyncio
async def test_empty_url_short_circuits_without_request(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """全部为空地址时压根不建连接：少一次网络依赖就少一个失败面。"""
    client = _FakeClient({})
    _patch(monkeypatch, client)

    out = await verify_resources([{"title": "无地址", "url": ""}, {"title": "缺字段"}])

    assert [r["verification"] for r in out] == [UNVERIFIED, UNVERIFIED]
    assert client.head_calls == []


@pytest.mark.asyncio
async def test_head_unsupported_falls_back_to_get(monkeypatch: pytest.MonkeyPatch) -> None:
    """405/501 退回一次 GET：有些站点不支持 HEAD，不能因此误判不可达。"""
    resources = [{"title": "A", "url": "https://ok.example/headless"}]
    client = _FakeClient({"https://ok.example/headless": 405})
    _patch(monkeypatch, client)

    out = await verify_resources(resources)

    assert out[0]["verification"] == REACHABLE
    assert client.get_calls == ["https://ok.example/headless"]


@pytest.mark.asyncio
async def test_input_items_are_not_mutated(monkeypatch: pytest.MonkeyPatch) -> None:
    """返回新列表，不改调用方传入的 dict（状态合并时不希望有隐藏别名）。"""
    original = [{"title": "A", "url": "https://ok.example/a"}]
    _patch(monkeypatch, _FakeClient({}))

    out = await verify_resources(original)

    assert "verification" not in original[0]
    assert out[0]["verification"] == REACHABLE
