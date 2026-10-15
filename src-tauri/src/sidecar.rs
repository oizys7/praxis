//! 内置 Agent sidecar 的生命周期管理（release 构建生效；debug 由 scripts/dev.mjs 管理）。
//!
//! - 动态端口：向系统申请 127.0.0.1 随机空闲端口，避免固定端口冲突与防火墙提示；
//! - 子进程引导（任务 28）：端口与 config 路径经 `--port N --config <路径>` 参数传递
//!   （agent_entry 承接在任务 29；任务 28–29 联调窗口内 dev 侧 Python 需手动带 --port 起服务）；
//! - 冷启动配置：settings.json 的 LLM 配置经环境变量注入子进程（空值跳过，
//!   避免覆盖 dev 环境 .env 中已有的配置）；
//! - 就绪探测：轮询 /health 直到服务可响应；应用退出时收割子进程。

use std::{process::Stdio, sync::OnceLock, time::Duration};

use tauri::Manager;
use tokio::{process::Child, sync::Mutex};
use tracing::{debug, error, info};

use crate::agent_client::{http_client, DEFAULT_SERVICE_URL, PING_TIMEOUT};
use crate::error::AgentError;

/// sidecar 子进程句柄：release 模式由 ensure_agent_running 拉起，应用退出时收割
static AGENT_CHILD: Mutex<Option<Child>> = Mutex::const_new(None);
/// sidecar 实际监听的动态端口（http://127.0.0.1:{port}）
static AGENT_PORT: OnceLock<u16> = OnceLock::new();

/// 读取 Agent 服务地址（任务 28 弃用 AGENT_SERVICE_URL 环境变量）：
/// - release：优先使用 sidecar 动态端口（每次启动随机分配，避免端口冲突）
/// - 开发：config.json service.host/port 推导（缺省即本地 8000，scripts/dev.mjs 启动的服务；
///   联调窗口内 dev 侧未传 --port 时与此默认值一致）
pub fn agent_service_url() -> String {
    if !cfg!(debug_assertions) {
        if let Some(port) = AGENT_PORT.get() {
            return format!("http://127.0.0.1:{port}");
        }
    }
    match crate::app_config::global().map(|(_, config)| &config.service) {
        Some(service) => format!("http://{}:{}", service.host, service.port),
        None => DEFAULT_SERVICE_URL.to_string(),
    }
}

/// 确保内置 Agent 服务已启动且就绪。
///
/// - debug 构建：不做任何事，服务由 scripts/dev.mjs（或手动 uvicorn）管理
/// - release 构建：拉起安装目录 agent/praxis-agent.exe，
///   用 127.0.0.1 随机空闲端口避免冲突，经 --port/--config 参数传给子进程
///   （agent_entry 承接在任务 29，正式联调同轮）
pub async fn ensure_agent_running(app: &tauri::AppHandle) -> Result<(), AgentError> {
    if cfg!(debug_assertions) {
        debug!("debug 构建：跳过 sidecar 管理（服务由 scripts/dev.mjs 管理）");
        return Ok(());
    }

    // 已有存活的子进程则直接复用；已退出/状态未知则清理句柄后重新拉起
    {
        let mut guard = AGENT_CHILD.lock().await;
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                _ => *guard = None,
            }
        }
    }

    // 向系统申请一个随机空闲端口（绑定后立即释放，让给子进程绑定）
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AgentError::Sidecar(format!("无法分配本地端口：{e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AgentError::Sidecar(format!("无法读取本地端口：{e}")))?
        .port();
    drop(listener);

    // 定位随应用分发的 sidecar 可执行文件
    let sidecar = std::env::current_exe()
        .map_err(|e| AgentError::Sidecar(format!("无法定位应用目录：{e}")))?
        .parent()
        .ok_or_else(|| AgentError::Sidecar("无法定位应用目录".into()))?
        .join("agent")
        .join("praxis-agent.exe");
    if !sidecar.exists() {
        return Err(AgentError::Sidecar(format!(
            "未找到内置 Agent 组件：{}",
            sidecar.display()
        )));
    }

    let started = std::time::Instant::now();
    // 冷启动即生效：把 settings.json 的 LLM 配置注入子进程环境变量
    let app_settings = crate::settings::load_settings(app);
    let mut cmd = tokio::process::Command::new(&sidecar);
    // 端口与 config 路径经命令行参数传递（任务 28；agent_entry 承接在任务 29）。
    // arg 逐项传路径：含空格安全（OS 参数边界由 std 处理，不拼字符串）。
    // 路径来自全局缓存（main() 装载成功的 config.json）；未装载则不传，
    // 由 agent_entry 自己沿查找顺序查找（两边查找到的是同一份）。
    cmd.arg("--port").arg(port.to_string());
    if let Some((config_path, _)) = crate::app_config::global() {
        cmd.arg("--config").arg(config_path);
    }
    // 日志目录统一由 Tauri 管理（app_log_dir），经环境变量对齐 Python 端写同一目录
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    cmd.envs(app_settings.sidecar_envs())
        .env("PRAXIS_LOG_DIR", log_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // CREATE_NO_WINDOW：防御性禁止为子进程分配终端窗口
    // （sidecar 已用 --noconsole 打包为 GUI 子系统，此标志兜底任何残留控制台行为）
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    info!(port, sidecar = %sidecar.display(), "拉起内置 Agent 子进程");
    let child = cmd.spawn().map_err(|e| {
        error!(port, error = %e, "内置 Agent 启动失败");
        AgentError::Sidecar(format!("启动内置 Agent 失败：{e}"))
    })?;
    *AGENT_CHILD.lock().await = Some(child);
    let _ = AGENT_PORT.set(port);

    // 轮询 /health 直到就绪（PyInstaller onedir 冷启动约 1~3 秒）
    let url = format!("http://127.0.0.1:{port}/health");
    let client = http_client();
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Ok(response) = client.get(&url).timeout(PING_TIMEOUT).send().await {
            if response.status().is_success() {
                info!(
                    port,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "内置 Agent 已就绪"
                );
                return Ok(());
            }
        }
    }
    error!(port, "内置 Agent 启动超时（10 秒内 /health 未就绪）");
    Err(AgentError::Sidecar("内置 Agent 服务启动超时".into()))
}

/// 应用退出时收割 sidecar 子进程（同步上下文调用：main 的 RunEvent 回调）
pub fn shutdown_agent() {
    if let Ok(mut guard) = AGENT_CHILD.try_lock() {
        if let Some(child) = guard.as_mut() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未装载 config.json（测试进程不 set 全局缓存）→ 回退缺省本地 8000。
    /// 全局缓存是进程级：任何未来把它 set 的测试都会影响本用例，
    /// 因此用例自身不得 store_global_config（进程隔离不可能，只能靠约定）。
    #[test]
    fn no_global_config_falls_back_to_default_url() {
        // 如果未来某测试装载了全局缓存，本用例会在 setup 阶段崩（改名前先看此注释）
        if crate::app_config::global().is_some() {
            return; // 环境依赖：全局缓存已被别的用例占用，跳过以避免误报
        }
        assert_eq!(
            agent_service_url(),
            "http://127.0.0.1:8000",
            "无配置应回退 DEFAULT_SERVICE_URL（127.0.0.1:8000）"
        );
    }
}
