//! 统一日志初始化。
//!
//! 格式契约（与 Python 端 `agent-service/app/logging_config.py` 双向对齐，修改需同步）：
//!
//! ```text
//! <ISO8601 本地时间> [<LEVEL>] [<logger 目标>] <消息> [key=value ...]
//! 2026-10-03T15:04:05.123+08:00 [INFO] [praxis.commands] 内置 Agent 已就绪 port=8123
//! ```
//!
//! - 级别：config.json `logging.level`（任务 28 起弃用 PRAXIS_LOG_LEVEL 环境变量，
//!   默认 info，非法值回退 info；任务 28/29 联调窗口前 Python 仍读旧环境变量）
//! - 文件：`<app_log_dir>/praxis.log`（Tauri app_log_dir 统一管理，Python 端经
//!   PRAXIS_LOG_DIR 环境变量写同一目录）大小轮转——单文件上限 10MB、
//!   保留 3 个滚动备份（.1/.2/.3），总占用上限 40MB
//! - debug 构建额外输出 stderr（scripts/dev.mjs 终端实时可见）；release 仅写文件
//! - 时间戳与 Python `datetime.isoformat(timespec="milliseconds")` 输出逐字符一致

use std::io::Write;
use std::path::PathBuf;

use chrono::SecondsFormat;
use tracing::Event;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// ISO 8601 本地时间（毫秒 + 时区偏移）
fn iso_local_now() -> String {
    chrono::Local::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

/// 统一行格式：`<ts> [<LEVEL>] [<target>] <fields>`
struct PraxisFormat;

impl<S, N> FormatEvent<S, N> for PraxisFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        write!(
            writer,
            "{} [{}] [{}] ",
            iso_local_now(),
            meta.level(),
            meta.target()
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// 初始化全局日志；返回的 WorkerGuard 需由调用方持有到进程结束（异步刷盘线程）。
/// 目录由调用方传入（Tauri `app_log_dir()` 统一管理，Python 端经 PRAXIS_LOG_DIR 对齐，
/// logging.dir 迁移在任务 29 联调）；级别由调用方传 config.json 的 `logging.level`。
pub fn init(dir: &std::path::Path, level: &str) -> WorkerGuard {
    std::fs::create_dir_all(dir).expect("无法创建日志目录");
    let rotating = SizeRotatingWriter::new(dir.to_path_buf(), "praxis.log");
    let (file_writer, guard) = tracing_appender::non_blocking(rotating);

    let file_layer = tracing_subscriber::fmt::layer()
        .event_format(PraxisFormat)
        .with_writer(file_writer)
        .with_ansi(false);

    // debug 构建额外打到 stderr，便于开发终端实时查看
    let stderr_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .event_format(PraxisFormat)
            .with_writer(std::io::stderr)
            .with_ansi(false)
    });

    // 级别由 config.json logging.level 控制（Python 同源），非法值回退 info
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();

    guard
}

// 单文件上限 10MB（用户要求：日志不能无限制增长；与 Python 端 RotatingFileHandler 对齐）
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
// 滚动备份数：总占用上限 = (1 + LOG_BACKUPS) × MAX_LOG_BYTES = 40MB
const LOG_BACKUPS: u32 = 3;

/// 大小轮转写入器：当前文件超过 MAX_LOG_BYTES 后滚动为 .1/.2/.3，
/// 最老的一份被删除——总磁盘占用有界，永不无限增长。
/// 语义与 Python `logging.handlers.RotatingFileHandler`（maxBytes/backupCount）对齐。
struct SizeRotatingWriter {
    dir: PathBuf,
    name: String,
    current: Option<std::fs::File>,
    written: u64,
}

impl SizeRotatingWriter {
    fn new(dir: PathBuf, name: &str) -> Self {
        let mut this = Self {
            dir,
            name: name.to_string(),
            current: None,
            written: 0,
        };
        // 追加模式打开现有文件，并从文件大小恢复计数（重启后不重置、不立刻误轮转）
        this.open_current();
        this
    }

    fn current_path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }

    fn open_current(&mut self) {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.current_path())
            .expect("无法打开日志文件");
        self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.current = Some(file);
    }

    /// 滚动：当前 → .1，.1 → .2，.2 → .3，原 .3 删除
    fn rotate(&mut self) {
        // 先关闭句柄：Windows 不允许重命名打开中的文件
        self.current = None;
        for i in (1..=LOG_BACKUPS).rev() {
            let from = if i == 1 {
                self.current_path()
            } else {
                self.dir.join(format!("{}.{}", self.name, i - 1))
            };
            let to = self.dir.join(format!("{}.{}", self.name, i));
            let _ = std::fs::remove_file(&to); // Windows rename 目标必须不存在
            if from.exists() {
                let _ = std::fs::rename(&from, &to);
            }
        }
        self.open_current();
    }
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.current.is_none() {
            self.open_current();
        }
        if self.written + buf.len() as u64 > MAX_LOG_BYTES {
            self.rotate();
        }
        let n = self
            .current
            .as_mut()
            .expect("日志文件句柄在 write 前已重新打开")
            .write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.current.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}
