"""生成任务化 API 测试：提交即返、事件日志回放/直播、取消、429 与 TTL 驱逐。

用例覆盖任务简报清单：
- POST /api/generations 立即返回 taskId（不等生成完成）；
- plan 任务：[outline, phase×N, done] / 短目标澄清 [done(clarification)]；
- regen_phase → [phase, done]；regen_chapter → [chapter, done]；
- WS 回放：cursor=0 全量 == 事件日志；cursor=中途 → seq 连续后缀；
- WS 直播：提交未完任务连 WS，逐事件收到直至 done；
- cancel 幂等（done(error, cancelled) 落日志，状态 cancelled）；
- MAX_CONCURRENT 占满 → 429；TTL=0 驱逐 → snapshot 404；快照字段完备；
- 意外异常 / 业务失败 → done(error, code=service)（done 必为日志最后一项）。
"""

import asyncio
import json
import time
from collections import deque
from typing import Any

import app.graph as graph_module
import httpx
import pytest
from app import nodes, tasks
from app.config import settings
from app.errors import AgentServiceError
from app.main import app
from app.mock import build_mock_outline
from app.tasks import TaskKind, TaskStatus, registry
from fastapi.testclient import TestClient

# 合法请求：goal 足够长、background 非空——避开规则澄清（与 test_stream.py 同源）
VALID_REQUEST = {
    "goal": "学习 Rust 与系统编程",
    "level": "beginner",
    "weeklyHours": 10,
    "weeks": 12,
    "interests": ["系统编程"],
    "background": "写过一些 C++",
}


@pytest.fixture(autouse=True)
def _clean_registry():
    """每例前后清空 registry；顺带取消在前例循环里遗留的 running 任务（防止污染并发池）。"""
    for entry in list(registry.tasks.values()):
        try:
            if entry.status == "running" and entry.task and not entry.task.done():
                entry.task.cancel()
        except RuntimeError:  # pragma: no cover - 外来 loop 已关闭时 cancel 的调度失败
            pass
    registry.tasks.clear()
    yield
    registry.tasks.clear()


async def _drain_until_terminal(task_id: str) -> list[dict[str, Any]]:
    """等待任务终态，返回事件日志（seq 升序的纯 event dict）。

    轮询等待属测试同步手段；终态凭据 = 日志末条 type==done（NDJSON 同构协议）。
    """
    for _ in range(600):  # 上限 30s 轮询兜底
        entry = registry.get(task_id)
        if entry and entry.status != "running":
            return [event for _, event in entry.events]
        await asyncio.sleep(0.05)
    raise AssertionError(f"任务 {task_id} 30s 未达终态")


def _post_generation(client: TestClient, payload: dict[str, Any]) -> str:
    """POST /api/generations（同步 TestClient），断言响应唯一字段 taskId。"""
    response = client.post("/api/generations", json=payload)
    assert response.status_code == 200, response.text
    body = response.json()
    assert list(body.keys()) == ["taskId"], body
    return body["taskId"]


def _drain_until_done(ws) -> list[dict[str, Any]]:
    """逐条读 WS 直到 done 终态：返回 [{"seq": n, "event": {...}}] 信封列表。

    WebSocketTestSession.receive 是阻塞调用；本端点保证每个连接最终有
    消息可读（回放或直播），且 done 必出现。
    """
    messages: list[dict[str, Any]] = []
    while True:
        envelope = json.loads(ws.receive_text())
        assert set(envelope) == {"seq", "event"}, f"WS 信封形状非法：{envelope}"
        messages.append(envelope)
        if envelope["event"]["type"] == "done":
            return messages


