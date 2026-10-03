//! Tauri commands：前端 `invoke()` 的入口层。
//!
//! 职责：
//! 1. 反序列化前端传来的请求参数（camelCase JSON -> Rust 结构体）
//! 2. 通过 reqwest 把请求转发给本地 Python FastAPI Agent 服务
//! 3. 把 Python 返回的统一包络（PlanResponse）原样透传给前端
//! 4. 将网络层错误（连接失败 / 超时 / 非法响应）归一化为带提示的错误对象
//!
//! 数据结构与以下两处严格对齐（字段均为 camelCase）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use std::{env, process::Stdio, sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};

use crate::settings::AgentSettings;
use tokio::{process::Child, sync::Mutex};
use tracing::{debug, error, info, warn};

/// Python Agent 服务地址，可用环境变量 AGENT_SERVICE_URL 覆盖（见 agent-service/.env.example）
const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:8000";
/// 生成计划涉及多次 LLM 调用（规划 + 评审 + 可能的一次修订），给足超时时间
const PLAN_TIMEOUT: Duration = Duration::from_secs(600);
/// 健康检查使用短超时，快速探测服务是否在线
const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// sidecar 子进程句柄：release 模式由 ensure_agent_running 拉起，应用退出时收割
static AGENT_CHILD: Mutex<Option<Child>> = Mutex::const_new(None);
/// sidecar 实际监听的动态端口（http://127.0.0.1:{port}）
static AGENT_PORT: OnceLock<u16> = OnceLock::new();

/// 读取 Agent 服务地址：
/// - release：优先使用 sidecar 动态端口（每次启动随机分配，避免端口冲突）
/// - 开发：环境变量 AGENT_SERVICE_URL 优先，缺省本地 8000（scripts/dev.mjs 启动的服务）
fn agent_service_url() -> String {
    if !cfg!(debug_assertions) {
        if let Some(port) = AGENT_PORT.get() {
            return format!("http://127.0.0.1:{port}");
        }
    }
    env::var("AGENT_SERVICE_URL").unwrap_or_else(|_| DEFAULT_SERVICE_URL.to_string())
}

/// 请求关联 ID：随 X-Request-ID 头传给 Python，双端日志可按 id 对齐时间线
fn new_request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("req-{nanos:016x}")
}

/// reqwest 客户端无全局超时，由每个请求单独设置（ping 短超时、plan 长超时）
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// 数据模型（与 agent-service/app/schemas.py、frontend/src/types/agent.ts 对齐）
// ---------------------------------------------------------------------------

/// 学习计划生成请求（前端 -> Rust -> Python）
/// Serialize：转发给 Python 时由 reqwest .json() 序列化；Deserialize：接收 Tauri IPC 参数
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    /// 学习目标
    pub goal: String,
    /// 当前水平：beginner | intermediate | advanced
    pub level: String,
    /// 每周可用小时数
    pub weekly_hours: u32,
    /// 期望总周数
    pub weeks: u32,
    /// 偏好方向（多选标签）
    #[serde(default)]
    pub interests: Vec<String>,
    /// 已有基础描述
    #[serde(default)]
    pub background: String,
}

/// 阶段收尾项目及其验收标准
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PhaseProject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// 学习计划中的一个阶段：理论 / 实操 / 作业 / 项目 四位一体
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StudyPhase {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub week_range: String,
    #[serde(default)]
    pub objectives: Vec<String>,
    #[serde(default)]
    pub theory: Vec<String>,
    #[serde(default)]
    pub practice: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<String>,
    #[serde(default)]
    pub project: PhaseProject,
}

/// 推荐学习资源（type 字段与 Python 端对齐，Rust 侧改名避免关键字冲突）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StudyResource {
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub note: String,
}

/// 完整学习计划
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StudyPlan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub estimated_total_hours: u32,
    #[serde(default)]
    pub phases: Vec<StudyPhase>,
    #[serde(default)]
    pub resources: Vec<StudyResource>,
    #[serde(default)]
    pub review_notes: String,
}

