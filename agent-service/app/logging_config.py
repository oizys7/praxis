"""统一日志配置。

格式契约（与 Rust 端 `src-tauri/src/logging.rs` 双向对齐，修改需同步两端）：

    <ISO8601 本地时间> [<LEVEL>] [<logger 名>] <消息> [key=value ...]
    2026-10-03T15:04:05.123+08:00 [INFO] [praxis.main] --> POST /api/regenerate/stream req_id=req-...

- 级别：config.json 的 logging.level（双端同一配置文件，默认 INFO）
- 目录：config.json 的 logging.dir；为空时按运行数据目录常量推导
  （RUNTIME_DATA_DIR/logs，与 Rust 端 app_log_dir 同构）
- 轮转：单文件上限 10MB（与 Rust 端 MAX_LOG_BYTES 对齐），保留 3 个滚动备份
  （agent.log.1/2/3），总占用上限 = 4 × 10MB = 40MB
- 输出：日志目录/agent.log；开发（有控制台）额外输出 stderr；打包
  （--noconsole）仅写文件
- 时间戳与 Rust chrono to_rfc3339_opts(Millis) 输出逐字符一致
"""

from __future__ import annotations

import logging
import logging.handlers
import sys
from datetime import datetime
from pathlib import Path

from app import config as _config
from app.config import RUNTIME_DATA_DIR

#: 统一格式模板（时间由 IsoFormatter 生成 ISO 8601，与 Rust 端 PraxisFormat 对齐）
LOG_FORMAT = "%(asctime)s [%(levelname)s] [%(name)s] %(message)s"

#: 单个日志文件上限（字节）：10MB
MAX_LOG_BYTES = 10 * 1024 * 1024

#: 滚动备份数：总占用上限 = (1 + BACKUP_COUNT) × 10MB = 40MB
BACKUP_COUNT = 3


def resolve_log_dir() -> Path:
    """日志目录：config.json 的 logging.dir 优先（相对路径相对启动 cwd 解析）；
    为空时（缺省）落到运行数据目录常量下的 logs/——只按 Path.home() 推导，
    不存在任何环境变量读取（配置文件化的唯一数据源是本文件 + config.json）。

    经 _config 模块属性动态读取：tests 重绑 config.runtime_config 即可
    覆盖取值，不依赖导入期捕获的单例引用。
    """
    configured = _config.runtime_config.logging_dir
    if configured:
        return Path(configured)
    return RUNTIME_DATA_DIR / "logs"


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

    # 显式 level 参数仅保留给调用方覆盖；默认取 config.json 的 logging.level
    level_name = (level or _config.runtime_config.logging_level or "INFO").upper()
    log_dir = resolve_log_dir()
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