# ---------------------------------------------------------------------------
# 提交与事件日志
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_submit_returns_immediately(monkeypatch):
    """提交即返：POST 立即得 taskId（不等生成完成）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        started = time.perf_counter()
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        elapsed = time.perf_counter() - started
    assert response.status_code == 200, response.text
    task_id = response.json()["taskId"]
    assert registry.get(task_id) is not None, "任务必须在 registry 中可寻址"
    assert elapsed < 1.5, f"提交应立即返回，实测 {elapsed:.2f}s（不该等生成）"


@pytest.mark.asyncio
async def test_generation_payload_invalid_422(monkeypatch):
    """GenerationSubmit：kind + planRequest|regenRequest 二选一，显式 camelCase 校验。"""
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        # 非法 kind → 422
        bad_kind = await client.post(
            "/api/generations", json={"kind": "bogus", "planRequest": VALID_REQUEST}
        )
        assert bad_kind.status_code == 422, bad_kind.text
        # 缺 payload → 422
        missing = await client.post("/api/generations", json={"kind": "plan"})
        assert missing.status_code == 422, missing.text
        # kind=plan 却带 regenRequest → 422
        both = await client.post(
            "/api/generations",
            json={"kind": "plan", "planRequest": VALID_REQUEST, "regenRequest": {}},
        )
        assert both.status_code == 422, both.text
        # regen_phase 缺 regenRequest → 422
        wrong_field = await client.post(
            "/api/generations",
            json={"kind": "regen_phase", "planRequest": VALID_REQUEST},
        )
        assert wrong_field.status_code == 422, wrong_field.text


@pytest.mark.asyncio
async def test_plan_event_sequence(monkeypatch):
    """plan 任务 finished 后：[outline, chapter×M, phase×N, done] 且 done.status=plan；
    mermaid / blob 等键集断言直接引用 mock 产物（不硬编形状）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    types = [e["type"] for e in events]
    # 序列形状：outline → chapter×M（每章一条，可乱序）→ phase×N → done 必为末尾
    outline = events[0]
    chapter_events = [e for e in events if e["type"] == "chapter"]
    phase_events = [e for e in events if e["type"] == "phase"]
    assert types == (
        ["outline"]
        + ["chapter"] * len(chapter_events)
        + ["phase"] * len(phase_events)
        + ["done"]
    ), types
    assert outline["phaseCount"] >= 2
    assert outline["title"], "outline 事件必须携带计划标题"
    expected_chapters = sum(len(p["chapterTitles"]) for p in outline["phases"])
    assert len(chapter_events) == expected_chapters, "每章恰好一条 chapter 事件"
    assert len(phase_events) == outline["phaseCount"], "每阶段恰好一条 phase 事件"
    for e in phase_events:
        assert 0 <= e["index"] < outline["phaseCount"], "phase index 落在大纲范围内"
        assert e["phase"]["chapters"], "每阶段章节列表必须非空"
    done = events[-1]
    assert done["type"] == "done"
    assert done["response"]["status"] == "plan", "终态必须携带 plan 包络"
    assert done["response"]["plan"], "plan 终态必须携带计划体"

    # 键集断言：chapter 与 WS 重生成单章链同构；phase 与重组语义同构。
    # outline 额外携带完整大纲（outline 键）：Rust 存 plans.outline_json，
    # 重生成据此还原章型 / 知识点 / 版本事实，缺了它 regen 上下文会退化。
    assert set(outline) == {
        "type",
        "phaseCount",
        "title",
        "overview",
        "phases",
        "outline",
    }
    full_phase = outline["outline"]["phases"][0]
    assert full_phase["chapterKinds"], "完整大纲必须携带章型"
    assert full_phase["chapterKnowledgePoints"], "完整大纲必须携带知识点"
    # techFacts 由真实路径的版本探测注入（mock 不联网、不探测，故此处不产生）；
    # 读取方一律 `.get("techFacts") or []`，两种形态都能工作。
    for e in chapter_events:
        assert set(e) == {
            "type",
            "index",
            "phaseIndex",
            "chapter",
            # 生成输入留档：素材（重生成复用）+ 提示词变量快照（可复原提示词）
            "researchSnippets",
            "promptTrace",
        }
        chapter = e["chapter"]
        assert set(chapter) == {"title", "summary", "contentMd", "kind", "keyTerms"}
        assert "```mermaid" in chapter["contentMd"], "正文必须内联 mermaid 码块"
        assert (
            chapter["title"] in outline["phases"][e["phaseIndex"]]["chapterTitles"]
        ), "章题与骨架一致"
    for e in phase_events:
        assert set(e) == {"type", "index", "phase"}


