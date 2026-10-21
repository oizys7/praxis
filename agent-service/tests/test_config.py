"""config.py 配置文件装载测试：查找顺序 / CLI 前置扫描 / 解析容错 / 装载应用。

覆盖任务 29 的四个验收点：
- load_runtime_config 解析（默认值 / 点路径错误信息 / 非法 JSON）；
- extract_cli_config_path 的 --config <路径> 与 --config=<路径> 两种形式；
- 查找顺序（显式路径 > workspace marker 根 > 应用数据目录）与缺失致命错；
- bootstrap_runtime 把 service/agent 区段应用到 settings 单例；
- logging.dir 覆盖与默认推导（resolve_log_dir）；
- LLM 冷启动退役语义（无 Key 未开 mock → 业务错误引导设置页，不提 .env）。
"""

import json
import sys
from pathlib import Path
from typing import Any

import pytest

import app.config as config_module
from app.config import (
    RUNTIME_DATA_DIR,
    ConfigNotFoundError,
    ConfigParseError,
    bootstrap_runtime,
    extract_cli_config_path,
    load_runtime_config,
    settings,
)
from app.logging_config import resolve_log_dir


def _write_config(tmp_path: Path, raw: Any) -> Path:
    """把 dict 写成临时 config.json，返回路径。"""
    target = tmp_path / "config.json"
    target.write_text(json.dumps(raw, ensure_ascii=False), encoding="utf-8")
    return target


# ---------------------------------------------------------------------------
# 解析：load_runtime_config（显式路径）
# ---------------------------------------------------------------------------


def test_full_config_parses(tmp_path):
    """全字段装载：logging/service/agent 三区段 camelCase 键 → snake_case 字段。"""
    path = _write_config(
        tmp_path,
        {
            "logging": {"level": "debug", "dir": "C:/logs/praxis"},
            "service": {"host": "127.0.0.5", "port": 9100},
            "agent": {"mockMode": True},
        },
    )

    cfg = load_runtime_config(path)

    assert cfg.logging_level == "debug"
    assert cfg.logging_dir == "C:/logs/praxis"
    assert cfg.host == "127.0.0.5"
    assert cfg.port == 9100
    assert cfg.mock_mode is True


def test_empty_config_all_defaults(tmp_path):
    """空对象：全部字段取默认（host/port 与 Rust 侧一致）。"""
    path = _write_config(tmp_path, {})

    cfg = load_runtime_config(path)

    assert cfg.logging_level == "info"
    assert cfg.logging_dir == ""
    assert cfg.host == "127.0.0.1"
    assert cfg.port == 8000
    assert cfg.mock_mode is False


def test_missing_sections_defaults(tmp_path):
    """缺区段容错：只给 agent 区段，其余取默认。"""
    path = _write_config(tmp_path, {"agent": {"mockMode": True}})

    cfg = load_runtime_config(path)

    assert cfg.mock_mode is True
    assert cfg.host == "127.0.0.1"
    assert cfg.port == 8000


def test_invalid_port_type_reports_dotted_path(tmp_path):
    """类型容错：service.port 非整数 → 错误信息带点路径。"""
    path = _write_config(tmp_path, {"service": {"port": "abc"}})

    with pytest.raises(ConfigParseError) as excinfo:
        load_runtime_config(path)
    assert "service.port" in str(excinfo.value)


def test_invalid_field_type_reports_dotted_path(tmp_path):
    """类型容错：logging.level 非字符串 → 错误信息带点路径。"""
    path = _write_config(tmp_path, {"logging": {"level": 3}})

    with pytest.raises(ConfigParseError) as excinfo:
        load_runtime_config(path)
    assert "logging.level" in str(excinfo.value)


def test_invalid_root_type(tmp_path):
    """根不是 JSON 对象 → 解析错误。"""
    path = _write_config(tmp_path, [1, 2])

    with pytest.raises(ConfigParseError):
        load_runtime_config(path)


def test_invalid_json_decode_error(tmp_path):
    """文件不是合法 JSON → 解析错误（非崩溃栈裸奔）。"""
    path = tmp_path / "config.json"
    path.write_text("{ not json", encoding="utf-8")

    with pytest.raises(ConfigParseError):
        load_runtime_config(path)


