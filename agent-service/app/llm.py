"""LLM 工厂：根据配置返回 LangChain 聊天模型实例。

通过环境变量切换提供商：
- LLM_PROVIDER=openai      -> langchain-openai 的 ChatOpenAI（默认）
- LLM_PROVIDER=anthropic   -> langchain-anthropic 的 ChatAnthropic

所有结构化输出（强制 JSON）均通过 `llm.with_structured_output(PydanticModel)`
实现：由 provider 的 function calling / JSON schema 能力保证输出可解析。
"""

from __future__ import annotations

from functools import lru_cache

from langchain_core.language_models.chat_models import BaseChatModel

from app.config import settings


def has_llm_config() -> bool:
    """当前提供商是否已配置 API Key。"""
    if settings.llm_provider == "anthropic":
        return bool(settings.anthropic_api_key)
    return bool(settings.openai_api_key)


@lru_cache(maxsize=4)
def get_chat_model() -> BaseChatModel:
    """构建聊天模型实例（带缓存）。

    - temperature=0.2：规划/评审任务需要稳定、低随机性
    - timeout=120：单次 LLM 调用上限，避免请求悬挂
    - max_retries=2：瞬时失败自动重试

    raise ValueError：提供商名无法识别时抛出。
    """
    if settings.llm_provider == "anthropic":
        from langchain_anthropic import ChatAnthropic

        return ChatAnthropic(
            model=settings.anthropic_model,
            api_key=settings.anthropic_api_key,
            temperature=0.2,
            timeout=120,
            max_retries=2,
        )

    if settings.llm_provider == "openai":
        from langchain_openai import ChatOpenAI

        return ChatOpenAI(
            model=settings.openai_model,
            api_key=settings.openai_api_key,
            temperature=0.2,
            timeout=120,
            max_retries=2,
        )

    raise ValueError(
        f"不支持的 LLM_PROVIDER：{settings.llm_provider}（可选 openai / anthropic）"
    )