@pytest.mark.asyncio
async def test_plan_clarification(monkeypatch):
    """短目标 → 单 [done] clarification（无 outline/phase 事件）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations",
            json={"kind": "plan", "planRequest": {**VALID_REQUEST, "goal": "ab"}},
        )
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    assert [e["type"] for e in events] == ["done"], events
    assert events[-1]["response"]["status"] == "clarification"
    assert events[-1]["response"]["clarifications"], "必须携带澄清问题"


@pytest.mark.asyncio
async def test_regen_phase_task(monkeypatch):
    """regen_phase kind：事件序列 [phase, done]，phase index 与请求一致。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations",
            json={
                "kind": "regen_phase",
                "regenRequest": {
                    "request": VALID_REQUEST,
                    "outline": outline,
                    "phaseIndex": 0,
                },
            },
        )
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    # 串行逐章：chapterStart/chapter 交替 ×N + phase 收尾（轻装补齐）
    types = [e["type"] for e in events]
    body = types[:-2]
    assert types[0] == "chapterStart" and types[-2] == "phase" and types[-1] == "done"
    # 章级并行后 start 可能集中在前面（不再严格交替）：只要求两类事件等量
    assert set(body) == {"chapterStart", "chapter"}, f"事件类型异常：{body}"
    assert events[-2]["index"] == 0  # phase 收尾事件（events[0] 是 chapterStart）
    assert events[-2]["phase"]["chapters"], "重生成的阶段必须携带章节正文"
    assert events[-1]["response"]["status"] == "plan"


@pytest.mark.asyncio
async def test_regen_chapter_task(monkeypatch):
    """regen_chapter kind：事件序列 [chapter, done]，chapter 键集 camelCase。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations",
            json={
                "kind": "regen_chapter",
                "regenRequest": {
                    "request": VALID_REQUEST,
                    "outline": outline,
                    "phaseIndex": 1,
                    "chapterIndex": 1,
                },
            },
        )
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    # 2026-10-16 逐章动画：chapterStart 先行（点击即转圈）
    # phase 由 assemble 统一产出（单章模式 Rust 忽略），事件流与全阶段同构
    assert [e["type"] for e in events] == ["chapterStart", "chapter", "phase", "done"]
    chapter_event = events[1]
    assert chapter_event["index"] == 1 and chapter_event["phaseIndex"] == 1
    assert set(chapter_event["chapter"]) == {"title", "summary", "contentMd", "kind", "keyTerms"}
    assert events[-1]["response"]["status"] == "plan"


@pytest.mark.asyncio
async def test_snapshot_fields(monkeypatch):
    """快照字段完备：taskId/kind/status/firstSeq/lastSeq/done；不存在 404。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        task_id = response.json()["taskId"]
        for _ in range(600):
            snap = await client.get(f"/api/generations/{task_id}/snapshot")
            if snap.status_code == 200 and snap.json()["done"]:
                break
            await asyncio.sleep(0.05)
        else:
            pytest.fail("任务未在 30s 内完成")
        snapshot = snap.json()
        assert set(snapshot) == {
            "taskId",
            "kind",
            "status",
            "firstSeq",
            "lastSeq",
            "done",
        }
        assert snapshot["taskId"] == task_id
        assert snapshot["kind"] == "plan"
        assert snapshot["status"] == "finished"
        assert snapshot["firstSeq"] == 1
        assert snapshot["lastSeq"] >= 3
        assert snapshot["done"] is True
        # 不存在的任务 → 404
        missing = await client.get("/api/generations/nonexistent/snapshot")
        assert missing.status_code == 404


# ---------------------------------------------------------------------------
# WS 通道：回放 + 直播
# ---------------------------------------------------------------------------


async def _wait_finished_plan() -> str:
    from app.schemas import PlanRequest

    task_id = await registry.submit("plan", PlanRequest.model_validate(VALID_REQUEST))
    await _drain_until_terminal(task_id)
    return task_id


def _fetch_log(task_id: str) -> list[dict[str, Any]]:
    """读出注册表日志（回放对照基准），终态已由 _wait_finished_plan 保证。"""
    entry = registry.get(task_id)
    assert entry is not None and entry.status != "running"
    return [event for _, event in entry.events]


