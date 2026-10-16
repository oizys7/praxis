"""应用配置：config.json 冷启动装载 + 设置页热更新链。

配置来源分工明确：
- 冷启动（本模块）：仓库/工作区 config.json 的 Python 运行时区段——
  service.host / service.port / logging.level / logging.dir / agent.mockMode。
  查找顺序：CLI --config（前置扫描 sys.argv，Rust sidecar 拉起时的标准传参）
  → 工作区根（沿 cwd 向上找 src-tauri marker，与 Rust 侧一致）→ 应用数据
  目录。全部落空 = 致命错：打印候选路径后 sys.exit(2)。
- LLM 无环境变量冷启动：LLM 十二字段只有 settings.json → POST /api/config
  一条运行时链（设置页保存即热更）。启动时未配置任何 Key 且 mockMode 未开
  → 服务正常启动（健康检查可用），生成接口返回明确错误引导到应用设置页。
- 热更新：设置页推送的 camelCase 配置经 settings.update_runtime() 就地更新，
  无需重启服务进程。

支持五家 LLM 提供商（每家独立 API Key 与模型名，切换互不覆盖）：
- openai    通用 OpenAI
- anthropic Claude（原生 SDK）
- deepseek  DeepSeek（官方 langchain-deepseek SDK）
- glm       智谱 GLM（OpenAI 兼容端点 open.bigmodel.cn/api/paas/v4）
- opencode  OpenCode Go 套餐（OpenAI 兼容端点，订阅计费）

其他模块一律通过 settings 单例访问（属性读取始终是最新值）；
logging 级别 / 目录经 runtime_config 单例读取（见 app.logging_config）。
"""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# --------------------------------------------------------------------------
# 运行数据目录常量：仅由平台 + Path.home() 推导，不读 LOCALAPPDATA/XDG_DATA_HOME
# 等 OS 约定变量（配置文件化后 Python 侧零环境变量读取）
# --------------------------------------------------------------------------
if os.name == "nt":
    # 已申报的平台分歧（任务 29 审查 F3）：Rust config.json 兜底候选用
    # dirs::data_dir()（Windows = Roaming），Python 这里 = Local
    # （与 Tauri app_log_dir 日志目录同构）；主链路 sidecar 显式传
    # --config 不受影响，兜底候选两端可能不同文件，仅注释申报
    RUNTIME_DATA_DIR = Path.home() / "AppData/Local/com.praxis.desktop"
else:
    RUNTIME_DATA_DIR = Path.home() / ".local/share/com.praxis.desktop"

# 共享配置文件名与 workspace marker（与 Rust 侧 src-tauri/src/config.rs 对齐）：
# marker 存在即认为该目录是工作区根，config.json 放在根下与 config.example.json 同位
CONFIG_FILENAME = "config.json"
WORKSPACE_MARKER = Path("src-tauri") / "tauri.conf.json"


class ConfigNotFoundError(RuntimeError):
    """config.json 未找到：候选路径已随异常信息列出（bootstrap 据此 exit(2)）。"""


class ConfigParseError(RuntimeError):
    """config.json 解析失败：错误信息携带 config.example.json 的点路径字段名。"""


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
class PraxisRuntimeConfig:
    """config.json 中 Python 侧消费的运行时区段（与 config.example.json 对齐）。

    Python 只读三区段：logging（级别/目录）、service（监听 host/port）、
    agent.mockMode（离线演示冷启动开关）；database/secrets 供 Rust/测试使用，
    本模块不解析（未知区段静默忽略，两端共享同一文件互不干扰）。
    """

    # logging 区段
    logging_level: str = "info"
    logging_dir: str = ""  # 空 = 运行数据目录常量下的 logs/（logging_config 推导）

    # service 区段（监听地址/端口属进程生命周期属性，不支持 /api/config 热更）
    host: str = "127.0.0.1"
    port: int = 8000

    # agent 区段：冷启动默认；settings.json 的 mock 开关仍是运行时权威
    mock_mode: bool = False


