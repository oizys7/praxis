"""POST /api/llm-check：生成前的 LLM 连通性探测。

动机：用户点击「生成计划」时，Key 无效 / 网络不通 / 模型名写错应当在
进入生成逻辑之前就变成一句可读提示，而不是等生成流跑到第一次模型调用
才失败（那时用户已等待十几秒，且 generating 行已经建好）。
"""

import httpx
import pytest
from httpx import ASGITransport

from app import llm
from app.config import settings
from app.main import app


async def _post() -> dict:
    transport = ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post("/api/llm-check")
    assert response.status_code == 200
    return response.json()


class _OkModel:
    def bind(self, **_kwargs):
        return self

    async def ainvoke(self, _message):
        return "pong"


class _BoomModel:
    def bind(self, **_kwargs):
        return self

    async def ainvoke(self, _message):
        raise RuntimeError("Error code: 401 - invalid api key")


@pytest.mark.asyncio
async def test_mock_mode_reports_ok(monkeypatch):
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    data = await _post()
    assert data["ok"] is True
    assert "离线演示" in data["message"]


@pytest.mark.asyncio
async def test_missing_key_reports_config(monkeypatch):
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(llm, "has_llm_config", lambda: False)
    data = await _post()
    assert data["ok"] is False
    assert "API Key" in data["message"]


@pytest.mark.asyncio
async def test_probe_reports_success(monkeypatch):
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(llm, "has_llm_config", lambda: True)
    monkeypatch.setattr(llm, "get_chat_model", lambda: _OkModel())
    data = await _post()
    assert data["ok"] is True
    assert data["provider"] == settings.llm_provider


@pytest.mark.asyncio
async def test_probe_reports_connection_failure(monkeypatch):
    """鉴权失败/网络不通：ok=false 且原因可读，不抛异常到 HTTP 层。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(llm, "has_llm_config", lambda: True)
    monkeypatch.setattr(llm, "get_chat_model", lambda: _BoomModel())
    data = await _post()
    assert data["ok"] is False
    assert "401" in data["message"]