class TestWsReplay:
    """WS 回放通道（FastAPI TestClient websocket_connect，同步上下文）。"""

    def test_ws_replay_full(self, monkeypatch):
        """finished 任务 cursor=0 全量回放 == 事件日志，seq 从 1 连续自增。"""
        monkeypatch.setattr(settings, "allow_mock_llm", True)
        task_id = asyncio.run(_wait_finished_plan())
        events = _fetch_log(task_id)

        with (
            TestClient(app) as client,
            client.websocket_connect(
                f"/api/generations/{task_id}/events?cursor=0"
            ) as ws,
        ):
            replay = _drain_until_done(ws)

        assert len(replay) == len(events), "回放必须不丢不重，与日志一比一对齐"
        for seq, event in enumerate(events, start=1):
            assert replay[seq - 1]["seq"] == seq
            assert replay[seq - 1]["event"] == event

    def test_ws_replay_suffix(self, monkeypatch):
        """cursor=中途 → 仅取后缀且 seq 连续（回放缓存下界过滤）。"""
        monkeypatch.setattr(settings, "allow_mock_llm", True)
        task_id = asyncio.run(_wait_finished_plan())
        events = _fetch_log(task_id)

        with (
            TestClient(app) as client,
            client.websocket_connect(
                f"/api/generations/{task_id}/events?cursor=1"
            ) as ws,
        ):
            replay = _drain_until_done(ws)

        assert len(replay) == len(events) - 1, "cursor=1 应跳过 seq=1"
        for i, message in enumerate(replay):
            assert message["seq"] == 2 + i, "后缀 seq 必须连续"
        assert [m["event"] for m in replay] == events[1:], (
            "后缀必须逐事件与日志 seq>=2 完全一致"
        )

    def test_ws_missing_task_4404(self):
        """任务不存在 → WS 关闭 code=4404。"""
        with (
            TestClient(app) as client,
            pytest.raises(Exception) as exc_info,
            client.websocket_connect(
                "/api/generations/nonexistent/events?cursor=0"
            ) as ws,
        ):
            ws.receive_text()

        code = getattr(exc_info.value, "code", None)
        assert code == 4404, f"应在关闭帧携带 4404，实际异常：{exc_info.value!r}"


class TestWsLive:
    """WS 直播通道：提交未完任务立即连 WS，逐事件到达直至 done。"""

    def test_ws_live_until_done(self, monkeypatch):
        """真实直播：提交未完任务 → 立即连接 WS 收到真实事件直至 done。"""
        monkeypatch.setattr(settings, "allow_mock_llm", True)

        # 提交走 TestClient：任务在 TestClient 的 portal loop 内跑，与 WS 同循环；
        # 提交即返，随后立即连 WS（回放 + 直播衔接由 per-task lock 保证不丢不重）。
        with TestClient(app) as client:
            response = client.post(
                "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
            )
            assert response.status_code == 200, response.text
            task_id = response.json()["taskId"]
            with client.websocket_connect(
                f"/api/generations/{task_id}/events?cursor=0"
            ) as ws:
                messages = _drain_until_done(ws)

        seqs = [m["seq"] for m in messages]
        assert len(seqs) == len(set(seqs)), "seq 不得重复（不丢不重）"
        assert seqs == sorted(seqs), "seq 必须单调递增"
        assert messages[-1]["event"]["type"] == "done"
        # 事件类型合法性（合法全集）
        assertion_types = {m["event"]["type"] for m in messages}
        assert assertion_types <= {"outline", "phase", "chapter", "done"}
        assert messages[0]["event"]["type"] in {"outline", "done"}


