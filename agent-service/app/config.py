"""应用配置：环境变量 / agent-service/.env 提供初始值，运行时可被设置页热更新。

配置加载分两个阶段：
- 冷启动：`load_dotenv()` 读取 .env（dev 模式），release 模式由 Rust 壳在拉起
  sidecar 时把 settings.json 的值注入环境变量——两者都只作为「初始默认值」；
- 热更新：设置页保存后，Rust 调 `POST /api/config` 推送全量配置，
  `settings` 单例通过 `update_runtime()` 就地更新，无需重启服务进程。

支持五家 LLM 提供商（每家独立 API Key 与模型名，切换互不覆盖）：
- openai    通用 OpenAI
- anthropic Claude（原生 SDK）
- deepseek  DeepSeek（官方 langchain-deepseek SDK）
- glm       智谱 GLM（OpenAI 兼容端点 open.bigmodel.cn/api/paas/v4）
- opencode  OpenCode Go 套餐（OpenAI 兼容端点，订阅计费）

其他模块一律通过 `settings` 单例访问（属性读取始终是最新值）。
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

from dotenv import load_dotenv

# 加载 agent-service/.env（存在时）；不覆盖已有环境变量
load_dotenv()


def _bool(value: str | None) -> bool:
    """宽松解析布尔环境变量：1/true/yes/on 均视为 True。"""
    return (value or "").strip().lower() in {"1", "true", "yes", "on"}


def _int_env(name: str, default: int) -> int:
    """读取整型环境变量；缺失或非法时回退到默认值，不让配置错误拖垮启动。"""
    try:
        return int(os.getenv(name, "") or default)
    except ValueError:
        return default


# 每家提供商的字段名映射：key 字段 / model 字段（模块级常量，非实例状态）
_PROVIDER_FIELDS: dict[str, tuple[str, str]] = {
    "openai": ("openai_api_key", "openai_model"),
    "anthropic": ("anthropic_api_key", "anthropic_model"),
    "deepseek": ("deepseek_api_key", "deepseek_model"),
    "glm": ("glm_api_key", "glm_model"),
    "opencode": ("opencode_api_key", "opencode_model"),
}

# 热更新字段映射：三端协议的 camelCase 名 → Settings 字段名。
# 显式列出而非运行时推导：协议本身是三端手写对齐的，这里保持同样直观，
# 「哪些字段可热更」一眼可查（推导版本曾因大小写规则出错丢失中间段）
_RUNTIME_FIELDS: dict[str, str] = {
    "llmProvider": "llm_provider",
    "openaiApiKey": "openai_api_key",
    "openaiModel": "openai_model",
    "anthropicApiKey": "anthropic_api_key",
    "anthropicModel": "anthropic_model",
    "deepseekApiKey": "deepseek_api_key",
    "deepseekModel": "deepseek_model",
    "glmApiKey": "glm_api_key",
    "glmModel": "glm_model",
    "opencodeApiKey": "opencode_api_key",
    "opencodeModel": "opencode_model",
}


@dataclass
class Settings:
    """Praxis Agent 服务配置。

    注意：不再是 frozen——设置页热更新需要就地修改；监听地址 / 端口
    属于进程生命周期属性，不支持热更（修改需重启服务）。
    """

    # LLM 提供商："openai" | "anthropic" | "deepseek" | "glm" | "opencode"
    llm_provider: str = (os.getenv("LLM_PROVIDER") or "openai").strip().lower()

    # 通用 OpenAI
    openai_api_key: str = os.getenv("OPENAI_API_KEY", "")
    openai_model: str = os.getenv("OPENAI_MODEL", "gpt-4o-mini")

    # Anthropic（Claude）
    anthropic_api_key: str = os.getenv("ANTHROPIC_API_KEY", "")
    anthropic_model: str = os.getenv("ANTHROPIC_MODEL", "claude-3-5-sonnet-latest")

    # DeepSeek（官方 SDK 约定的环境变量名 DEEPSEEK_API_KEY）
    deepseek_api_key: str = os.getenv("DEEPSEEK_API_KEY", "")
    deepseek_model: str = os.getenv("DEEPSEEK_MODEL", "deepseek-chat")

    # 智谱 GLM（开放平台 bigmodel.cn）
    glm_api_key: str = os.getenv("GLM_API_KEY", "")
    glm_model: str = os.getenv("GLM_MODEL", "glm-5.3-flash")

    # OpenCode Go 套餐（opencode.ai/zen，订阅计费）
    opencode_api_key: str = os.getenv("OPENCODE_API_KEY", "")
    opencode_model: str = os.getenv("OPENCODE_MODEL", "grok-4.7")

    # 服务监听地址（与 Tauri 侧 AGENT_SERVICE_URL 对应；不支持热更）
    host: str = os.getenv("AGENT_SERVICE_HOST", "127.0.0.1")
    port: int = _int_env("AGENT_SERVICE_PORT", 8000)

    # 离线演示模式：为 True 时不调用 LLM，返回确定性 mock 计划（便于无 Key 开发调试）
    allow_mock_llm: bool = _bool(os.getenv("PRAXIS_ALLOW_MOCK_LLM"))

    def current_api_key(self) -> str:
        """返回当前提供商已配置的 API Key（未识别的提供商返回空串）。"""
        fields = _PROVIDER_FIELDS.get(self.llm_provider)
        if fields is None:
            return ""
        return str(getattr(self, fields[0]))

    def current_model(self) -> str:
        """返回当前提供商配置的模型名。"""
        fields = _PROVIDER_FIELDS.get(self.llm_provider)
        if fields is None:
            return ""
        return str(getattr(self, fields[1]))

    def update_runtime(self, payload: dict[str, Any]) -> None:
        """从设置页推送的 camelCase 配置就地更新（仅 LLM 相关字段支持热更）。

        - 缺失的键保留当前值，因此 Rust 端可以安全地推送「全量或部分」配置；
        - 监听地址 / 端口刻意不在此列：进程绑定后不可热更。
        """
        for camel, field in _RUNTIME_FIELDS.items():
            if camel not in payload:
                continue
            value = str(payload[camel]).strip()
            if camel == "llmProvider":
                value = value.lower()
            setattr(self, field, value)
        if "allowMockLlm" in payload:
            self.allow_mock_llm = bool(payload["allowMockLlm"])

    def public_dict(self) -> dict[str, Any]:
        """导出 camelCase 配置视图（API Key 原文返回：桌面单用户本机场景）。

        供 GET /api/config 与 POST /api/config 响应使用，字段名与
        三端协议的 AgentSettings（agent.ts / commands.rs / schemas.py）对齐。
        """
        result: dict[str, Any] = {
            "llmProvider": self.llm_provider,
            "allowMockLlm": self.allow_mock_llm,
        }
        for provider, (key_field, model_field) in _PROVIDER_FIELDS.items():
            result[f"{provider}ApiKey"] = str(getattr(self, key_field))
            result[f"{provider}Model"] = str(getattr(self, model_field))
        return result


settings = Settings()
