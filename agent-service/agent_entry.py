"""Praxis Agent 服务启动引导：sidecar 生产入口 + 开发热重载入口。

两条入口统一经 app.config.bootstrap_runtime(sys.argv) 承接 CLI 参数：
- 生产（PyInstaller onedir）：Rust 壳拉起时传 --port N --config <路径>
  （动态端口与共享配置文件位置），不带 --reload；
- 开发（scripts/dev.mjs）：同样传 --port/--config 并追加 --reload 走
  uvicorn 热重载（此时 uvicorn.run 用导入字符串，子进程会重新导入
  app.main 并按 workspace marker 兜底装载 config.json）。

端口取值链：CLI --port 优先（sidecar 动态端口语义），否则用
config.json service.port（缺省 8000，与开发模式一致）。

静默运行约定：打包使用 --noconsole（不给子进程分配终端窗口），此时
sys.stdout / sys.stderr 为 None——日志统一走 logging 文件（setup_logging），
残留的 print / 第三方 stderr 输出重定向到 os.devnull，避免写入 None 崩溃。
"""

import argparse
import os
import sys
from pathlib import Path

import uvicorn
from app.config import settings
from app.logging_config import setup_logging, uvicorn_log_config
from app.main import app


def _parse_args(argv: list[str]) -> argparse.Namespace:
    """解析启动参数：--port / --config / --reload（config 装载由 bootstrap 统一完成）。"""
    parser = argparse.ArgumentParser(description="Praxis Agent 服务启动引导")
    parser.add_argument(
        "--port",
        type=int,
        default=None,
        help="监听端口；缺省用 config.json 的 service.port（最终缺省 8000）",
    )
    # --config 仅作参数面兼容（argparse 必须认识它才不抛 SystemExit）；
    # 实际装载走 app.config 导入期的 argv 前置扫描（与 uvicorn -m 直跑同一条通道）
    parser.add_argument(
        "--config",
        type=Path,
        default=None,
        help="config.json 路径（由 app.config bootstrap 的 argv 前置扫描装载）",
    )
    parser.add_argument(
        "--reload",
        action="store_true",
        help="开发热重载（scripts/dev.mjs 使用；生产 sidecar 不传）",
    )
    return parser.parse_args(argv)


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
    """启动 FastAPI 应用：CLI --port 优先，config.json 提供 host/port 兜底。"""
    _setup_silent_output()
    args = _parse_args(sys.argv[1:])
    if args.port is not None:
        settings.port = args.port  # CLI 优先（sidecar 动态端口约定）
    setup_logging()
    if args.reload:
        # 开发热重载：必须用导入字符串（reload 子进程重新导入模块）
        uvicorn.run(
            "app.main:app",
            host=settings.host,
            port=settings.port,
            reload=True,
            access_log=True,
            log_config=uvicorn_log_config(),
        )
    else:
        # 生产：uvicorn.run 直接传 app 对象（PyInstaller 打包后模块布局变化，
        # 导入字符串不可靠，见历史注释）
        uvicorn.run(
            app,
            host=settings.host,
            port=settings.port,
            access_log=True,
            log_config=uvicorn_log_config(),
        )


if __name__ == "__main__":
    main()