@pytest.mark.asyncio
async def test_cancel_idempotent(monkeypatch):
    """DELETE cancel：幂等（不存在 404；已终态返回 200）；done(error, cancelled)。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        # 不存在的任务 → 404
        assert (await client.delete("/api/generations/nonexistent")).status_code == 404

        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        task_id = response.json()["taskId"]
        first = await client.delete(f"/api/generations/{task_id}")
        second = await client.delete(f"/api/generations/{task_id}")
        assert first.status_code == 200, first.text
        assert second.status_code == 200, second.text
        assert second.json() == first.json(), "重复 cancel 必须幂等"

    events = await _drain_until_terminal(task_id)
    done = events[-1]
    assert done["type"] == "done"
    assert done["response"]["status"] == "error"
    assert done["response"]["error"]["code"] == "cancelled", done
    entry = registry.get(task_id)
    assert entry is not None, "幂等 cancel 后任务应仍在缓存"
    assert entry.status == "cancelled"


@pytest.mark.asyncio
async def test_concurrency_limit_429(monkeypatch):
    """monkeypatch MAX_CONCURRENT=1 占满 → 第二个提交 429（中文 message）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    monkeypatch.setattr(tasks, "MAX_CONCURRENT", 1)

    started: list[int] = []
    gate = asyncio.Event()

    class _SlowGraph:
        """模拟长任务：哨兵事件挂起，等待测试放行。"""

        def astream(self, *args, **kwargs):
            async def _gen():
                started.append(1)
                yield "values", {}
                await gate.wait()

            return _gen()

    monkeypatch.setattr(graph_module, "agent_graph", _SlowGraph())

    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        first = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        assert first.status_code == 200, first.text
        for _ in range(100):
            if started:
                break
            await asyncio.sleep(0.02)
        assert started, "第一个任务必须实际进入运行"

        full = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        assert full.status_code == 429, full.text
        assert "并发" in full.text or "稍后" in full.text
        gate.set()


@pytest.mark.asyncio
async def test_registry_capacity_429(monkeypatch):
    """registry 容量满（sweep 后仍超）→ 429；上限以常量呈现，可 monkeypatch。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    monkeypatch.setattr(tasks, "MAX_REGISTRY_SIZE", 2)
    now = time.time()
    for i in range(2):
        registry.tasks[f"stale-{i}"] = tasks.TaskEntry(
            task_id=f"stale-{i}",
            kind=TaskKind.plan,
            events=deque(),
            status=TaskStatus.finished,
            created_at=now,
            finished_at=now,
            task=None,
        )
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
    assert response.status_code == 429, response.text
    assert "稍后" in response.text or "满" in response.text


@pytest.mark.asyncio
async def test_ttl_zero_eviction(monkeypatch):
    """TTL=0 → snapshot 404（finished 任务立即驱逐）；TTL 以常量呈现可 monkeypatch。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    monkeypatch.setattr(tasks, "TASK_TTL_SECONDS", 0)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        task_id = response.json()["taskId"]
        for _ in range(600):
            snap = await client.get(f"/api/generations/{task_id}/snapshot")
            if snap.status_code == 404:
                break
            await asyncio.sleep(0.05)
        else:
            pytest.fail("TTL=0 的 finished 任务应在终态后立即驱逐（404）")


@pytest.mark.asyncio
async def test_business_error_becomes_done_error(monkeypatch):
    """计划任务里 LLM 配置缺失等业务失败 → 正常 done(error) 事件（非 4xx）。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)

    async def _boom(*args, **kwargs):
        raise AgentServiceError(
            "模型响应超时或网络异常，请稍后重试，或在设置页更换模型"
        )

    monkeypatch.setattr(nodes, "invoke_structured", _boom)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        assert response.status_code == 200, response.text
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    assert events[-1]["type"] == "done"
    assert events[-1]["response"]["status"] == "error"
    assert events[-1]["response"]["error"]["code"] == "service"
    assert events[-1]["response"]["error"]["message"]


@pytest.mark.asyncio
async def test_unexpected_exception_becomes_service_done(monkeypatch):
    """意外异常必须翻译为 done(error, code=service)——done 必为事件日志最后一项。"""
    monkeypatch.setattr(settings, "allow_mock_llm", False)

    async def _boom(*args, **kwargs):
        raise RuntimeError("Unexpected internal glitch: unicorn timezone confusion")

    monkeypatch.setattr(nodes, "invoke_structured", _boom)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: True)

    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        response = await client.post(
            "/api/generations", json={"kind": "plan", "planRequest": VALID_REQUEST}
        )
        assert response.status_code == 200, response.text
        task_id = response.json()["taskId"]

    events = await _drain_until_terminal(task_id)
    assert events[-1]["type"] == "done"
    assert events[-1]["response"]["status"] == "error"
    assert events[-1]["response"]["error"]["code"] == "service"
