"""LLM 工厂：根据配置返回 LangChain 聊天模型实例。

支持五家提供商（设置页可热更新，经 POST /api/config 生效）：
- openai    通用 OpenAI（langchain-openai ChatOpenAI，默认）
- anthropic Claude 原生 SDK（langchain-anthropic ChatAnthropic）
- deepseek  DeepSeek 官方 SDK（langchain-deepseek ChatDeepSeek）
- glm       智谱 GLM（OpenAI 兼容端点 open.bigmodel.cn/api/paas/v4）
- opencode  OpenCode Go 套餐——OpenAI 兼容端点，复用 ChatOpenAI 换 baseUrl

所有结构化输出（强制 JSON）均通过 `llm.with_structured_output(PydanticModel)`
实现：由 provider 的 function calling / JSON schema 能力保证输出可解析。
"""

from __future__ import annotations

from langchain_core.language_models.chat_models import BaseChatModel
from pydantic import SecretStr

from app.config import settings

# OpenCode Go 套餐的 OpenAI 兼容端点（订阅计费路径；官方文档 docs/opencode.ai/docs/go）
OPENCODE_BASE_URL = "https://opencode.ai/zen/go/v1"

# 智谱开放平台 OpenAI 兼容端点（Bearer 鉴权，与 opencode 同构复用 ChatOpenAI）
ZHIPU_BASE_URL = "https://open.bigmodel.cn/api/paas/v4/"

# 统一的调用参数：规划/评审需要稳定低随机性 + 长超时 + 瞬时失败重试
_TEMPERATURE = 0.2
_TIMEOUT_SECONDS = 600
_MAX_RETRIES = 2

# 模块级缓存：配置热更新时由 reset_chat_model_cache() 清空，下次调用按新配置重建
_chat_model: BaseChatModel | None = None


def has_llm_config() -> bool:
    """当前提供商是否已配置 API Key。"""
    return bool(settings.current_api_key())


def get_chat_model() -> BaseChatModel:
    """返回聊天模型实例（缓存单例，配置热更新后自动重建）。

    raise ValueError：提供商名无法识别时抛出。
    """
    global _chat_model
    if _chat_model is None:
        _chat_model = _build_chat_model()
    return _chat_model


def reset_chat_model_cache() -> None:
    """丢弃缓存的模型实例：设置页热更新配置后调用，下次 get_chat_model 重建。"""
    global _chat_model
    _chat_model = None


def _build_chat_model() -> BaseChatModel:
    """按当前配置构建聊天模型实例。

    注意各 SDK 的构造差异（按已装版本的真实签名）：
    - ChatAnthropic 的 pydantic alias：模型参数须传 model_name，stop 须显式 None；
    - SecretStr 包装：langchain 各集成统一要求 SecretStr 类型的 Key。
    """
    provider = settings.llm_provider

    if provider == "anthropic":
        from langchain_anthropic import ChatAnthropic

        return ChatAnthropic(
            model_name=settings.anthropic_model,
            stop=None,
            api_key=SecretStr(settings.anthropic_api_key),
            temperature=_TEMPERATURE,
            timeout=_TIMEOUT_SECONDS,
            max_retries=_MAX_RETRIES,
        )

    if provider == "deepseek":
        from langchain_deepseek import ChatDeepSeek

        return ChatDeepSeek(
            model=settings.deepseek_model,
            api_key=SecretStr(settings.deepseek_api_key),
            temperature=_TEMPERATURE,
            timeout=_TIMEOUT_SECONDS,
            max_retries=_MAX_RETRIES,
        )

    if provider == "glm":
        # 智谱官方 OpenAI 兼容端点：超时/重试与其他提供商统一；
        # 不用 langchain-community 的 ChatZhipuAI（httpx 超时硬编码 60s 且包已 sunset）。
        # thinking low：glm-5.3 系列是“始终思考”模型（disabled 被服务端拒，错误 1210
        # 提示仅支持 low/high/max）。不限档位时思考可耗时 100s+ 且 content 不按
        # schema 输出，结构化解析失败；low 压低思考成本让 content 正常返回
        from langchain_openai import ChatOpenAI

        return ChatOpenAI(
            model=settings.glm_model,
            api_key=SecretStr(settings.glm_api_key),
            base_url=ZHIPU_BASE_URL,
            temperature=_TEMPERATURE,
            timeout=_TIMEOUT_SECONDS,
            max_retries=_MAX_RETRIES,
            extra_body={"reasoning_effort": "low"},
        )

    if provider == "opencode":
        # OpenCode Go：OpenAI 兼容协议，仅端点不同
        from langchain_openai import ChatOpenAI

        return ChatOpenAI(
            model=settings.opencode_model,
            api_key=SecretStr(settings.opencode_api_key),
            base_url=OPENCODE_BASE_URL,
            temperature=_TEMPERATURE,
            timeout=_TIMEOUT_SECONDS,
            max_retries=_MAX_RETRIES,
        )

    if provider == "openai":
        from langchain_openai import ChatOpenAI

        return ChatOpenAI(
            model=settings.openai_model,
            api_key=SecretStr(settings.openai_api_key),
            temperature=_TEMPERATURE,
            timeout=_TIMEOUT_SECONDS,
            max_retries=_MAX_RETRIES,
        )

    raise ValueError(
        f"不支持的 LLM 提供商：{provider}"
        "（可选 openai / anthropic / deepseek / glm / opencode）"
    )
