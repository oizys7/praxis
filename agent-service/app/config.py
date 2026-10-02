"""应用配置：从环境变量 / agent-service/.env 读取。

所有配置集中在此，其他模块一律通过 `settings` 单例访问。
"""

from __future__ import annotations

import os
from dataclasses import dataclass

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


@dataclass(frozen=True)
class Settings:
    """Praxis Agent 服务配置。"""

    # LLM 提供商："openai" 或 "anthropic"
    llm_provider: str = (os.getenv("LLM_PROVIDER") or "openai").strip().lower()

    # OpenAI 配置
    openai_api_key: str = os.getenv("OPENAI_API_KEY", "")
    openai_model: str = os.getenv("OPENAI_MODEL", "gpt-4o-mini")

    # Anthropic 配置
    anthropic_api_key: str = os.getenv("ANTHROPIC_API_KEY", "")
    anthropic_model: str = os.getenv("ANTHROPIC_MODEL", "claude-3-5-sonnet-latest")

    # 服务监听地址（与 Tauri 侧 AGENT_SERVICE_URL 对应）
    host: str = os.getenv("AGENT_SERVICE_HOST", "127.0.0.1")
    port: int = _int_env("AGENT_SERVICE_PORT", 8000)

    # 离线演示模式：为 True 时不调用 LLM，返回确定性 mock 计划（便于无 Key 开发调试）
    allow_mock_llm: bool = _bool(os.getenv("PRAXIS_ALLOW_MOCK_LLM"))


settings = Settings()
