//! Tauri commands：前端 `invoke()` 的入口层。
//!
//! 多数命令职责仅限「参数接收 + 委托 + 结果透传」，不承载细节逻辑：
//! - HTTP 转发与超时策略见 agent_client；
//! - sidecar 生命周期见 sidecar；
//! - 错误归一与序列化见 error；数据模型见 protocol。
//!
//! 例外是 generate_plan：流式消费 + 进度推送 + 自动入库的编排枢纽（见函数文档）。
//!
//! 数据结构与以下两处严格对齐（字段均为 camelCase）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use sqlx::PgPool;
use tauri::ipc::{Channel, JavaScriptChannelId};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::agent_client::{new_request_id, post_stream, send, PING_TIMEOUT, PLAN_TIMEOUT};
use crate::db;
use crate::error::AgentError;
use crate::plan_store;
use crate::protocol::{
    parse_ndjson_line, PlanProgressEvent, PlanRecord, PlanRequest, PlanResponse, PlanSummary,
    SavePlanRequest, StreamEvent,
};
use crate::settings::{load_settings, save_settings_to_disk, AgentSettings};
use crate::sidecar::ensure_agent_running;

/// 读取云端数据库配置（config.yaml）；密码经主密钥解密为内存明文，
/// 手写明文会在加载时自动加密回写（见 config::AppConfig::load_from）
fn load_db_config(app: &tauri::AppHandle) -> crate::config::DbConfig {
    match crate::settings::key_dir(app) {
        Ok(dir) => crate::config::AppConfig::load(&dir).db,
        Err(_) => crate::config::DbConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// 学习计划生成（流式消费 + 进度推送 + 自动入库编排）
// ---------------------------------------------------------------------------

/// 本次生成的云端入库句柄：连接池 + 预置行 id；None 表示本次生成无云端入库
struct Storage {
    pool: PgPool,
    plan_id: Uuid,
}

/// 进度推送：channel 未传（前端旧调用）或推送失败时静默跳过，不影响主流程
fn notify(channel: Option<&Channel<PlanProgressEvent>>, event: PlanProgressEvent) {
    if let Some(ch) = channel {
        if let Err(e) = ch.send(event) {
            warn!(error = %e, "进度事件推送失败");
        }
    }
}

/// 生成学习计划（流式）：POST /api/generate-plan/stream，逐行消费 NDJSON 事件，
/// 每阶段自动入库（plan_store 规范化多表）并经 Channel 向前端推送进度；
/// done 终态的统一包络整体透传作为命令返回值（形状与旧非流式链路一致）。
///
/// 编排约定：
/// - 入库前置：DB 可用则预置 generating 行（终态按 plan / clarification / error 分别
///   落 complete / 删行 / failed）；DB 不可用不阻塞生成，仅经 StorageIssue 事件提示；
/// - 进度推送：channel 为可选参数（前端旧调用不传时优雅降级为不发进度）；
/// - 容错：坏行告警跳过、单阶段入库失败不中断流（存储问题不吞生成结果）；
/// - 异常路径：传输错误 / 超时 / 流提前中断（未收到 done）→ 已入库则收尾 failed + Err。
///
/// channel 形态说明：`tauri::Channel<T>` 无 Deserialize（仅 CommandArg），可选参数
/// 用 `Option<JavaScriptChannelId>`（有 Deserialize，前端 `new Channel()` 传入的
/// `__CHANNEL__ID` 字符串即其字面形式；key 缺失时反序列化为 None）+ webview 注入
/// 还原，语义等同可选 Channel。
#[tauri::command]
pub async fn generate_plan(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    request: PlanRequest,
    channel: Option<JavaScriptChannelId>,
) -> Result<PlanResponse, AgentError> {
    ensure_agent_running(&app).await?;

    let started = std::time::Instant::now();
    let req_id = new_request_id();
    info!(%req_id, goal = %request.goal, "收到学习计划生成请求（流式）");

    // 前端传了 Channel 则还原为进度推送通道；旧调用不传则优雅降级为不发进度
    let channel = channel.map(|id| id.channel_on(webview));
    let progress = channel.as_ref();

    // 1. 入库前置：DB 可用则预置 generating 行；不可用只提示不阻塞主流程
    let config = load_db_config(&app);
    let storage = match db::pool(&config).await {
        Ok(pool) => {
            let plan_id = Uuid::new_v4();
            match plan_store::create_generating_plan(&pool, plan_id, &request).await {
                Ok(()) => Some(Storage { pool, plan_id }),
                Err(e) => {
                    warn!(%req_id, error = %e, "预置生成中计划行失败，本次生成不入库");
                    notify(
                        progress,
                        PlanProgressEvent::StorageIssue {
                            message: format!("云端库写入失败，本次生成不会自动保存：{e}"),
                        },
                    );
                    None
                }
            }
        }
        Err(e) => {
            warn!(%req_id, error = %e, "云端库不可用，本次生成不入库");
            let message = if config.host.trim().is_empty() {
                "云端库未配置，本次生成未自动保存".to_string()
            } else {
                format!("云端库连接失败，本次生成未自动保存：{e}")
            };
            notify(progress, PlanProgressEvent::StorageIssue { message });
            None
        }
    };

    // 2. 流消费：总超时（PLAN_TIMEOUT）包住整个消费过程（含逐阶段入库）；
    //    reqwest 请求侧同值总超时兜底（覆盖到响应体读完）
    let consumed = match post_stream(
        "/api/generate-plan/stream",
        &request,
        PLAN_TIMEOUT,
        Some(&req_id),
    )
    .await
    {
        Ok(mut response) => {
            match tokio::time::timeout(PLAN_TIMEOUT, async {
                // 流式端点前置校验失败（如 422 参数校验）：响应体不是 NDJSON，归一为错误
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let snippet: String = body.chars().take(200).collect();
                    warn!(%req_id, http_status = %status, body = %snippet, "流式生成端点返回非 2xx");
                    return Err(AgentError::Service(format!("Agent 服务返回 HTTP {status}")));
                }

                let mut buffer: Vec<u8> = Vec::new();
                let mut phase_count: u32 = 0;
                while let Some(line) = next_line(&mut response, &mut buffer).await? {
                    if line.trim().is_empty() {
                        continue; // 空行跳过
                    }
                    let Some(event) = parse_ndjson_line(&line) else {
                        warn!(%req_id, "流中出现无法解析的事件行，已跳过");
                        continue; // 坏行不中断流
                    };
                    match event {
                        StreamEvent::Outline { phase_count: count, title, overview, phases } => {
                            phase_count = count;
                            if let Some(s) = &storage {
                                // 预计总学时：大纲阶段数 × 每周学时（粗估，供云端列表页展示）
                                let hours =
                                    i64::from(request.weekly_hours).saturating_mul(i64::from(count));
                                if let Err(e) = plan_store::update_outline_meta(
                                    &s.pool,
                                    s.plan_id,
                                    &title,
                                    hours,
                                )
                                .await
                                {
                                    warn!(%req_id, error = %e, "计划大纲元信息入库失败，不影响生成");
                                    notify(
                                        progress,
                                        PlanProgressEvent::StorageIssue {
                                            message: format!("计划大纲未能自动保存：{e}"),
                                        },
                                    );
                                }
                                // 大纲骨架播种（幂等：已有阶段行则跳过；旧 Python 载荷
                                // 无 phases 时为 None，静默不播种）。骨架播种失败不中断生成。
                                if let Some(outline) = &phases {
                                    if let Err(e) =
                                        plan_store::seed_outline(&s.pool, s.plan_id, outline, &[]).await
                                    {
                                        warn!(%req_id, error = %e, "大纲骨架播种失败，不影响生成");
                                        notify(
                                            progress,
                                            PlanProgressEvent::StorageIssue {
                                                message: format!("大纲骨架未能自动保存：{e}"),
                                            },
                                        );
                                    }
                                }
                            }
                            notify(
                                progress,
                                PlanProgressEvent::Outline { phase_count: count, title, overview, phases },
                            );
                        }
                        StreamEvent::Phase { index, phase } => {
                            if let Some(s) = &storage {
                                // Python 端 index 为小非负整数；防御性越界按 0 处理（实际不可达）
                                let idx = i32::try_from(index).unwrap_or(0);
                                if let Err(e) =
                                    plan_store::upsert_phase(&s.pool, s.plan_id, idx, &phase).await
                                {
                                    warn!(%req_id, index, error = %e, "阶段入库失败，不中断生成流");
                                    notify(
                                        progress,
                                        PlanProgressEvent::StorageIssue {
                                            message: format!(
                                                "第 {} 阶段未能自动保存：{e}",
                                                index.saturating_add(1)
                                            ),
                                        },
                                    );
                                }
                            }
                            notify(
                                progress,
                                PlanProgressEvent::PhaseDone { index, total: phase_count },
                            );
                        }
                        StreamEvent::Done { response: envelope } => {
                            if let Some(s) = &storage {
                                if let Err(e) =
                                    settle_terminal(&s.pool, s.plan_id, &envelope.status).await
                                {
                                    warn!(%req_id, error = %e, "终态入库收尾失败，不影响生成结果返回");
                                    notify(
                                        progress,
                                        PlanProgressEvent::StorageIssue {
                                            message: format!("生成结果未能按终态入库：{e}"),
                                        },
                                    );
                                }
                            }
                            info!(
                                %req_id,
                                status = %envelope.status,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "学习计划流式生成完成"
                            );
                            return Ok(Some(envelope));
                        }
                    }
                }
                Ok(None)
            })
            .await
            {
                Ok(inner) => inner,
                Err(_) => {
                    error!(%req_id, timeout_secs = PLAN_TIMEOUT.as_secs(), "学习计划流式消费总超时");
                    Err(AgentError::Service(format!(
                        "生成计划超时（{} 秒），请稍后重试",
                        PLAN_TIMEOUT.as_secs()
                    )))
                }
            }
        }
        Err(e) => {
            error!(%req_id, error = %e, "流式生成请求发送失败");
            Err(e)
        }
    };

    // 3. 终态分派结果：done 包络整体透传；未收到 done（传输错误/超时/提前 EOF）→ Err
    let result = match consumed {
        Ok(Some(envelope)) => Ok(envelope),
        Ok(None) => {
            error!(%req_id, "生成流提前结束：未收到 done 终态事件");
            Err(AgentError::Service(
                "生成流提前中断，未收到最终结果，请重试".into(),
            ))
        }
        Err(e) => Err(e),
    };

    // 4. 异常路径收尾：已入库则标记 failed（done 三态已在流内收尾，不受此影响）
    if result.is_err() {
        if let Some(s) = &storage {
            if let Err(e) =
                plan_store::finalize_plan(&s.pool, s.plan_id, "failed", None, None).await
            {
                warn!(%req_id, error = %e, "标记生成失败状态失败");
            }
        }
    }
    result
}

/// done 终态对应的入库收尾：plan -> complete；clarification -> 删除 generating 行；
/// error（及防御性未知状态）-> failed，避免行滞留 generating。
async fn settle_terminal(pool: &PgPool, plan_id: Uuid, status: &str) -> Result<(), AgentError> {
    match status {
        "plan" => plan_store::finalize_plan(pool, plan_id, "complete", None, None).await,
        "clarification" => plan_store::delete_plan_full(pool, plan_id).await,
        other => {
            if other != "error" {
                warn!(status = other, "未知 done 终态状态，按 failed 收尾");
            }
            plan_store::finalize_plan(pool, plan_id, "failed", None, None).await
        }
    }
}

/// 行字节解码：UTF-8 非法字节替换为 U+FFFD；行尾 \r\n / \n 去除（NDJSON 按 \n 分帧，\r 兼容 Windows）
fn decode_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

/// 从字节缓冲取出首个完整行（\n 结尾分帧）；无完整行返回 None，缓冲保持原样等待后续数据。
/// 粘包：多行一次到达时逐次取出；尾行无换行结尾时留在缓冲（由流结束残行逻辑兜底）。
fn take_line(buffer: &mut Vec<u8>) -> Option<String> {
    let pos = buffer.iter().position(|&b| b == b'\n')?;
    let line: Vec<u8> = buffer.drain(..=pos).collect();
    Some(decode_line(&line))
}

/// 从 NDJSON 流读取下一行：优先取缓冲完整行；不足则继续读响应体 chunk；
/// 流结束时把缓冲残留（最后一行可能无换行结尾）作为行返回，此后返回 None。
async fn next_line(
    response: &mut reqwest::Response,
    buffer: &mut Vec<u8>,
) -> Result<Option<String>, AgentError> {
    loop {
        if let Some(line) = take_line(buffer) {
            return Ok(Some(line));
        }
        match response.chunk().await? {
            Some(chunk) => buffer.extend_from_slice(&chunk),
            // 流结束：残行返回（缓冲为空则 None）
            None => {
                let rest = std::mem::take(buffer);
                return Ok((!rest.is_empty()).then(|| decode_line(&rest)));
            }
        }
    }
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

// ---------------------------------------------------------------------------
// 云端数据库（学习计划历史）：db.rs 的薄壳，每次从 config.yaml 读取连接配置
// ---------------------------------------------------------------------------

/// 保存学习计划到云端（新记录），返回生成的计划 id
#[tauri::command]
pub async fn save_plan(
    app: tauri::AppHandle,
    record: SavePlanRequest,
) -> Result<String, AgentError> {
    let config = load_db_config(&app);
    db::save_plan(&config, &record).await
}

/// 计划列表（不含整份计划）：收藏置顶 + 创建时间倒序；query 非空时模糊过滤标题/目标
#[tauri::command]
pub async fn list_plans(
    app: tauri::AppHandle,
    query: Option<String>,
) -> Result<Vec<PlanSummary>, AgentError> {
    let config = load_db_config(&app);
    db::list_plans(&config, query.as_deref().unwrap_or("")).await
}

/// 读取完整计划记录（含整份计划）
#[tauri::command]
pub async fn get_plan(app: tauri::AppHandle, id: String) -> Result<PlanRecord, AgentError> {
    let config = load_db_config(&app);
    db::get_plan(&config, &id).await
}

/// 删除计划（幂等）
#[tauri::command]
pub async fn delete_plan(app: tauri::AppHandle, id: String) -> Result<(), AgentError> {
    let config = load_db_config(&app);
    db::delete_plan(&config, &id).await
}

/// 切换收藏状态，返回切换后的值
#[tauri::command]
pub async fn toggle_favorite(app: tauri::AppHandle, id: String) -> Result<bool, AgentError> {
    let config = load_db_config(&app);
    db::toggle_favorite(&config, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 粘包分帧：一次喂入多行 + 尾行无换行结尾，逐次取出完整行，残行留在缓冲
    #[test]
    fn take_line_splits_buffered_chunks() {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.extend_from_slice(
            b"{\"type\":\"outline\"}\n{\"type\":\"phase\"}\n{\"type\":\"done\"}",
        );
        assert_eq!(
            take_line(&mut buffer).as_deref(),
            Some("{\"type\":\"outline\"}")
        );
        assert_eq!(
            take_line(&mut buffer).as_deref(),
            Some("{\"type\":\"phase\"}")
        );
        assert!(
            take_line(&mut buffer).is_none(),
            "尾行无换行结尾时不应提前取出"
        );
        // 残行补上换行后可完整取出
        buffer.push(b'\n');
        assert_eq!(
            take_line(&mut buffer).as_deref(),
            Some("{\"type\":\"done\"}")
        );
        assert!(take_line(&mut buffer).is_none(), "缓冲耗尽后应返回 None");
    }

    /// Windows 行尾 \r\n 与空行：行内容剥 \r，空行产出空串由调用方跳过
    #[test]
    fn take_line_strips_crlf_and_yields_empty_lines() {
        let mut buffer: Vec<u8> = b"first\r\n\r\nsecond\n".to_vec();
        assert_eq!(take_line(&mut buffer).as_deref(), Some("first"));
        assert_eq!(
            take_line(&mut buffer).as_deref(),
            Some(""),
            "空行应产出空串（parse_ndjson_line 会过滤）"
        );
        assert_eq!(take_line(&mut buffer).as_deref(), Some("second"));
        assert!(take_line(&mut buffer).is_none());
    }

    /// 行序列逐行解析：三种合法事件 + 空行过滤（与 protocol::parse_ndjson_line 协同）
    #[test]
    fn ndjson_lines_parse() {
        let stream = concat!(
            "{\"type\":\"outline\",\"phaseCount\":2,\"title\":\"Rust\"}\n",
            "\n",
            "{\"type\":\"phase\",\"index\":0,\"phase\":{}}\n",
            "{\"type\":\"done\",\"response\":{\"status\":\"plan\"}}\n",
        );
        let events: Vec<StreamEvent> = stream.lines().filter_map(parse_ndjson_line).collect();
        assert_eq!(events.len(), 3, "空行应被过滤");
        assert!(matches!(
            events[0],
            StreamEvent::Outline { phase_count: 2, .. }
        ));
        assert!(matches!(events[1], StreamEvent::Phase { index: 0, .. }));
        assert!(matches!(events[2], StreamEvent::Done { .. }));
    }
}
