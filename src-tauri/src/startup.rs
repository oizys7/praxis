//! 启动序列：把「连云端库 + 起 Agent 服务」显式化，供前端展示启动进度。
//!
//! 背景（2026-10-22 实机反馈）：连接池是惰性创建的，首个 DB 命令才触发建池
//! 与全套 DDL，远端 PG 上耗时 12.8 秒。这段时间里用户点任何页面都只看到
//! 转圈，观感像「应用坏了」。改成启动时主动预热，并把真实阶段与耗时暴露
//! 给前端，等待过程有明确文案可看。
//!
//! 就绪语义：无论成功还是降级（DB 未配置/连不上、Agent 起不来）都置
//! ready=true——数据库故障不该把整个桌面应用锁死在启动页；降级原因随
//! 状态一起交给前端提示。

use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tracing::{info, warn};

/// 启动状态快照（前端据此渲染时间轴；阶段边界固定，阶段内不假装精确）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    /// 是否已可进入主界面（成功或降级都算就绪）
    pub ready: bool,
    /// 阶段标识：database | agent | ready
    pub phase: String,
    /// 面向用户的中文文案
    pub message: String,
    /// 0-100 估算进度
    pub progress: u8,
    /// 降级原因；非空表示部分功能不可用
    pub degraded: Option<String>,
    /// 启动耗时（毫秒），ready 后有意义
    pub elapsed_ms: u64,
}

static STATUS: Mutex<StartupStatus> = Mutex::new(StartupStatus {
    ready: false,
    phase: String::new(),
    message: String::new(),
    progress: 0,
    degraded: None,
    elapsed_ms: 0,
});

/// 取锁并读改写；锁中毒（持锁线程 panic）时用内部值继续，不因状态锁拖垮启动。
fn with_lock<R>(f: impl FnOnce(&mut StartupStatus) -> R) -> R {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 当前启动状态（供 startup_status 命令返回）。
pub fn status() -> StartupStatus {
    with_lock(|status| status.clone())
}

fn set(phase: &str, message: &str, progress: u8) {
    with_lock(|status| {
        status.phase = phase.to_string();
        status.message = message.to_string();
        status.progress = progress;
    });
}

fn finish(degraded: Option<String>, elapsed_ms: u64) {
    with_lock(|status| {
        status.ready = true;
        status.phase = "ready".to_string();
        status.progress = 100;
        status.message = match degraded {
            Some(_) => "已进入应用（部分功能不可用）".to_string(),
            None => "准备就绪".to_string(),
        };
        status.degraded = degraded;
        status.elapsed_ms = elapsed_ms;
    });
    info!(elapsed_ms, "启动序列完成");
}

/// 执行启动序列（setup 中 spawn，不阻塞窗口创建）。
///
/// 顺序固定：先连库（慢，10s+）再起 Agent（sidecar 冷启动约数秒），
/// 因为库就绪前任何历史/日历命令都不可用，而 Agent 只影响生成与健康检查。
pub async fn run(app: tauri::AppHandle) {
    let started = Instant::now();
    // 配置在 main() 阶段已装载完成：这里只作为进度轴的第一格，让用户立刻
    // 看到「已经在推进」，同时给三段式时间轴一个完整的起点语义。
    set("config", "配置已载入", 10);
    set("database", "正在连接云端数据库…", 20);

    let mut degraded: Option<String> = None;
    let config = crate::commands::load_db_config();
    if config.host.trim().is_empty() {
        degraded = Some("云端数据库未配置：历史计划与打卡日历暂不可用".to_string());
        set("database", "云端数据库未配置", 55);
    } else {
        match crate::db::pool(&config).await {
            Ok(pool) => {
                set("database", "云端数据库已连接", 60);
                // 启动自愈（幂等）：取出 failed/generating 的计划，按
                // chapters（正文）→ phases → plans 自下而上反推整棵状态树。
                match crate::plan_store::sweep_stale_generating(&pool).await {
                    Ok(report) if !report.is_clean() => {
                        info!(
                            scanned_plans = report.scanned_plans,
                            healed_plans = report.healed_plans,
                            healed_children = report.healed_children,
                            "启动自愈完成：可疑计划状态树已按叶子数据反推"
                        )
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "启动自愈执行失败"),
                }
            }
            Err(e) => {
                warn!(error = %e, "云端数据库连接失败，降级启动");
                degraded = Some(format!("云端数据库连接失败：{e}"));
                set("database", "云端数据库连接失败", 60);
            }
        }
    }

    set("agent", "正在启动 Agent 服务…", 78);
    match crate::sidecar::ensure_agent_running(&app).await {
        Ok(()) => set("agent", "Agent 服务已就绪", 92),
        Err(e) => {
            warn!(error = %e, "Agent 服务启动失败，降级启动");
            degraded = Some(match degraded {
                Some(prev) => format!("{prev}；Agent 服务启动失败：{e}"),
                None => format!("Agent 服务启动失败：{e}"),
            });
        }
    }

    finish(degraded, started.elapsed().as_millis() as u64);
}