/// 错误详情（Python 端返回的 envelope.error）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorDetail {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// 三端统一的响应包络：status = "plan" | "clarification" | "error"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanResponse {
    pub status: String,
    #[serde(default)]
    pub plan: Option<StudyPlan>,
    /// clarify 节点产出的澄清问题（status = "clarification" 时非空）
    #[serde(default)]
    pub clarifications: Option<Vec<String>>,
    #[serde(default)]
    pub error: Option<AgentErrorDetail>,
}

// ---------------------------------------------------------------------------
// 错误类型：实现 Serialize，前端 invoke catch 到的是 { code, message, hint }
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// reqwest 网络层错误（连接失败 / 超时 / 解码失败）
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    /// Python 服务返回了业务错误
    #[error("{0}")]
    Service(String),
    /// 其他内部错误（当前代码路径暂未构造，保留以便扩展）
    #[error("{0}")]
    #[allow(dead_code)]
    Other(String),
    /// sidecar 生命周期错误（release 模式内置 Agent 子进程的启动/就绪失败）
    #[error("{0}")]
    Sidecar(String),
}

impl Serialize for AgentError {
    /// 序列化为 { code, message, hint }：message 给技术信息，hint 给用户可操作的提示
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let (code, message, hint) = match self {
            AgentError::Http(e) if e.is_connect() => (
                "connection",
                e.to_string(),
                "无法连接 Python Agent 服务。请先启动：cd agent-service && uvicorn app.main:app --port 8000",
            ),
            AgentError::Http(e) if e.is_timeout() => (
                "timeout",
                e.to_string(),
                "请求超时：模型生成耗时较长或服务无响应，请稍后重试",
            ),
            AgentError::Http(e) => (
                "http",
                e.to_string(),
                "Agent 服务返回了无效响应，请查看 agent-service 终端日志",
            ),
            AgentError::Service(msg) => (
                "service",
                msg.clone(),
                "Agent 服务处理失败，请查看 agent-service 终端日志",
            ),
            AgentError::Other(msg) => ("internal", msg.clone(), "Tauri 内部错误"),
            AgentError::Sidecar(msg) => (
                "sidecar",
                msg.clone(),
                "内置 Agent 组件异常：请尝试重新安装 Praxis 桌面应用",
            ),
        };

        let mut s = serializer.serialize_struct("AgentError", 3)?;
        s.serialize_field("code", code)?;
        s.serialize_field("message", &message)?;
        s.serialize_field("hint", hint)?;
        s.end()
    }
}

// ---------------------------------------------------------------------------
// sidecar 生命周期（release 模式）：动态端口 + 拉起内置 Agent + 等待就绪
// ---------------------------------------------------------------------------

