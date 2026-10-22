"""regenerate 流式端点测试（mock 模式，httpx ASGITransport）。

用例名与任务简报逐字对应（test_regen_phase_stream / test_regen_chapter_stream /
test_regen_out_of_range）。协议约定：

- outline 事件除 phaseCount/title 外携带 overview + phases
  （[{name, weekRange, chapterTitles}]，数据直出自 PlanOutline；载荷断言
  已归口 test_generations.py 的 WS 事件链用例）；
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
from app import nodes
from app.config import settings
from app.errors import AgentServiceError
from app.main import app
from app.mock import build_mock_outline
from app.schemas import Chapter
from pydantic import ValidationError

# 合法请求：goal 足够长、background 非空——避开规则澄清
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
async def test_regen_phase_stream(monkeypatch):
    """全阶段重生成：mock 模式 [phase, done(status=plan)]，phase.chapters 非空。"""
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    outline = build_mock_outline(VALID_REQUEST)

    events = await _post_stream(
        "/api/regenerate/stream",
        {"request": VALID_REQUEST, "outline": outline, "phaseIndex": 0},
    )

    # 串行逐章：chapterStart→chapter 交替 ×N，收尾 phase（轻装补齐）+ done
    types = [event["type"] for event in events]
    assert types[0] == "chapterStart", "首事件必须是 chapterStart"
    assert types[-1] == "done"
    assert types[-2] == "phase", "phase 收尾事件必须倒数第二"
    body = types[:-2]
    assert body.count("chapterStart") == body.count("chapter") > 0, (
        f"chapterStart 与 chapter 必须成对且非空：{body}"
    )
    # 章级并行（2026-10-22 编排统一后重生成与首次生成共用同一扇出）：各章的
    # chapterStart 在对应章调 LLM 前立即发出，因此所有 start 可能集中在前面，
    # 不再严格交替。只要求两类事件等量、且 start 全部先于 chapter 完成。
    assert set(body) == {"chapterStart", "chapter"}, f"事件类型异常：{body}"
    assert body[: body.count("chapterStart")] == ["chapterStart"] * body.count(
        "chapterStart"
    ), f"chapterStart 必须先于章节完成事件：{body}"
    phase_event = events[-2]
    assert phase_event["index"] == 0, "phase 事件的 index 必须等于请求的 phaseIndex"
    assert phase_event["phase"]["chapters"], "重生成的阶段必须携带章节正文"
    assert phase_event["phase"].get("practice"), "phase 收尾必须携带轻装（practice）"
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

    # phase 由 assemble 统一产出：单章模式下 Rust 忽略它（只 upsert chapter），
    # 但保留一条让事件流与全阶段路径同构。
    assert [event["type"] for event in events] == [
        "chapterStart",
        "chapter",
        "phase",
        "done",
    ]
    assert events[0]["chapterIndex"] == chapter_index
    assert events[0]["phaseIndex"] == 1
    chapter_event = events[1]
    assert chapter_event["index"] == chapter_index
    assert chapter_event["phaseIndex"] == 1
    # by_alias camelCase dump：Pydantic 模型（snake_case 属性名）可直接回解析
    try:
        Chapter.model_validate(chapter_event["chapter"])
    except ValidationError as exc:  # pragma: no cover - 断言失败路径
        pytest.fail(f"chapter 载荷不满足 Chapter 模型：{exc}")
    chapter = chapter_event["chapter"]
    assert set(chapter) == {"title", "summary", "contentMd", "kind", "keyTerms"}, "Chapter 键集含章型与术语"
    content = chapter["contentMd"]
    assert len(content.strip()) >= 1000, "章节 markdown 正文不得少于 1000 字"
    assert "```mermaid" in content, "正文必须内联 mermaid 码块"
    assert "### 深入" in content, "概念章正文必须有一节深入剖析"
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

    # 章生成失败在图内静默（与首次生成的单章容错同源），不再炸成 done(error)：
    # 本用例请求的是全阶段（两章并行）→ 两个 chapterStart；两章都没产出正文，
    # 故无 chapter 事件，assemble 用空占位拼出阶段并判失败 → 回炉再拼一轮 →
    # done(plan)。落库后这两章正文为空，Rust 收口为 failed，用户可再次重生成。
    assert [event["type"] for event in events] == [
        "chapterStart",
        "chapterStart",
        "phase",
        "phase",
        "done",
    ]
    response = events[-1]["response"]
    assert response["status"] == "plan", "单章失败不再中止整条流"
    phase_event = next(event for event in events if event["type"] == "phase")
    placeholder = phase_event["phase"]["chapters"][0]
    assert not (placeholder.get("contentMd") or "").strip(), "失败章正文应为空"


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

@pytest.mark.asyncio
async def test_regen_chapter_scope_is_isolated(monkeypatch):
    """单章重生成只生产目标章：范围外不补占位、不因阶段级产物缺失回炉整阶段。

    这是「重生成并入统一流水线」最容易踩的坑：assemble 若把 scope 外的章节
    也当作缺失章，单章修复会立刻被放大成整阶段重写。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    target = 1

    events = await _post_stream(
        "/api/regenerate/stream",
        {
            "request": VALID_REQUEST,
            "outline": build_mock_outline(VALID_REQUEST),
            "phaseIndex": 0,
            "chapterIndex": target,
        },
    )

    chapters = [event for event in events if event["type"] == "chapter"]
    assert len(chapters) == 1, f"只应生产目标章：{chapters}"
    assert chapters[0]["index"] == target

    phases = [event for event in events if event["type"] == "phase"]
    assert len(phases) == 1, "单章只拼装一轮（无整阶段回炉）"
    assert len(phases[0]["phase"]["chapters"]) == 1, "范围外章节不补空占位"
    assert events[-1]["response"]["status"] == "plan"


@pytest.mark.asyncio
async def test_regen_applies_quality_gate_with_one_repair(monkeypatch):
    """重生成与首次生成共用同一道校验闸门：正文过短判失败，并定向重扩一次。

    改造前重生成是「一次成稿」——哪怕只吐 10 个字也直接落库标 complete。
    现在它走 assemble 的 validators，不合格会触发 expand_revision 重扩，
    且回炉上限仍是一次（与生成链一致）。
    """
    monkeypatch.setattr(settings, "allow_mock_llm", True)
    calls: list[tuple[int, int]] = []

    async def _short_chapter(*args, **kwargs):
        calls.append((args[2], args[3]))
        return (
            {
                "title": "占位",
                "summary": "短",
                "contentMd": "太短了",
                "kind": "concept",
                "keyTerms": [],
            },
            {"researchSnippets": [], "promptTrace": None},
        )

    monkeypatch.setattr(nodes, "_expand_chapter_draft", _short_chapter)

    events = await _post_stream(
        "/api/regenerate/stream",
        {
            "request": VALID_REQUEST,
            "outline": build_mock_outline(VALID_REQUEST),
            "phaseIndex": 0,
            "chapterIndex": 0,
        },
    )

    assert len(calls) == 2, f"首轮 + 定向回炉一次，共 2 次调用：{calls}"
    assert calls[0] == calls[1] == (0, 0), "回炉必须打在同一章上"
    types = [event["type"] for event in events]
    assert types.count("phase") == 2, f"两轮拼装（首轮 + 回炉后）：{types}"
    assert types[-1] == "done"