def test_missing_file_raises_not_found(tmp_path, monkeypatch):
    """全候选落空 → ConfigNotFoundError（候选含显式路径；与 Rust 同语义可回落）。"""
    # cwd 无 marker 也无 config.json：显式路径不命中后无其余候选可用
    monkeypatch.chdir(tmp_path)
    explicit = tmp_path / "missing.json"

    with pytest.raises(ConfigNotFoundError) as excinfo:
        load_runtime_config(explicit)
    message = str(excinfo.value)
    assert str(explicit) in message
    assert str(RUNTIME_DATA_DIR / "config.json") in message


# ---------------------------------------------------------------------------
# CLI 前置扫描：extract_cli_config_path
# ---------------------------------------------------------------------------


def test_cli_scan_space_form():
    """--config <路径>（空格分隔，Rust sidecar 标准形式）。"""
    argv = ["agent_entry.py", "--port", "9000", "--config", "C:/x/config.json"]
    assert extract_cli_config_path(argv) == Path("C:/x/config.json")


def test_cli_scan_equals_form():
    """--config=<路径>（等号形式兼容）。"""
    argv = ["uvicorn", "app.main:app", "--config=C:/Program Files/Praxis/config.json"]
    assert extract_cli_config_path(argv) == Path("C:/Program Files/Praxis/config.json")


def test_cli_scan_absent():
    """无 --config → None（走默认查找）。"""
    assert extract_cli_config_path(["python", "-m", "uvicorn"]) is None


def test_cli_scan_empty_value_is_unspecified():
    """审查 F5 回归：--config='' / --config= 空值视为未指定（与 Rust 同构）。"""
    assert extract_cli_config_path(["x", "--config", ""]) is None
    assert extract_cli_config_path(["x", "--config="]) is None
    # 首中即决语义（Rust config_cli_override 同构）：首个 --config 命中空值
    # 即判未指定返回 None，不再向后扫描
    assert extract_cli_config_path(["x", "--config", "", "--config=C:/y.json"]) is None


# ---------------------------------------------------------------------------
# 查找顺序
# ---------------------------------------------------------------------------


def test_explicit_path_wins_over_workspace(tmp_path, monkeypatch):
    """显式 --config 优先于 workspace marker 根的 config.json。"""
    workspace = tmp_path / "ws"
    (workspace / "src-tauri").mkdir(parents=True)
    (workspace / "src-tauri" / "tauri.conf.json").write_text("{}", encoding="utf-8")
    (workspace / "config.json").write_text("{}", encoding="utf-8")
    explicit = _write_config(tmp_path, {"service": {"port": 9101}})
    monkeypatch.chdir(workspace)

    cfg = load_runtime_config(explicit)

    assert cfg.port == 9101


def test_workspace_marker_found_by_walk_up(tmp_path, monkeypatch):
    """workspace marker：从 cwd（含子目录）向上找到含 src-tauri 标记的根。"""
    workspace = tmp_path / "ws"
    (workspace / "src-tauri").mkdir(parents=True)
    (workspace / "src-tauri" / "tauri.conf.json").write_text("{}", encoding="utf-8")
    (workspace / "config.json").write_text(
        json.dumps({"service": {"port": 9102}}), encoding="utf-8"
    )
    nested = workspace / "agent-service" / "deep"
    nested.mkdir(parents=True)
    monkeypatch.chdir(nested)

    cfg = load_runtime_config(None)

    assert cfg.port == 9102


def test_not_found_lists_candidates_and_mentions_template(tmp_path, monkeypatch):
    """找不到：ConfigNotFoundError 信息含候选路径与模板引导。"""
    monkeypatch.chdir(tmp_path)  # 无 marker、无 config.json

    with pytest.raises(ConfigNotFoundError) as excinfo:
        load_runtime_config(None)
    message = str(excinfo.value)
    assert str(RUNTIME_DATA_DIR / "config.json") in message
    assert "config.example.json" in message


def test_bootstrap_exit_2_when_not_found(tmp_path, monkeypatch, capsys):
    """bootstrap：找不到 config.json → 打印候选并 sys.exit(2)。"""
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sys, "argv", ["app.main"])

    with pytest.raises(SystemExit) as excinfo:
        bootstrap_runtime(sys.argv)
    assert excinfo.value.code == 2
    assert "config.example.json" in capsys.readouterr().err


