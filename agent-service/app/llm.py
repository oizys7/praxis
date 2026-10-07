"""LLM 工厂与调用层：模型构建、结构化调用、异常归一。

支持五家提供商（设置页可热更新，经 POST /api/config 生效）：
- openai    通用 OpenAI（langchain-openai ChatOpenAI，默认）
- anthropic Claude 原生 SDK（langchain-anthropic ChatAnthropic）
- deepseek  DeepSeek 官方 SDK（langchain-deepseek ChatDeepSeek）
- glm       智谱 GLM（OpenAI 兼容端点 open.bigmodel.cn/api/paas/v4）
- opencode  OpenCode Go 套餐——OpenAI 兼容端点，复用 ChatOpenAI 换 baseUrl

节点层只应使用 invoke_structured()：提示词渲染、结构化解析、解析重试
与传输层异常翻译全部收敛在这里，业务代码不接触 LangChain / SDK 细节。
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from typing import TypeVar, cast

import httpx
import openai
from langchain_core.language_models.chat_models import BaseChatModel
from langchain_core.messages import HumanMessage, SystemMessage
from pydantic import BaseModel, SecretStr, ValidationError

from app import prompts
from app.config import settings
from app.errors import AgentServiceError

logger = logging.getLogger("praxis.llm")

# OpenCode Go 套餐的 OpenAI 兼容端点（订阅计费路径；官方文档 docs.opencode.ai/docs/go）
OPENCODE_BASE_URL = "https://opencode.ai/zen/go/v1"

# 智谱开放平台 OpenAI 兼容端点（Bearer 鉴权，与 opencode 同构复用 ChatOpenAI）
ZHIPU_BASE_URL = "https://open.bigmodel.cn/api/paas/v4/"

# 统一的调用参数：规划/评审需要稳定低随机性 + 长超时 + 瞬时失败重试
_TEMPERATURE = 0.2
_TIMEOUT_SECONDS = 600
_MAX_RETRIES = 2

# 结构化输出的解析尝试次数：glm-5.3 等思考模型对大 schema 的工具参数
# 偶发拼接错乱（字段丢失 / 类型漂移），一次重试通常即可成功
_PARSE_ATTEMPTS = 2

# 模块级缓存：配置热更新时由 reset_chat_model_cache() 清空，下次调用按新配置重建
_chat_model: BaseChatModel | None = None


def has_llm_config() -> bool:
    """当前提供商是否已配置 API Key。"""
    return bool(settings.current_api_key())


def get_chat_model() -> BaseChatModel:
    """返回聊天模型实例（缓存单例，配置热更新后自动重建）。"""
    global _chat_model
    if _chat_model is None:
        _chat_model = _build_chat_model()
    return _chat_model


def reset_chat_model_cache() -> None:
    """丢弃缓存的模型实例：设置页热更新配置后调用，下次 get_chat_model 重建。"""
    global _chat_model
    _chat_model = None


# ---------------------------------------------------------------------------
# 模型构建：工厂表分发，新提供商只需一个小函数 + 注册一行
# ---------------------------------------------------------------------------


def _openai_compatible(
    model: str,
    api_key: str,
    *,
    base_url: str | None = None,
    extra_body: dict[str, object] | None = None,
) -> BaseChatModel:
    """构造 OpenAI 协议族模型：openai / glm / opencode 三个分支共用。"""
    from langchain_openai import ChatOpenAI

    return ChatOpenAI(
        model=model,
        api_key=SecretStr(api_key),
        base_url=base_url,
        extra_body=extra_body,
        temperature=_TEMPERATURE,
        timeout=_TIMEOUT_SECONDS,
        max_retries=_MAX_RETRIES,
    )


def _build_openai() -> BaseChatModel:
    return _openai_compatible(settings.openai_model, settings.openai_api_key)


def _build_glm() -> BaseChatModel:
    # 智谱不走 langchain-community 的 ChatZhipuAI（httpx 超时硬编码 60s 且包已
    # sunset），改用官方 OpenAI 兼容端点。reasoning_effort=low 用于压制 glm-5.3
    # 系列的强制思考：thinking.type=disabled 被服务端拒绝（错误 1210，官方迁移
    # 指南指定用 reasoning_effort 控制思考程度）；不限档位时单次调用可耗时
    # 100s+ 且 content 不按 schema 输出，结构化解析会失败
    return _openai_compatible(
        settings.glm_model,
        settings.glm_api_key,
        base_url=ZHIPU_BASE_URL,
        extra_body={"reasoning_effort": "low"},
    )


def _build_opencode() -> BaseChatModel:
    return _openai_compatible(
        settings.opencode_model, settings.opencode_api_key, base_url=OPENCODE_BASE_URL
    )


def _build_anthropic() -> BaseChatModel:
    # 注意已装版本的真实签名：模型参数须传 model_name（model 字段的 alias），
    # stop 须显式 None，Key 须 SecretStr 包装
    from langchain_anthropic import ChatAnthropic

    return ChatAnthropic(
        model_name=settings.anthropic_model,
        stop=None,
        api_key=SecretStr(settings.anthropic_api_key),
        temperature=_TEMPERATURE,
        timeout=_TIMEOUT_SECONDS,
        max_retries=_MAX_RETRIES,
    )


def _build_deepseek() -> BaseChatModel:
    from langchain_deepseek import ChatDeepSeek

    return ChatDeepSeek(
        model=settings.deepseek_model,
        api_key=SecretStr(settings.deepseek_api_key),
        temperature=_TEMPERATURE,
        timeout=_TIMEOUT_SECONDS,
        max_retries=_MAX_RETRIES,
    )


_BUILDERS: dict[str, Callable[[], BaseChatModel]] = {
    "openai": _build_openai,
    "anthropic": _build_anthropic,
    "deepseek": _build_deepseek,
    "glm": _build_glm,
    "opencode": _build_opencode,
}


def _build_chat_model() -> BaseChatModel:
    """按当前配置构建模型实例；提供商名无法识别时抛 ValueError。"""
    builder = _BUILDERS.get(settings.llm_provider)
    if builder is None:
        raise ValueError(
            f"不支持的 LLM 提供商：{settings.llm_provider}"
            "（可选 openai / anthropic / deepseek / glm / opencode）"
        )
    return builder()


# ---------------------------------------------------------------------------
# 结构化调用：节点层唯一的 LLM 入口
# ---------------------------------------------------------------------------

TModel = TypeVar("TModel", bound=BaseModel)


async def invoke_structured(
    model_cls: type[TModel],
    prompt_key: str,
    variables: dict[str, str],
) -> TModel:
    """加载提示词 → 渲染变量 → 结构化输出 → 解析为 model_cls 实例。

    - 解析失败自动重试（上限 _PARSE_ATTEMPTS）：思考模型偶发的工具参数
      错乱大多一次重试即可恢复；
    - 传输层异常（超时 / 网络错误）统一翻译为 AgentServiceError，图节点
      与 HTTP 端点无需感知 httpx / openai SDK 的异常类型。

    raise AgentServiceError: 解析重试耗尽，或模型调用超时 / 网络失败。
    """
    template = prompts.load_prompt(prompt_key)
    messages = [
        SystemMessage(content=template["system"]),
        HumanMessage(content=prompts.render(template["human"], **variables)),
    ]
    structured = get_chat_model().with_structured_output(
        model_cls, method="function_calling"
    )

    last_parse_error: ValidationError | None = None
    try:
        for attempt in range(1, _PARSE_ATTEMPTS + 1):
            try:
                # ainvoke 静态签名宽泛（dict | BaseModel），运行时必为 model_cls
                return cast(TModel, await structured.ainvoke(messages))
            except ValidationError as exc:
                last_parse_error = exc
                logger.warning(
                    "结构化输出解析失败（第 %d/%d 次，model=%s）",
                    attempt,
                    _PARSE_ATTEMPTS,
                    model_cls.__name__,
                )
    except (httpx.TimeoutException, TimeoutError, openai.APIConnectionError) as exc:
        # openai / anthropic SDK 会把底层 httpx 异常包装为自有类型再抛出
        logger.warning("LLM 调用失败（provider=%s）：%s", settings.llm_provider, exc)
        raise AgentServiceError(
            "模型响应超时或网络异常，请稍后重试，或在设置页更换模型"
        ) from exc

    raise AgentServiceError(
        "模型返回内容无法解析为结构化数据，请重新生成"
    ) from last_parse_error
