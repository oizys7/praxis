"""PyInstaller 打包入口：sidecar 生产模式的启动引导。

与开发模式（python -m app.main，带 --reload）的区别：
- 不带 reload（生产子进程由 Rust 桌面壳管理生命周期）；
- uvicorn.run 直接传 app 对象而非 "app.main:app" 字符串
  （PyInstaller 打包后模块布局变化，import 字符串不可靠）。

动态端口约定：Rust 壳拉起本进程前设置 AGENT_SERVICE_PORT 环境变量，
settings.port 会读取它；未设置时回退默认 8000（与开发模式一致）。

静默运行约定：打包使用 --noconsole（不给子进程分配终端窗口），此时
sys.stdout / sys.stderr 为 None——日志统一走 logging 文件（setup_logging），
残留的 print / 第三方 stderr 输出重定向到 os.devnull，避免写入 None 崩溃。
"""

import os
import sys

import uvicorn

from app.config import settings
from app.logging_config import setup_logging, uvicorn_log_config
from app.main import app


def _setup_silent_output() -> None:
    """无控制台模式下把 stdout/stderr 指向 devnull（真正的日志走 logging 文件）。"""
    if sys.stdout is not None and sys.stderr is not None:
        return  # 带控制台运行（开发直跑），保持原样
    try:
        devnull = open(os.devnull, "w", encoding="utf-8")  # noqa: SIM115 —— 进程生命周期持有
    except OSError:
        return  # 极端环境无法打开 devnull：保持 None，让失败显式暴露
    sys.stdout = devnull
    sys.stderr = devnull


def main() -> None:
    """以生产参数启动 FastAPI 应用。"""
    _setup_silent_output()
    setup_logging()
    uvicorn.run(
        app,
        host=settings.host,
        port=settings.port,
        access_log=True,
        log_config=uvicorn_log_config(),
    )


if __name__ == "__main__":
    main()
