"""统一日志配置。

格式契约（与 Rust 端 `src-tauri/src/logging.rs` 双向对齐，修改需同步两端）：

    <ISO8601 本地时间> [<LEVEL>] [<logger 名>] <消息> [key=value ...]
    2026-10-03T15:04:05.123+08:00 [INFO] [praxis.main] --> POST /api/generate-plan req_id=req-...

- 级别：PRAXIS_LOG_LEVEL 环境变量（双端读同一变量，默认 INFO）
- 轮转：单文件上限 10MB（与 Rust 端 MAX_LOG_BYTES 对齐），保留 3 个滚动备份
  （agent.log.1/2/3），总占用上限 = 4 × 10MB = 40MB
- 输出：%LOCALAPPDATA%/Praxis/logs/agent.log（Linux/macOS ~/.local/share/Praxis/logs/）
- 开发（有控制台）：额外输出 stderr；打包（--noconsole）仅写文件
- 时间戳与 Rust chrono to_rfc3339_opts(Millis) 输出逐字符一致
"""

from __future__ import annotations

import logging
import logging.handlers
import os
import sys
from datetime import datetime
from pathlib import Path

#: 统一格式模板（时间由 IsoFormatter 生成 ISO 8601，与 Rust 端 PraxisFormat 对齐）
LOG_FORMAT = "%(asctime)s [%(levelname)s] [%(name)s] %(message)s"

#: 单个日志文件上限（字节）：10MB
MAX_LOG_BYTES = 10 * 1024 * 1024

#: 滚动备份数：总占用上限 = (1 + BACKUP_COUNT) × 10MB = 40MB
BACKUP_COUNT = 3

#: 日志级别环境变量（与 Rust 端同一变量）
LEVEL_ENV = "PRAXIS_LOG_LEVEL"


def default_log_dir() -> Path:
    """日志目录：release 由 Rust 经 PRAXIS_LOG_DIR 注入（Tauri app_log_dir，双端同目录）；
    未注入时（独立 uvicorn 开发模式）回退到同结构的本机目录。"""
    env_dir = os.getenv("PRAXIS_LOG_DIR")
    if env_dir:
        return Path(env_dir)
    if os.name == "nt":
        base = Path(os.getenv("LOCALAPPDATA") or Path.home() / "AppData/Local")
    else:
        base = Path(os.getenv("XDG_DATA_HOME") or Path.home() / ".local/share")
    return base / "com.praxis.desktop" / "logs"


class IsoFormatter(logging.Formatter):
    """ISO 8601 本地时间（毫秒 + 时区偏移），与 Rust 端输出逐字符一致。"""

    def formatTime(self, record: logging.LogRecord, datefmt: str | None = None) -> str:
        return (
            datetime.fromtimestamp(record.created)
            .astimezone()
            .isoformat(timespec="milliseconds")
        )


def setup_logging(level: str | None = None) -> None:
    """初始化 root logger：文件轮转 +（有控制台时）stderr。

    uvicorn 等第三方 logger 通过 propagate 汇入 root，共享同一格式
    （见 uvicorn_log_config）。重复调用安全（uvicorn --reload 的子进程会
    重新执行模块导入触发本函数）。
    """
    root = logging.getLogger()
    # 防重复挂载：已有轮转文件 handler 视为已初始化
    if any(isinstance(h, logging.handlers.RotatingFileHandler) for h in root.handlers):
        return

    level_name = (level or os.getenv(LEVEL_ENV) or "INFO").upper()
    log_dir = default_log_dir()
    log_dir.mkdir(parents=True, exist_ok=True)

    file_handler = logging.handlers.RotatingFileHandler(
        log_dir / "agent.log",
        maxBytes=MAX_LOG_BYTES,
        backupCount=BACKUP_COUNT,
        encoding="utf-8",
    )
    formatter = IsoFormatter(LOG_FORMAT)
    file_handler.setFormatter(formatter)

    handlers: list[logging.Handler] = [file_handler]
    # --noconsole 打包下 sys.stderr 为 None：只有开发直跑才追加控制台输出
    if sys.stderr is not None:
        console = logging.StreamHandler(sys.stderr)
        console.setFormatter(formatter)
        handlers.append(console)

    root.setLevel(getattr(logging, level_name, logging.INFO))
    root.handlers.extend(handlers)

    logging.getLogger("praxis").info(
        "日志已初始化：level=%s dir=%s", level_name, log_dir
    )


def uvicorn_log_config() -> dict:
    """uvicorn.run 的 log_config：移除其默认控制台 handler，全部向上传播到 root。"""
    return {
        "version": 1,
        "disable_existing_loggers": False,
        "formatters": {},
        "handlers": {},
        "loggers": {
            "uvicorn": {"level": "INFO", "handlers": [], "propagate": True},
            "uvicorn.error": {"level": "INFO", "handlers": [], "propagate": True},
            "uvicorn.access": {"level": "INFO", "handlers": [], "propagate": True},
        },
    }