@dataclass
class Settings:
    """LLM 十二字段配置（settings.json → /api/config 链路的运行时载体）。

    注意：不再是 frozen——设置页热更新需要就地修改；监听地址 / 端口属于
    进程生命周期属性，由 config.json 冷启动注入（bootstrap_runtime），
    不支持热更（修改需重启服务）。
    """

    # LLM 提供商："openai" | "anthropic" | "deepseek" | "glm" | "opencode"
    llm_provider: str = "openai"

    # 通用 OpenAI
    openai_api_key: str = ""
    openai_model: str = "gpt-4o-mini"

    # Anthropic（Claude）
    anthropic_api_key: str = ""
    anthropic_model: str = "claude-3-5-sonnet-latest"

    # DeepSeek（官方 SDK）
    deepseek_api_key: str = ""
    deepseek_model: str = "deepseek-chat"

    # 智谱 GLM（开放平台 bigmodel.cn）
    glm_api_key: str = ""
    glm_model: str = "glm-5.3-flash"

    # OpenCode Go 套餐（opencode.ai/zen，订阅计费）
    opencode_api_key: str = ""
    opencode_model: str = "grok-4.7"

    # 服务监听（由 bootstrap_runtime 从 config.json 注入，默认值与 Rust 侧一致）
    host: str = "127.0.0.1"
    port: int = 8000

    # 离线演示模式：为 True 时不调用 LLM，返回确定性 mock 计划（便于无 Key 开发调试）
    allow_mock_llm: bool = False

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


# --------------------------------------------------------------------------
# config.json 装载：查找 → 解析（点路径容错）
# --------------------------------------------------------------------------


def extract_cli_config_path(argv: Sequence[str]) -> Path | None:
    """前置扫描 argv 提取 --config（兼容空格与等号两种形式）。

    之所以扫描而非等入口 argparse：uvicorn -m 直跑（开发模式）没有 argparse
    层，模块导入期的 bootstrap 需要同一条通道拿到配置路径。
    """
    for index, arg in enumerate(argv):
        if arg == "--config" and index + 1 < len(argv):
            # 审查 F5 对齐：空值视为未指定（与 Rust config_cli_override
            # 同构——Path("") 会变成候选路径 "."，偏离文案同构）
            if argv[index + 1]:
                return Path(argv[index + 1])
            return None
        if arg.startswith("--config="):
            value = arg[len("--config=") :]
            if value:
                return Path(value)
            return None
    return None


def _workspace_root(start: Path) -> Path | None:
    """沿启动 cwd 向上寻找工作区根（与 Rust app_config.rs 同一双形态 marker）：

    - 某级目录直接含 tauri.conf.json（即 src-tauri/）→ 仓库根为其上一级；
    - 某级目录含 src-tauri/tauri.conf.json（即仓库根）→ 该级目录即仓库根。

    20 级上限防异常挂载的死循环。未命中返回 None（release 独立运行无标记）。
    """
    for directory in [start, *start.resolve().parents][:20]:
        if (directory / "tauri.conf.json").is_file():
            parent = directory.parent
            return parent if str(parent) else None
        if (directory / WORKSPACE_MARKER).is_file():
            return directory
    return None


def find_config_path(explicit: Path | None = None) -> tuple[Path | None, list[Path]]:
    """按查找顺序返回（命中路径, 候选列表）。

    顺序：CLI --config > 工作区根 config.json（src-tauri marker）>
    应用数据目录 config.json（release 独立运行兜底）。候选列表用于
    未命中时的致命错误文案（与 Rust 壳同构）。

    与 Rust 端 search_candidates 同语义：CLI 覆盖是「优先」而非排他 ——
    显式路径不存在时继续尝试其余候选，保证两端同一文件查找行为一致。
    """
    candidates: list[Path] = []
    if explicit is not None:
        candidates.append(explicit)
    workspace = _workspace_root(Path.cwd())
    if workspace is not None:
        candidates.append(workspace / CONFIG_FILENAME)
    candidates.append(RUNTIME_DATA_DIR / CONFIG_FILENAME)
    for candidate in candidates:
        if candidate.is_file():
            return candidate, candidates
    return None, candidates


def _not_found_message(candidates: list[Path]) -> str:
    """未命中致命错文案：与 Rust 端 AppConfigError::NotFound 同构。"""
    listed = "\n".join(f"- {path}" for path in candidates)
    return (
        "未找到配置文件 config.json：请复制仓库根 config.example.json 为 config.json 并填写。\n"
        "已按顺序查找以下位置，均未找到可读的 config.json：\n"
        f"{listed}"
    )


def _section(raw: dict[str, Any], name: str) -> dict[str, Any]:
    """取区段对象；缺失按空对象容错，类型不对则报点路径错误。"""
    value = raw.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ConfigParseError(
            f"配置字段 {name} 必须是对象，实际为 {type(value).__name__}"
        )
    return value


