//! Tauri commands：前端 `invoke()` 的入口层（薄壳）。
//!
//! 职责仅限「参数接收 + 委托 + 结果透传」，不承载细节逻辑：
//! - HTTP 转发与超时策略见 agent_client；
//! - sidecar 生命周期见 sidecar；
//! - 错误归一与序列化见 error；数据模型见 protocol。
//!
//! 数据结构与以下两处严格对齐（字段均为 camelCase）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use tracing::{debug, error, info, warn};

use crate::agent_client::{new_request_id, send, PING_TIMEOUT, PLAN_TIMEOUT};
use crate::error::AgentError;
use crate::protocol::{AgentErrorDetail, PlanRequest, PlanResponse};
use crate::settings::{load_settings, save_settings_to_disk, AgentSettings};
use crate::sidecar::ensure_agent_running;

/// 生成学习计划：POST /api/generate-plan，透传 Python 的统一包络。
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

    let response = send(
        reqwest::Method::POST,
        "/api/generate-plan",
        PLAN_TIMEOUT,
        Some(&request),
        Some(&req_id),
    )
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

    // 正常情况下业务错误也在 200 的包络里；这里兜底非 2xx 且包络缺 error 的情况
    //（例如反向代理层报错）。
    if !http_status.is_success() && envelope.error.is_none() {
        envelope.status = "error".to_string();
        envelope.error = Some(AgentErrorDetail {
            code: "http".to_string(),
            message: format!("Agent 服务返回 HTTP {http_status}"),
        });
    }

    Ok(envelope)
}

/// 健康检查：GET /health，前端启动时调用以显示在线状态
#[tauri::command]
pub async fn ping_agent(app: tauri::AppHandle) -> Result<bool, AgentError> {
    ensure_agent_running(&app).await?;

    let started = std::time::Instant::now();
    let response = send(
        reqwest::Method::GET,
        "/health",
        PING_TIMEOUT,
        None::<&()>,
        None,
    )
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
    Ok(load_settings(&app))
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
    save_settings_to_disk(&app, &settings).map_err(AgentError::Service)?;
    info!("设置已保存到 settings.json");

    match send(
        reqwest::Method::POST,
        "/api/config",
        PING_TIMEOUT,
        Some(&settings),
        None,
    )
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