/// 确保内置 Agent 服务已启动且就绪。
///
/// - debug 构建：不做任何事，服务由 scripts/dev.mjs（或手动 uvicorn）管理
/// - release 构建：拉起安装目录 agent/praxis-agent.exe，
///   用 127.0.0.1 随机空闲端口避免冲突，经 AGENT_SERVICE_PORT 传给子进程
async fn ensure_agent_running(app: &tauri::AppHandle) -> Result<(), AgentError> {
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
    let sidecar = env::current_exe()
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
    // 冷启动即生效：把 settings.json 的 LLM 配置注入子进程环境变量（空值跳过，
    // 避免覆盖 dev 环境里已有的 .env 配置）
    let app_settings = crate::settings::load_settings(app);
    let mut cmd = tokio::process::Command::new(&sidecar);
    // 日志目录统一由 Tauri 管理（app_log_dir），经环境变量对齐 Python 端写同一目录
    use tauri::Manager;
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    cmd.env("AGENT_SERVICE_PORT", port.to_string())
        .envs(app_settings.sidecar_envs())
        .env("PRAXIS_LOG_DIR", log_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // CREATE_NO_WINDOW：防御性禁止为子进程分配终端窗口
    // （sidecar 已用 --noconsole 打包为 GUI 子系统，此标志兑底任何残留控制台行为）
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

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// 生成学习计划：POST {AGENT_SERVICE_URL}/api/generate-plan
///
/// Python 端约定：无论成功失败，响应体始终是统一包络（PlanResponse），
/// 因此这里解析成功即可整体透传；仅当 HTTP 非 2xx 且包络缺 error 时补一条。
#[tauri::command]
pub async fn generate_plan(
    app: tauri::AppHandle,
    request: PlanRequest,
) -> Result<PlanResponse, AgentError> {
    ensure_agent_running(&app).await?;

    let started = std::time::Instant::now();
    let req_id = new_request_id();
    info!(%req_id, goal = %request.goal, "收到学习计划生成请求");

    let url = format!(
        "{}/api/generate-plan",
        agent_service_url().trim_end_matches('/')
    );

    let response = http_client()
        .post(&url)
        .timeout(PLAN_TIMEOUT)
        .header("X-Request-ID", &req_id)
        .json(&request)
        .send()
        .await
        .map_err(|e| {
            error!(%req_id, error = %e, "学习计划请求发送失败");
            e
        })?;

    let http_status = response.status();
    let mut envelope: PlanResponse = response.json().await.map_err(|e| {
        error!(%req_id, error = %e, "学习计划响应解析失败");
        AgentError::Service(format!("响应不是合法的 PlanResponse JSON：{e}"))
    })?;

    info!(
        %req_id,
        http_status = %http_status,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "学习计划请求完成"
    );

    // 正常情况下 Python 端业务错误也会放在 200 的包络里；
    // 这里兜底处理非 2xx 且包络缺 error 的情况（例如反向代理层报错）。
    if !http_status.is_success() && envelope.error.is_none() {
        envelope.status = "error".to_string();
        envelope.error = Some(AgentErrorDetail {
            code: "http".to_string(),
            message: format!("Agent 服务返回 HTTP {http_status}"),
        });
    }

    Ok(envelope)
}

/// 健康检查：GET {AGENT_SERVICE_URL}/health，前端启动时调用以显示在线状态
#[tauri::command]
pub async fn ping_agent(app: tauri::AppHandle) -> Result<bool, AgentError> {
    ensure_agent_running(&app).await?;

    let url = format!("{}/health", agent_service_url().trim_end_matches('/'));
    let started = std::time::Instant::now();
    let response = http_client()
        .get(&url)
        .timeout(PING_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            warn!(error = %e, "Agent 健康检查失败");
            e
        })?;
    let ok = response.status().is_success();
    debug!(
        ok,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "Agent 健康检查完成"
    );
    Ok(ok)
}

/// 读取应用设置：settings.json（不存在时返回默认值）
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<AgentSettings, AgentError> {
    Ok(crate::settings::load_settings(&app))
}

/// 保存应用设置：写盘持久化 + 热推送给运行中的 Agent 服务（POST /api/config）。
///
/// 推送失败不阻塞保存：release 模式下次拉起 sidecar 时经环境变量注入生效；
/// dev 模式下服务离线时同样静默（服务起来后重新保存即可）。
#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    settings: AgentSettings,
) -> Result<(), AgentError> {
    crate::settings::save_settings_to_disk(&app, &settings).map_err(AgentError::Service)?;
    info!("设置已保存到 settings.json");

    let url = format!("{}/api/config", agent_service_url().trim_end_matches('/'));
    match http_client()
        .post(&url)
        .timeout(PING_TIMEOUT)
        .json(&settings)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            info!("配置已热加载到 Agent 服务");
        }
        Ok(response) => {
            warn!(
                status = %response.status(),
                "配置热加载失败（服务返回非 2xx）；下次 Agent 启动时生效"
            );
        }
        Err(e) => {
            debug!(error = %e, "Agent 服务未运行，配置将在下次启动时生效");
        }
    }
    Ok(())
}