def _string(section: dict[str, Any], name: str, path: str, default: str) -> str:
    """取字符串字段；缺失回默认值，类型不对报点路径错误。"""
    value = section.get(name)
    if value is None:
        return default
    if not isinstance(value, str):
        raise ConfigParseError(f"配置字段 {path} 必须是字符串，实际为 {value!r}")
    return value


def _port(section: dict[str, Any], name: str, path: str, default: int) -> int:
    """取 1-65535 的端口整数；缺失回默认值，类型/范围不对报点路径错误。"""
    value = section.get(name)
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise ConfigParseError(f"配置字段 {path} 必须是 1-65535 的整数，实际为 {value!r}")
    if not 1 <= value <= 65535:
        raise ConfigParseError(f"配置字段 {path} 必须是 1-65535 的整数，实际为 {value!r}")
    return value


def _flag(section: dict[str, Any], name: str, path: str, default: bool) -> bool:
    """取布尔开关；缺失回默认值，类型不对报点路径错误。"""
    value = section.get(name)
    if value is None:
        return default
    if not isinstance(value, bool):
        raise ConfigParseError(f"配置字段 {path} 必须是布尔值，实际为 {value!r}")
    return value


def _parse_config(raw: Any) -> PraxisRuntimeConfig:
    """解析 config.json dict → PraxisRuntimeConfig（缺区段容错，点路径报错）。"""
    if not isinstance(raw, dict):
        raise ConfigParseError(
            f"配置文件根必须是 JSON 对象，实际为 {type(raw).__name__}"
        )
    logging_section = _section(raw, "logging")
    service = _section(raw, "service")
    agent = _section(raw, "agent")
    return PraxisRuntimeConfig(
        logging_level=_string(logging_section, "level", "logging.level", "info"),
        logging_dir=_string(logging_section, "dir", "logging.dir", ""),
        host=_string(service, "host", "service.host", "127.0.0.1"),
        port=_port(service, "port", "service.port", 8000),
        mock_mode=_flag(agent, "mockMode", "agent.mockMode", False),
    )


def load_runtime_config(path: Path | None = None) -> PraxisRuntimeConfig:
    """装载 config.json 的 Python 运行时区段；未命中抛 ConfigNotFoundError。

    path 为 None 时按查找顺序检索（见 find_config_path）；路径给定时只查它。
    解析失败（非法 JSON / 类型不符）抛 ConfigParseError，信息携带点路径。
    """
    found, candidates = find_config_path(path)
    if found is None:
        raise ConfigNotFoundError(_not_found_message(candidates))
    try:
        text = found.read_text(encoding="utf-8")
    except OSError as exc:
        raise ConfigParseError(f"配置文件无法读取（{found}）：{exc}") from exc
    try:
        raw = json.loads(text)
    except json.JSONDecodeError as exc:
        # 与 Rust 端 AppConfigError 文案同构：带文件路径原文
        raise ConfigParseError(f"配置文件解析失败（{found}）：{exc}") from exc
    return _parse_config(raw)


# --------------------------------------------------------------------------
# settings 单例 + 导入期 bootstrap
# --------------------------------------------------------------------------

settings = Settings()

# 导入期 bootstrap（模块底部调用）会覆盖为实际装载值；先放默认实例
# 以便测试/异常路径总能 import 成功（logging_config 引用它推导默认日志目录）
runtime_config = PraxisRuntimeConfig()


def bootstrap_runtime(argv: Sequence[str]) -> PraxisRuntimeConfig:
    """冷启动 bootstrap：argv 扫描 --config → 装载 → 应用到 settings/runtime_config。

    未找到 / 解析失败 = 致命错：打印原因与候选路径后 sys.exit(2)（与 Rust
    壳缺配置文案同构）。模块导入时自动执行一次；agent_entry 等入口可显式
    重放（幂等：同一 argv 装载同一结果）。
    """
    global runtime_config
    try:
        cfg = load_runtime_config(extract_cli_config_path(argv))
    except (ConfigNotFoundError, ConfigParseError) as exc:
        # 审查 F2：PyInstaller --noconsole 下标准流为 None，print 会
        # AttributeError 坏掉 exit(2) 契约——守护后静默退出（错误信息
        # 由 Rust 壳的提示弹窗兜底，见任务 28 弹窗链）
        if sys.stderr is not None:
            print(exc, file=sys.stderr)
        sys.exit(2)
    runtime_config = cfg
    # 监听与 mock 冷启动注入；LLM 十二字段仍走设置页 → /api/config 热链
    settings.host = cfg.host
    settings.port = cfg.port
    settings.allow_mock_llm = cfg.mock_mode
    return cfg


bootstrap_runtime(sys.argv)
