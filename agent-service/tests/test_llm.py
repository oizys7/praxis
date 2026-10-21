"""invoke_structured 空 tool_call 重试链回归（2026-10-15 真机 GLM 故障）。

背景：GLM 真机逐段生成时 200 返回但 tool_call 参数为空，
with_structured_output 返回 None；原实现经 cast 直接穿透，
在 nodes._expand_phase_draft 的 model_dump 上空引用崩溃。
本用例锚定：None 按解析失败进入反悔重试链，耗尽后报专用文案。
"""

import pytest
from pydantic import BaseModel

from app import llm
from app.errors import AgentServiceError

_VARS = {
    "plan_context": "测试上下文",
    "phase_outline": "{}",
    "review_notes": "",
    "level": "初学者",
    "weekly_hours": "5",
    "weeks": "4",
}


class _Tiny(BaseModel):
    x: int


class _FakeStructured:
    def __init__(self, results):
        self._results = list(results)
        self.calls = []

    async def ainvoke(self, messages):
        self.calls.append(list(messages))
        if not self._results:
            raise AssertionError("结果序列耗尽")
        item = self._results.pop(0)
        # 结果为异常实例时模拟抛出（用于验证异常是否进入重试链）
        if isinstance(item, BaseException):
            raise item
        return item


class _FakeChat:
    def __init__(self, fake):
        self._fake = fake

    def with_structured_output(self, model_cls, method):
        return self._fake


@pytest.fixture
def funnel(monkeypatch):
    def install(results):
        fake = _FakeStructured(results)
        monkeypatch.setattr(llm, "get_chat_model", lambda: _FakeChat(fake))
        return fake

    return install


def test_empty_tool_call_twice_raises_dedicated_error(funnel):
    """两次空参数 → 耗尽 → 专用 AgentServiceError 文案。"""
    fake = funnel([None, None])
    with pytest.raises(AgentServiceError) as ei:
        import asyncio

        asyncio.run(llm.invoke_structured(_Tiny, "phase", _VARS))
    assert "空参数" in str(ei.value)
    # 两次调用都真实发生，且第二次带上了空参数反馈消息
    assert len(fake.calls) == 2
    assert any("未提供任何参数" in str(m.content) for m in fake.calls[1][1:])


def test_empty_tool_call_then_valid_returns_model(funnel):
    """第一次空参数 → 反馈重试后正常返回模型实例。"""
    fake = funnel([None, _Tiny(x=1)])
    import asyncio

    result = asyncio.run(llm.invoke_structured(_Tiny, "phase", _VARS))
    assert result.x == 1
    assert len(fake.calls) == 2


def test_output_parser_error_enters_retry_chain(funnel):
    """JSON 截断/非法（OutputParserException）此前不进重试链 → 现已纳入。"""
    import asyncio

    from langchain_core.exceptions import OutputParserException

    fake = funnel(
        [OutputParserException('Invalid json output: {"contentMd": "截断'), _Tiny(x=2)]
    )
    result = asyncio.run(llm.invoke_structured(_Tiny, "phase", _VARS))
    assert result.x == 2
    assert len(fake.calls) == 2
    assert any("无法解析" in str(m.content) for m in fake.calls[1][1:])


def test_output_parser_error_exhausted_raises_parse_error(funnel):
    """连续解析失败耗尽预算 → AgentServiceError（文案指向"无法解析"）。"""
    import asyncio

    from langchain_core.exceptions import OutputParserException

    fake = funnel([OutputParserException("bad"), OutputParserException("bad")])
    with pytest.raises(AgentServiceError) as ei:
        asyncio.run(llm.invoke_structured(_Tiny, "phase", _VARS))
    assert "无法解析" in str(ei.value)
    assert len(fake.calls) == 2


def test_chapter_prompt_gets_three_parse_attempts(funnel):
    """章节正文输出最大：解析预算 3 次，其余提示词仍为 2 次。"""
    import asyncio

    from langchain_core.exceptions import OutputParserException

    assert llm._parse_attempts("chapter") == 3
    assert llm._parse_attempts("outline") == 2
    fake = funnel([OutputParserException("a"), OutputParserException("b"), _Tiny(x=3)])
    result = asyncio.run(llm.invoke_structured(_Tiny, "chapter", _VARS))
    assert result.x == 3
    assert len(fake.calls) == 3
