"""agent_entry.py CLI 引导测试：--port 优先、argparse 兼容 --config/--reload。

config.json 装载本身由 app.config bootstrap（argv 前置扫描）负责并已在
test_config.py 覆盖；本文件只盯入口的端口取值链与参数面。
"""

import sys

import agent_entry as entry
from app.config import settings


def _fake_run(captured: dict):
    def fake_run(*args, **kwargs):
        """记录位置参数与关键字参数（uvicorn.run 两种传法都要能断言）。"""
        captured["positional"] = args
        captured.update(kwargs)

    return fake_run


def test_cli_port_takes_priority_over_settings(monkeypatch):
    """sidecar 动态端口约定：CLI --port 覆盖 config.json 注入的 settings.port。

    settings 是进程级单例：测试后恢复原值，避免污染其他用例。
    """
    captured: dict = {}
    monkeypatch.setattr(entry.uvicorn, "run", _fake_run(captured))
    monkeypatch.setattr(entry.sys, "argv", ["agent_entry.py", "--port", "9105"])
    original_port = settings.port
    try:
        entry.main()
        assert captured["port"] == 9105
        assert captured["host"] == settings.host
    finally:
        settings.port = original_port


def test_port_falls_back_to_config_injected_settings(monkeypatch):
    """不带 --port：uvicorn 用 settings.port（bootstrap 从 config.json 注入）。"""
    captured: dict = {}
    monkeypatch.setattr(entry.uvicorn, "run", _fake_run(captured))
    monkeypatch.setattr(entry.sys, "argv", ["agent_entry.py"])
    original_port = settings.port
    try:
        settings.port = 9106  # 模拟 bootstrap 已注入的 config service.port
        entry.main()
        assert captured["port"] == 9106
    finally:
        settings.port = original_port


def test_argparse_accepts_config_and_reload_flags(monkeypatch):
    """--config / --reload 参数面：argparse 能解析（装载走 bootstrap argv 扫描）。"""
    args = entry._parse_args(["--config", "x/config.json", "--reload"])

    assert args.config is not None
    assert args.reload is True


def test_reload_uses_import_string(monkeypatch):
    """开发热重载：uvicorn.run 以导入字符串启动（reload 子进程重新导入模块）。"""
    captured: dict = {}
    monkeypatch.setattr(entry.uvicorn, "run", _fake_run(captured))
    monkeypatch.setattr(
        entry.sys, "argv", ["agent_entry.py", "--port", "9107", "--reload"]
    )
    original_port = settings.port
    try:
        entry.main()
        assert captured["reload"] is True
        assert captured["positional"][0] == "app.main:app"
    finally:
        settings.port = original_port


def test_sys_argv_unchanged_by_silent_output():
    """sys 模块属性只被读取：入口不改 sys.argv（bootstrap 扫描依赖原始 argv）。"""
    argv_snapshot = list(sys.argv)
    entry._setup_silent_output()
    assert sys.argv == argv_snapshot