def test_bootstrap_exit_2_with_null_streams(tmp_path, monkeypatch):
    """审查 F2 回归：PyInstaller --noconsole 下标准流为 None，

    bootstrap 仍须保持 exit(2) 契约且不得 AttributeError（打印
    静默化由 Rust 壳弹窗兑底）。
    """
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sys, "argv", ["app.main"])
    monkeypatch.setattr(sys, "stderr", None)
    monkeypatch.setattr(sys, "stdout", None)

    with pytest.raises(SystemExit) as excinfo:
        bootstrap_runtime(sys.argv)
    assert excinfo.value.code == 2


# ---------------------------------------------------------------------------
# bootstrap 应用：service/agent 区段写入 settings 单例
# ---------------------------------------------------------------------------


@pytest.fixture
def restore_settings_and_runtime():
    """bootstrap 会就地改写 settings/runtime_config 单例：测试后恢复默认。"""
    snapshot = (settings.host, settings.port, settings.allow_mock_llm)
    yield
    settings.host, settings.port, settings.allow_mock_llm = snapshot
    config_module.runtime_config = config_module.PraxisRuntimeConfig()


def test_bootstrap_applies_service_and_agent(
    tmp_path, monkeypatch, restore_settings_and_runtime
):
    """bootstrap：config 的 service.port / agent.mockMode 应用到 settings 单例。"""
    path = _write_config(
        tmp_path,
        {"service": {"port": 9103}, "agent": {"mockMode": True}},
    )
    monkeypatch.setattr(sys, "argv", ["agent_entry.py", "--config", str(path)])

    bootstrap_runtime(sys.argv)

    assert settings.port == 9103
    assert settings.host == "127.0.0.1"
    assert settings.allow_mock_llm is True


def test_runtime_config_exposed_for_logging(tmp_path, restore_settings_and_runtime):
    """bootstrap 后 runtime_config 全局可读（logging_config 的 level/dir 数据源）。"""
    path = _write_config(tmp_path, {"logging": {"level": "warning", "dir": "D:/logs"}})

    cfg = bootstrap_runtime(["agent_entry.py", "--config", str(path)])

    assert config_module.runtime_config is cfg
    assert cfg.logging_level == "warning"
    assert cfg.logging_dir == "D:/logs"


# ---------------------------------------------------------------------------
# logging.dir → resolve_log_dir
# ---------------------------------------------------------------------------


def test_resolve_log_dir_uses_config_override(tmp_path, restore_settings_and_runtime):
    """logging.dir 非空 → 直接作为日志目录（不再读 PRAXIS_LOG_DIR 等环境变量）。"""
    config_module.runtime_config = config_module.PraxisRuntimeConfig(
        logging_dir=str(tmp_path / "custom-logs")
    )

    assert resolve_log_dir() == tmp_path / "custom-logs"


def test_resolve_log_dir_default_derivation(restore_settings_and_runtime):
    """logging.dir 空 → 运行数据目录常量下的 logs/（默认推导，不读 OS 约定变量）。"""
    config_module.runtime_config = config_module.PraxisRuntimeConfig()

    assert resolve_log_dir() == RUNTIME_DATA_DIR / "logs"


# ---------------------------------------------------------------------------
# LLM 冷启动退役语义：无 Key 且未开 mock → 生成接口明确报错引导设置页
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_outline_no_key_hint_mentions_settings_page(monkeypatch):
    """未配置 Key 且未开 mock：业务错误信息引导到应用设置页（不再提 .env）。"""
    from app import nodes
    from app.errors import AgentServiceError

    monkeypatch.setattr(settings, "allow_mock_llm", False)
    monkeypatch.setattr(nodes, "has_llm_config", lambda: False)

    with pytest.raises(AgentServiceError) as excinfo:
        await nodes.outline_node(
            {
                # 合法请求（哑数据）：节点先取请求再判 mock/Key，形状须过 PlanRequest 校验
                "request": {
                    "goal": "全栈 Web 开发",
                    "level": "beginner",
                    "weeklyHours": 10,
                    "weeks": 12,
                    "interests": [],
                    "background": "",
                }
            }
        )
    message = str(excinfo.value)
    assert "设置页" in message
    assert ".env" not in message
    assert "PRAXIS_ALLOW_MOCK_LLM" not in message


def test_settings_public_dict_unchanged_by_config_load():
    """config 装载不触碰 LLM 十二字段：public_dict 仍是设置页协议形状。"""
    public = settings.public_dict()

    assert set(public) >= {"llmProvider", "allowMockLlm", "openaiApiKey"}
    assert public["allowMockLlm"] == settings.allow_mock_llm
