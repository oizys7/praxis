//! Tauri commands：前端 `invoke()` 的入口层。
//!
//! 多数命令职责仅限「参数接收 + 委托 + 结果透传」，不承载细节逻辑：
//! - HTTP 转发与超时策略见 agent_client；
//! - sidecar 生命周期见 sidecar；
//! - 错误归一与序列化见 error；数据模型见 protocol。
//!
//! 例外是 generate_plan / regenerate_phase / regenerate_chapter：流式消费 +
//! 进度推送 + 自动入库的编排枢纽。
//! - generate_plan（任务 22）：提交即返 —— POST /api/generations 拿 taskId 后
//!   spawn 后台任务经 WS 事件通道消费（断线重连），命令同步返回 {planId}，
//!   进度经全局事件 'plan-progress' 推送；
//! - regenerate 双命令：仍走 /api/regenerate/stream NDJSON 长流（共享
//!   consume_ndjson_stream 骨架），进度经可选 Channel 推送（见函数文档）。
//!
//! 数据结构与以下两处严格对齐（字段均为 camelCase）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use serde::Serialize;
use sqlx::PgPool;
use tauri::ipc::{Channel, JavaScriptChannelId};
use tauri::Emitter;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::agent_client::{
    check_llm, connect_generation_events, get_task_snapshot, new_request_id, post_stream,
    reconnect_cursor, send, submit_plan_generation, WsStream, PING_TIMEOUT, STREAM_IDLE_TIMEOUT,
    STREAM_MAX_TIMEOUT,
};
use crate::db;
use crate::error::AgentError;
use crate::plan_state;
use crate::plan_store::{self, PlanRequestRow};
use crate::protocol::{
    parse_ndjson_line, parse_ws_envelope, MonthCalendarDay, PlanProgressEnvelope,
    PlanProgressEvent, PlanRecord, PlanRequest, PlanResponse, PlanSummary, RegenOutline,
    RegenRequest, StreamEvent, YearHeatmapDay,
};
use crate::schedule;
use crate::settings::{load_settings, save_settings_to_disk, AgentSettings};
use crate::sidecar::ensure_agent_running;

/// 读取云端数据库配置（任务 28 起连 config.json `database.url`，config.yaml 链退居
/// 迁移期保留——见 config.rs 与计划「环境变量 → 去向表」）。
/// - 装载成功：把单条连接串解析为 DbConfig 字段（见 db_config_from_url）；
/// - config 无 url / 未装载 / 解析失败：空配置（host 为空）——沿用既有
///   「DB 不可用直通」语义：generate_plan 只提示不改主流程，regenerate 系列直接 Err。
///
/// pub(crate)：main.rs setup 钩子的启动清扫同样需要读库配置。
pub(crate) fn load_db_config() -> crate::config::DbConfig {
    let Some((_, config)) = crate::app_config::global() else {
        tracing::debug!("config.json 未装载，云端数据库视为未配置");
        return crate::config::DbConfig::default();
    };
    // 审查 F3：空串是常态「未配置」，不产生 warn 噪音（仅非空且解析失败才 warn）
    if config.database.url.trim().is_empty() {
        tracing::debug!("database.url 未填写，云端数据库视为未配置");
        return crate::config::DbConfig::default();
    }
    db_config_from_url(&config.database.url)
}

/// 把 postgres 连接串（`database.url`）解析为 DbConfig 字段。
///
/// 形如 `postgres://user:password@host:port/dbname`；亦接受 `postgresql://`
/// 与 IPv6 主机（`[::1]`）。缺省：port=5432，user/password/database 空串（与
/// config.yaml 字段语义一致，密码不以任何形式解密/加密——config.json 中明文）。
/// `%` 百分号编码按 UTF-8 解码（password 含保留字符时）：非法序列原样保留。
/// URL 查询参数（如 ?sslmode=...）不支持：本层不透传，db.rs 显式关闭 TLS 并
/// 自行注入 statement_timeout，不受影响。
/// 解析失败（非 postgres scheme / 空串）→ 空配置 + warn 日志，交上层走直通语义。
fn db_config_from_url(raw: &str) -> crate::config::DbConfig {
    use crate::config::DbConfig;

    let fail = |message: String| -> DbConfig {
        tracing::warn!(url = %raw, "{message}，云端数据库视为未配置");
        DbConfig::default()
    };

    let Some((scheme, rest)) = raw.split_once("://") else {
        return fail("连接串缺少 `://`（scheme 前缀）".to_string());
    };
    if scheme != "postgres" && scheme != "postgresql" {
        return fail(format!(
            "连接串 scheme 是 `{scheme}`，期望 postgres/postgresql"
        ));
    }

    // userinfo 部分（user[:password]@）：从右找 '@'，宿主名不含 '@'
    let (user, password) = match rest.rsplit_once('@') {
        Some((userinfo, _hostpath)) => match userinfo.split_once(':') {
            Some((user, password)) => (percent_decode(user), percent_decode(password)),
            None => (percent_decode(userinfo), String::new()),
        },
        None => (String::new(), String::new()),
    };

    // host[:port]/database：无 userinfo 时才是完整起点（rsplit 已把 userinfo 剥掉）
    let rest = rest
        .rsplit_once('@')
        .map(|(_, hostpath)| hostpath)
        .unwrap_or(rest);
    let (hostport, database_raw) = match rest.split_once('/') {
        Some((hostport, db)) => (hostport.to_string(), percent_decode(db)),
        None => (rest.to_string(), String::new()),
    };
    // dbname 之后的查询参数不参与连接语义（见函数文档），直接剥离
    let database = database_raw
        .split_once('?')
        .map(|(db, _)| db)
        .unwrap_or(&database_raw)
        .to_string();

    // host[:port]：IPv6 用 '[::1]:5432' 形态，'[' 到 ']' 之间是主机
    let (host, port_raw) = if let Some(stripped) = hostport.strip_prefix('[') {
        match stripped.split_once(']') {
            Some((host, after)) => (
                host.to_string(),
                after.strip_prefix(':').map(str::to_string),
            ),
            None => return fail("IPv6 主机缺少闭合 ']'".to_string()),
        }
    } else if let Some((host, port)) = hostport.split_once(':') {
        (host.to_string(), Some(port.to_string()))
    } else {
        (hostport.clone(), None)
    };

    let port = match port_raw {
        None => 5432,
        Some(port) => match port.parse::<u16>() {
            Ok(port) => port,
            Err(_) => return fail(format!("端口 `{port}` 不是 0-65535 的整数")),
        },
    };

    if host.is_empty() {
        return fail("主机为空".to_string());
    }
    DbConfig {
        host,
        port,
        database,
        user,
        password,
    }
}

/// `%percent` 编码 → UTF-8 文本（非法序列按字面保留，避免一条坏编码毁掉整串）
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1)
            .and_then(|h| bytes.get(index + 2).map(|l| (*h, *l)))
            .filter(|(h, l)| {
                bytes[index] == b'%' && h.is_ascii_hexdigit() && l.is_ascii_hexdigit()
            });
        match hex {
            Some((high, low)) => {
                decoded.push((hex_value(high) << 4) | hex_value(low));
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// ASCII 十六进制字符 → 数值（调用方已保证合法性）
fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        _ => (byte | 0x20) - b'a' + 10,
    }
}

// ---------------------------------------------------------------------------
// 学习计划生成（任务化：提交即返 + WS 事件消费 + 自动入库编排）
// ---------------------------------------------------------------------------

/// 本次生成的云端入库句柄：连接池 + 预置行 id；None 表示本次生成无云端入库
#[derive(Clone)]
struct Storage {
    pool: PgPool,
    plan_id: Uuid,
}

/// 进度推送：channel 未传（regen 旧调用）或推送失败时静默跳过，不影响主流程
fn notify(channel: Option<&Channel<PlanProgressEvent>>, event: PlanProgressEvent) {
    if let Some(ch) = channel {
        if let Err(e) = ch.send(event) {
            warn!(error = %e, "进度事件推送失败");
        }
    }
}

/// 全局事件推送（任务 22）：generate 链路进度经 'plan-progress' 事件推给前端，
/// 载荷为 {planId, kind, payload}（payload 三变体形状不动）；推送失败仅告警，
/// 不影响主流程（前端仍有 DB 轮询/刷新兑底）。
fn emit_progress(app: &tauri::AppHandle, plan_id: Uuid, event: PlanProgressEvent) {
    let envelope = PlanProgressEnvelope {
        plan_id: plan_id.to_string(),
        kind: "plan".into(),
        payload: event,
    };
    if let Err(e) = app.emit("plan-progress", &envelope) {
        warn!(error = %e, %plan_id, "plan-progress 事件推送失败");
    }
}

/// 断线重连退避序列（秒）：连接中断且未收终态时按序重试（1s→2s→5s→10s 长尾）；
/// 全部耗尽按失败收尾（生成所有权在 Python，重试只是补齐观测，不重不丢）。
const WS_RECONNECT_BACKOFF_SECS: &[u64] = &[1, 2, 5, 10, 10, 10, 10];

/// 生成前置校验：当前提供商未填 API Key 且未开离线演示模式时立即失败。
///
/// 为什么在 Rust 侧拦：Python 侧的 has_llm_config 只有在第一次 LLM 调用时
/// 才会触发，用户点击「生成计划」后要等十几秒才看到报错，且期间已经建了
/// generating 行、发出了请求。这里在读设置阶段就拦下，零等待、零副作用。
fn ensure_llm_ready(app: &tauri::AppHandle) -> Result<(), AgentError> {
    let settings = load_settings(app);
    if settings.llm_ready() {
        return Ok(());
    }
    Err(AgentError::Config(format!(
        "尚未配置「{}」的 API Key，无法生成学习计划",
        settings.provider_label()
    )))
}

/// 生成前探测 LLM 连通性：Key 填了但无效 / 网络不通 / 模型名写错时，
/// 同样在进入生成逻辑之前就失败并给出可读原因（不建 generating 行、
/// 不提交任务、不启动 WS 观测）。
async fn ensure_llm_connectivity() -> Result<(), AgentError> {
    let probe = check_llm().await?;
    if probe.ok {
        info!(provider = %probe.provider, "LLM 连通性探测通过");
        return Ok(());
    }
    warn!(
        provider = %probe.provider,
        message = %probe.message,
        "LLM 连通性探测失败，已阻止生成"
    );
    Err(AgentError::Llm(probe.message))
}

/// 生成学习计划（任务化，任务 22）：预置 generating 行 → POST /api/generations
/// 提交后台任务（毫秒级拿 taskId）→ spawn 后台消费任务（WS 事件通道观测 +
/// 逐阶段入库 + 全局事件推送）→ 命令立即返回 {planId}（前端不再等终态），
/// 进度经 tauri::Emitter 'plan-progress' 全局事件推送（任务 23 前端 listen）。
///
/// 编排约定：
/// - 入库前置：DB 可用则预置 generating 行（终态语义见 settle_terminal_with_hours）；
///   DB 不可用不阻塞生成，仅经 StorageIssue 事件提示；
/// - 提交失败（429 容量不足 / 422 参数校验 / 网络层）：删除预置行
///   （不会有任何生成事件，避免僵尸 generating 行）+ Err 归一透传；
/// - 生成所有权在 Python 后台任务：断线重连按游标补齐（见 GenerationConsumer），
///   Rust 重启后未收终态的行由 main.rs setup 启动清扫统一翻 failed。
#[tauri::command]
pub async fn generate_plan(
    app: tauri::AppHandle,
    request: PlanRequest,
) -> Result<PlanSubmitResult, AgentError> {
    ensure_llm_ready(&app)?;
    ensure_agent_running(&app).await?;
    sync_saved_settings_once(&app).await;
    // 连通性探测放在配置热推之后：探测用的正是本次要用的那份配置
    ensure_llm_connectivity().await?;

    let started = std::time::Instant::now();
    let req_id = new_request_id();
    info!(%req_id, goal = %request.goal, "收到学习计划生成请求（任务化）");

    // 1. 入库前置：预置 generating 行（plan_id 无论入库与否都先定死，
    //    事件信封 planId 必须与命令返回值一致）；不可入库只提示不阻塞主流程
    let plan_id = Uuid::new_v4();
    let config = load_db_config();
    let storage = match db::pool(&config).await {
        Ok(pool) => match plan_store::create_generating_plan(&pool, plan_id, &request).await {
            Ok(()) => Some(Storage { pool, plan_id }),
            Err(e) => {
                warn!(%req_id, error = %e, "预置生成中计划行失败，本次生成不入库");
                emit_progress(
                    &app,
                    plan_id,
                    PlanProgressEvent::StorageIssue {
                        message: format!("云端库写入失败，本次生成不会自动保存：{e}"),
                    },
                );
                None
            }
        },
        Err(e) => {
            warn!(%req_id, error = %e, "云端库不可用，本次生成不入库");
            let message = if config.host.trim().is_empty() {
                "云端库未配置，本次生成未自动保存".to_string()
            } else {
                format!("云端库连接失败，本次生成未自动保存：{e}")
            };
            emit_progress(&app, plan_id, PlanProgressEvent::StorageIssue { message });
            None
        }
    };

    // 2. 提交后台任务：失败态删除预置行（无任何生成事件，行不得滞留 generating）
    let task_id = match submit_plan_generation(&request, &req_id).await {
        Ok(id) => id,
        Err(e) => {
            error!(%req_id, error = %e, "生成任务提交失败");
            if let Some(s) = &storage {
                if let Err(e2) = plan_store::delete_plan_full(&s.pool, s.plan_id).await {
                    warn!(%req_id, error = %e2, "提交失败的预置行删除失败");
                }
            }
            return Err(e);
        }
    };

    // 3. spawn 后台消费：WS 观测 + 落库 + 全局事件推送，与命令返回解耦
    let consumer = GenerationConsumer {
        app: app.clone(),
        storage,
        plan_id,
        task_id: task_id.clone(),
        req_id: req_id.clone(),
        weekly_hours: request.weekly_hours,
        phase_count: 0,
        terminal: false,
    };
    tauri::async_runtime::spawn(async move {
        if let Err(e) = consumer.run().await {
            error!(error = %e, "后台生成消费任务异常退出");
        }
    });

    info!(
        %req_id,
        %plan_id,
        task_id,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "学习计划生成任务已提交（后台消费已启动）"
    );
    Ok(PlanSubmitResult {
        plan_id: plan_id.to_string(),
    })
}

/// generate_plan 命令返回值：立即返回 {planId}（camelCase wire），后续进度经
/// 'plan-progress' 全局事件推送（协议三端同步，plan v7 §2a）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSubmitResult {
    pub plan_id: String,
}

/// done 终态对应的入库收尾（任务 22 失败语义修订；任务 24 增学时终值变体；
/// 2026-10-16 用户定案：总计划校验以【章】为粒度）：
/// - clarification → 删除 generating 行（保留既有三态语义防回归）；
/// - 其余终态（plan / error / 防御性未知状态）：校验所有章脈状态，
///   全部章芄生成成功（至少一章且无失败/滞留）→ complete；存在任一
///   失败章或生成中滞留章 → failed（含 finalize_failed 的 pending 子行
///   翻牌；complete 子行不动，失败部件可从详情页重生成）；零章行
///   （纯骨架/未扩）→ failed。见 plan_store::all_chapters_complete。
/// - estimated_total_hours 为 Some 时随收尾覆写学时终值（done 包络携带的
///   plan.estimatedTotalHours 与 outline 期粗估差异修正，见 done 分派；
///   finalize_plan COALESCE 语义，None 保持原值）。
///   pub(crate)：tests_plan_store 的真库集成测试直接调取（收尾语义的库级验证）。
pub(crate) async fn settle_terminal_with_hours(
    pool: &PgPool,
    plan_id: Uuid,
    status: &str,
    estimated_total_hours: Option<i64>,
) -> Result<(), AgentError> {
    if status == "clarification" {
        return plan_store::delete_plan_full(pool, plan_id).await;
    }
    if status != "plan" && status != "error" {
        warn!(status, "未知 done 终态状态，按内容语义收尾");
    }
    // 终态口径仍是章级（all_chapters_complete = plan_state::plan_of_chapters 的
    // SQL 版），但收尾参数已类型化，不再有字符串传错的可能。
    let plan_status = if plan_store::all_chapters_complete(pool, plan_id).await? {
        plan_state::PlanStatus::Complete
    } else {
        plan_state::PlanStatus::Failed
    };
    plan_store::finalize_plan(pool, plan_id, plan_status, None, estimated_total_hours).await
}

/// chapter 事件的入库结局 → 章级进度事件（纯函数，单测快照）：Ok →
/// ChapterDone（章行确已落库才发，与 RegenSink 单章路径语义对齐）；
/// Err → StorageIssue（生成不中断，前端降级提示）。章级事件的进度约定：
/// PhaseDone 保持只随 phase 事件推送——章分支产不出 PhaseDone，前端
/// phaseStatuses 索引集齐的终态口径与骨架 done-bit 不被章级流扰动。
pub(crate) fn chapter_progress_from_outcome(
    outcome: Result<(), AgentError>,
    phase_index: u32,
    chapter_index: u32,
    chapter_title: &str,
) -> PlanProgressEvent {
    match outcome {
        Ok(()) => PlanProgressEvent::ChapterDone {
            phase_index,
            chapter_index,
        },
        Err(e) => PlanProgressEvent::StorageIssue {
            message: format!("章节「{chapter_title}」未能自动保存：{e}"),
        },
    }
}

// ---------------------------------------------------------------------------
// 共享流消费骨架（generate_plan / regenerate 双命令同源，防双份漂移）
// ---------------------------------------------------------------------------

/// 流式消费事件分派器：每条流式命令链路实现一个，负责事件处理（入库 / 进度
/// 推送 / 终态收尾）；返回 Ok(Some(envelope)) 表示已收到 done 终态（消费提前
/// 收束），Ok(None) 表示事件已处理、继续消费。唯一消费者是
/// [consume_ndjson_stream]（分行 / 解析 / 非 2xx 校验基础设施同源）。
trait StreamEventSink {
    fn handle_event<'a>(&'a mut self, event: StreamEvent) -> SinkFuture<'a>;
}

/// 分派单个事件的 boxed future：借用 &mut self 的生命周期；Send 约束因外层
/// tokio::time::timeout 包裹的组合 future 需跨 await Send（逐事件装箱，
/// 事件频次为每流个位数，开销可忽略）。
type SinkFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<PlanResponse>, AgentError>> + Send + 'a>>;

/// NDJSON 流消费共享骨架：非 2xx 前置校验（如 422 参数校验，响应体不是
/// NDJSON，归一为错误）→ 逐行读取（take_line / next_line 分帧）→ 解析
/// （坏行告警跳过，不中断流）→ 交给 sink 分派。sink 返回 Some(envelope)
/// （done 终态）时提前收束返回 Ok(Some)；流读完仍无 done → Ok(None)
/// （调用方收尾 Err）。
async fn consume_ndjson_stream(
    response: &mut reqwest::Response,
    req_id: &str,
    sink: &mut impl StreamEventSink,
    idle_timeout: std::time::Duration,
) -> Result<Option<PlanResponse>, AgentError> {
    if !response.status().is_success() {
        let status = response.status();
        // reqwest 的 text() 按值收响应体；&mut 场景改用 chunk 累积读取
        // （错误响应体很小，循环成本可忽略）
        let mut body_bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            body_bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&body_bytes);
        let snippet: String = body.chars().take(200).collect();
        warn!(%req_id, http_status = %status, body = %snippet, "流式端点返回非 2xx");
        return Err(AgentError::Service(format!("Agent 服务返回 HTTP {status}")));
    }

    let mut buffer: Vec<u8> = Vec::new();
    loop {
        // 逐行空闲超时：每读到一行就重置计时。重生成全阶段路径逐章串行
        // 调用 LLM（单章可达数分钟），用总超时必然假超时；chapterStart
        // 事件天然是心跳，因此"多久没有新事件"才是流健康的真实判据。
        let next = match tokio::time::timeout(idle_timeout, next_line(response, &mut buffer)).await
        {
            Ok(result) => result?,
            Err(_) => {
                warn!(
                    %req_id,
                    idle_secs = idle_timeout.as_secs(),
                    "NDJSON 流空闲超时，判定流已中断"
                );
                return Err(AgentError::Service(format!(
                    "生成流已 {} 秒没有任何进展，已中断，请重试",
                    idle_timeout.as_secs()
                )));
            }
        };
        let Some(line) = next else { break };
        if line.trim().is_empty() {
            continue; // 空行跳过
        }
        let Some(event) = parse_ndjson_line(&line) else {
            warn!(%req_id, "流中出现无法解析的事件行，已跳过");
            continue; // 坏行不中断流
        };
        if let Some(envelope) = sink.handle_event(event).await? {
            return Ok(Some(envelope));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// WS 事件消费（任务 22）：generate_plan 后台观测链路
// ---------------------------------------------------------------------------

/// WS 事件消费分派器：generate 链路专用（与流式链路的 StreamEventSink 同构，
/// 但多带 seq——重连重放的安全网在游标，分派侧按 seq 只做日志关联）。
/// 返回 Ok(true) 表示已收 done 终态（消费收束）；Ok(false) 继续消费。
trait WsEventSink {
    fn handle_event<'a>(&'a mut self, seq: u64, event: StreamEvent) -> WsFuture<'a>;
}

/// 分派单个事件的 boxed future：借用 &mut self 的生命周期；Send 约束因组合
/// future 需跨 await（逐事件装箱，事件频次为每任务个位数，开销可忽略）。
type WsFuture<'a> = Pin<Box<dyn Future<Output = Result<bool, AgentError>> + Send + 'a>>;

/// 单条 WS 连接的读取循环：逐消息解析信封（坏消息告警跳过，不中断）→ 分派
/// sink；收到 done（sink 返回 true）即收束。返回（已消费的最大 seq, 可选错误）：
/// 出错路径也带出已读 seq（审查 P2）：重连回放会高于真实消费点，但幂等兜底
/// 前端不脏——代价仅为 Outline/PhaseDone 进度事件重复推送。
async fn read_ws_events<S: WsEventSink>(
    mut stream: WsStream,
    req_id: &str,
    sink: &mut S,
) -> (u64, Option<AgentError>) {
    let mut last_seq: u64 = 0;
    while let Some(msg) = ws_next(&mut stream).await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                return (
                    last_seq,
                    Some(AgentError::Service(format!("事件通道读取失败：{e}"))),
                );
            }
        };
        let text = match msg {
            Message::Text(t) => t.as_str().to_owned(),
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            // 协议层 ping/pong（uvicorn 保活）由 tungstenite 自动应答，应用层不关心
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(frame) => {
                debug!(%req_id, code = frame.as_ref().map(|c| u16::from(c.code)), "事件通道服务端关闭");
                break;
            }
        };
        let Some(envelope) = parse_ws_envelope(&text) else {
            warn!(%req_id, "事件通道出现无法解析的消息，已跳过");
            continue; // 坏消息不中断消费
        };
        last_seq = last_seq.max(envelope.seq);
        match sink.handle_event(envelope.seq, envelope.event).await {
            Ok(true) => return (last_seq, None), // 终态已收，正常收束
            Ok(false) => {}
            Err(e) => return (last_seq, Some(e)),
        }
    }
    (last_seq, None)
}

/// futures-util StreamExt::next 的窄包装：把 StreamExt 引入范围限制在这一个
/// 函数内，避免全局引入污染其它同名方法解析。
async fn ws_next(
    stream: &mut WsStream,
) -> Option<Result<Message, tokio_tungstenite::tungstenite::Error>> {
    use futures_util::StreamExt;
    stream.next().await
}

/// generate_plan 的后台消费者：持有云端入库句柄与推送出口，串起「snapshot →
/// 连 WS → 逐事件入库/推送 → done 终态收尾」与断线重连（指数退避，游标补齐）。
/// 全重试耗尽 / 任务消失：按内容语义收尾（有 phase 行 → complete，零内容 →
/// failed）+ 归一 Err（由 spawn 外层记 error 日志；命令早已返回 planId）。
struct GenerationConsumer {
    /// 全局事件推送出口（'plan-progress'）
    app: tauri::AppHandle,
    /// 云端入库句柄；None = 本次生成不入库（仅推送，不落库）
    storage: Option<Storage>,
    plan_id: Uuid,
    /// 后台生成任务 id（Python registry）
    task_id: String,
    req_id: String,
    /// 预计总学时估算的每周学时（outline 事件到达时 × 阶段数，粗估供列表页展示）
    weekly_hours: u32,
    /// PhaseDone 推送的阶段总数（outline 事件到达时更新）
    phase_count: u32,
    /// 是否已收 done 终态（重连判断依据）
    terminal: bool,
}

impl GenerationConsumer {
    /// 完整消费流程：重试循环内「snapshot 定游标 → 连接 → 读事件」；收终态
    /// 即返回 Ok；重试耗尽 / 任务消失按内容语义收尾并返回 Err。
    async fn run(mut self) -> Result<(), AgentError> {
        let mut last_consumed: u64 = 0;
        // attempt 0..=backoff.len()：首连 + 退避表次数的重连（共 8 次）
        for attempt in 0..=WS_RECONNECT_BACKOFF_SECS.len() {
            if attempt > 0 {
                let secs = WS_RECONNECT_BACKOFF_SECS[attempt - 1];
                warn!(
                    req_id = %self.req_id,
                    task_id = %self.task_id,
                    attempt, backoff_secs = secs,
                    "事件通道断开，退避后重连"
                );
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            }

            // 快照先于连接：任务不存在（sidecar 重启/驱逐）直接判失败，不在
            // 无意义的重试上耗时间；快照网络失败降级为无 lastSeq 连接
            // （cursor=0/last_consumed 全量回放，幂等安全）
            let snapshot = match get_task_snapshot(&self.task_id, &self.req_id).await {
                Ok(Some(s)) => {
                    if s.kind != "plan" {
                        warn!(
                            req_id = %self.req_id,
                            task_id = %self.task_id,
                            kind = %s.kind,
                            "快照 kind 与链路不符（协议异常），仍按 plan 消费"
                        );
                    }
                    debug!(
                        req_id = %self.req_id,
                        snapshot_task_id = %s.task_id,
                        status = %s.status,
                        first_seq = s.first_seq,
                        last_seq = s.last_seq,
                        done = s.done,
                        "任务快照已取（游标依据）"
                    );
                    Some(s)
                }
                Ok(None) => {
                    warn!(
                        req_id = %self.req_id,
                        task_id = %self.task_id,
                        "生成任务不存在或已驱逐"
                    );
                    self.settle_after_loss().await;
                    return Err(AgentError::Service(
                        "生成任务已丢失（服务重启或超期驱逐），请重新生成".into(),
                    ));
                }
                Err(e) => {
                    warn!(
                        req_id = %self.req_id,
                        task_id = %self.task_id,
                        error = %e,
                        "任务快照获取失败，直接尝试连接"
                    );
                    None
                }
            };
            let cursor = reconnect_cursor(last_consumed, snapshot.as_ref().map(|s| s.last_seq));
            debug!(
                req_id = %self.req_id,
                task_id = %self.task_id,
                attempt,
                cursor,
                "连接事件通道"
            );

            match connect_generation_events(&self.task_id, cursor).await {
                Ok(stream) => {
                    // req_id 先取出克隆，避免与 sink 的 &mut self 借用冲突
                    let req_id = self.req_id.clone();
                    let (seq, err) = read_ws_events(stream, &req_id, &mut self).await;
                    // 出错路径也推进消费点（审查 P2）：重连回放可能重叠已消费段，
                    // 处理幂等（upsert/收尾幂等 + done 单发）保证不脏，仅观感重复
                    last_consumed = last_consumed.max(seq);
                    if self.terminal {
                        return Ok(()); // 终态已收尾（done 分派内完成）
                    }
                    if let Some(e) = err {
                        warn!(req_id = %self.req_id, task_id = %self.task_id, error = %e, "事件通道读循环错误");
                    }
                    // 连接断开/读错误但未收终态：继续重试循环
                }
                Err(e) => {
                    warn!(req_id = %self.req_id, task_id = %self.task_id, error = %e, "事件通道连接失败");
                }
            }
        }

        // 全重试耗尽：按内容语义收尾（观测丢失不等于生成失败，已入库的部分
        // 内容对用户仍有价值，可重看/重生成）
        self.settle_after_loss().await;
        Err(AgentError::Service(
            "生成进度通道中断（多次重连失败），本次进度不再跟踪，请稍后刷新查看".into(),
        ))
    }

    /// 观测丢失后的收尾（2026-10-16 章级口径）：全部章脈生成成功 →
    /// complete；失败章/滞留骨架/零内容 → failed。
    /// 收尾失败仅告警（行可能滞留 generating，由下次启动清扫兜底）。
    async fn settle_after_loss(&mut self) {
        self.terminal = true;
        let Some(s) = &self.storage else {
            return;
        };
        let semantic_status = match plan_store::all_chapters_complete(&s.pool, s.plan_id).await {
            Ok(true) => plan_state::PlanStatus::Complete,
            Ok(false) => plan_state::PlanStatus::Failed,
            Err(e) => {
                warn!(req_id = %self.req_id, error = %e, "章脈完成状态查询失败，按 failed 收尾");
                plan_state::PlanStatus::Failed
            }
        };
        if let Err(e) =
            plan_store::finalize_plan(&s.pool, s.plan_id, semantic_status, None, None).await
        {
            warn!(req_id = %self.req_id, error = %e, "观测丢失收尾失败（下次启动清扫兜底）");
            emit_progress(
                &self.app,
                self.plan_id,
                PlanProgressEvent::StorageIssue {
                    message: format!("生成结果未能按终态入库：{e}"),
                },
            );
        }
    }
}

impl WsEventSink for GenerationConsumer {
    fn handle_event<'a>(&'a mut self, seq: u64, event: StreamEvent) -> WsFuture<'a> {
        Box::pin(async move {
            match event {
                StreamEvent::Outline {
                    phase_count: count,
                    title,
                    overview,
                    phases,
                } => {
                    self.phase_count = count;
                    if let Some(s) = &self.storage {
                        // 预计总学时：大纲阶段数 × 每周学时（粗估，供云端列表页展示）
                        let hours = i64::from(self.weekly_hours).saturating_mul(i64::from(count));
                        if let Err(e) = plan_store::update_outline_meta(
                            &s.pool,
                            s.plan_id,
                            &title,
                            overview.as_deref(),
                            hours,
                        )
                        .await
                        {
                            warn!(req_id = %self.req_id, seq, error = %e, "计划大纲元信息入库失败，不影响生成");
                            emit_progress(
                                &self.app,
                                self.plan_id,
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
                                warn!(req_id = %self.req_id, seq, error = %e, "大纲骨架播种失败，不影响生成");
                                emit_progress(
                                    &self.app,
                                    self.plan_id,
                                    PlanProgressEvent::StorageIssue {
                                        message: format!("大纲骨架未能自动保存：{e}"),
                                    },
                                );
                            }
                        }
                    }
                    emit_progress(
                        &self.app,
                        self.plan_id,
                        PlanProgressEvent::Outline {
                            phase_count: count,
                            title,
                            overview,
                            phases,
                        },
                    );
                    Ok(false)
                }
                StreamEvent::Phase { index, phase } => {
                    if let Some(s) = &self.storage {
                        // Python 端 index 为小非负整数；防御性越界按 0 处理（实际不可达）
                        let idx = i32::try_from(index).unwrap_or(0);
                        match plan_store::upsert_phase(&s.pool, s.plan_id, idx, &phase).await {
                            Ok(()) => {
                                // 审查 Finding-4（任务 24）：阶段行确已落库才推
                                // PhaseDone，避免前端假完成 + 回读缺行
                                emit_progress(
                                    &self.app,
                                    self.plan_id,
                                    PlanProgressEvent::PhaseDone {
                                        index,
                                        total: self.phase_count,
                                    },
                                );
                            }
                            Err(e) => {
                                warn!(req_id = %self.req_id, seq, index, error = %e, "阶段入库失败，不中断生成");
                                emit_progress(
                                    &self.app,
                                    self.plan_id,
                                    PlanProgressEvent::StorageIssue {
                                        message: format!(
                                            "第 {} 阶段未能自动保存：{e}",
                                            index.saturating_add(1)
                                        ),
                                    },
                                );
                            }
                        }
                    } else {
                        // 无云端入库句柄（库不可用降级）：无落库语义可判，进度
                        // 推送照常（观测与入库解耦，前端仍有终态对账兑底）
                        emit_progress(
                            &self.app,
                            self.plan_id,
                            PlanProgressEvent::PhaseDone {
                                index,
                                total: self.phase_count,
                            },
                        );
                    }
                    Ok(false)
                }
                // D1 章级扇出：初生链路每章完成产一条 chapter 事件（此前仅
                // 单章重生成路径出现，任务 14 时按协议异常 warn 忽略；旧
                // Python 端已退役，忽略分支随之升级为真处理）。入库与推送
                // 解耦：upsert 失败不中断生成（对齐 RegenSink 单章路径口径）。
                StreamEvent::Chapter {
                    index,
                    phase_index,
                    ref chapter,
                } => {
                    let outcome = if let Some(s) = &self.storage {
                        plan_store::upsert_chapter(
                            &s.pool,
                            self.plan_id,
                            phase_index,
                            index,
                            chapter,
                        )
                        .await
                    } else {
                        // 无云端入库句柄（库不可用降级）：无落库语义可判，
                        // 进度推送照常（与 phase 事件 storage=None 直通同构）
                        Ok(())
                    };
                    if let Err(e) = &outcome {
                        warn!(
                            req_id = %self.req_id,
                            seq, index, phase_index,
                            error = %e,
                            "生成章节入库失败，不中断生成"
                        );
                    }
                    emit_progress(
                        &self.app,
                        self.plan_id,
                        chapter_progress_from_outcome(outcome, phase_index, index, &chapter.title),
                    );
                    Ok(false)
                }
                // 生成链路不发 chapterStart（仅重生成链），防御性忽略
                StreamEvent::ChapterStart { .. } => {
                    warn!(req_id = %self.req_id, "生成流出现非预期 chapterStart 事件，已忽略");
                    Ok(false)
                }
                StreamEvent::Done { response: envelope } => {
                    self.terminal = true;
                    if let Some(s) = &self.storage {
                        // 审查 Finding-1（任务 24）：done 包络携带的推荐资源落库
                        // （DELETE+INSERT 幂等覆写，重发 done 不重复）——结果页
                        // 唯一内容源是回读 DB，缺此步则资源卡永久缺失。
                        // clarification（plan=None）不写：行随终态删除，级联清理。
                        // 空资源也不写（审查 P2-1）：保持与 docstring 契约一致，
                        // 避免末来 outline 播种资源被空 done 覆写清零。
                        if let Some(plan) = &envelope.plan {
                            if !plan.resources.is_empty() {
                                if let Err(e) = plan_store::upsert_resources(
                                    &s.pool,
                                    s.plan_id,
                                    &plan.resources,
                                )
                                .await
                                {
                                    warn!(req_id = %self.req_id, seq, error = %e, "done 包络资源落库失败，不影响终态收尾");
                                    emit_progress(
                                        &self.app,
                                        self.plan_id,
                                        PlanProgressEvent::StorageIssue {
                                            message: format!("推荐资源未能自动保存：{e}"),
                                        },
                                    );
                                }
                            } // resources 非空守卫
                        } // plan Some（clarification 级联清理豁免）
                          // 学时终值覆写（Finding-1 附加）：outline 期仅粗估
                          // （weeklyHours × 阶段数，计划 v7 §3 有意不引 LLM 终值），
                          // done 包络携带 plan.estimatedTotalHours 终值，一并修正；
                          // 零值视为缺省不覆写
                        let hours = envelope
                            .plan
                            .as_ref()
                            .map(|p| p.estimated_total_hours)
                            .filter(|h| *h > 0)
                            .map(i64::from);
                        if let Err(e) =
                            settle_terminal_with_hours(&s.pool, s.plan_id, &envelope.status, hours)
                                .await
                        {
                            warn!(req_id = %self.req_id, seq, error = %e, "终态入库收尾失败，不影响事件推送");
                            emit_progress(
                                &self.app,
                                self.plan_id,
                                PlanProgressEvent::StorageIssue {
                                    message: format!("生成结果未能按终态入库：{e}"),
                                },
                            );
                        }
                    }
                    info!(
                        req_id = %self.req_id,
                        task_id = %self.task_id,
                        status = %envelope.status,
                        "学习计划后台生成完成（终态已收）"
                    );
                    Ok(true)
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// 阶段/章节级重生成（任务 15）：上下文重建 + /api/regenerate/stream 流消费
// ---------------------------------------------------------------------------

/// 重生成目标越界校验（纯函数）：phase_index 对 outline.phases 范围；
/// chapter_index 对该阶段 chapterTitles 范围。越界直接 Err 不发 HTTP
/// （Python 端 done(error) 校验兜底，此处提前拦截省一次往返）。
fn validate_regen_targets(
    outline: &RegenOutline,
    phase_index: u32,
    chapter_index: Option<u32>,
) -> Result<(), AgentError> {
    let phases = outline.phases.len() as u32;
    if phase_index >= phases {
        return Err(AgentError::Service(format!(
            "重生成目标阶段越界：phaseIndex={phase_index}，计划共 {phases} 个阶段"
        )));
    }
    if let Some(ci) = chapter_index {
        let chapters = outline.phases[phase_index as usize].chapter_titles.len() as u32;
        if ci >= chapters {
            return Err(AgentError::Service(format!(
                "重生成目标章节越界：phaseIndex={phase_index} 的 chapterIndex={ci}，该阶段共 {chapters} 个章节"
            )));
        }
    }
    Ok(())
}

/// 重建 regenerate 请求体（纯函数，便于单测）：PlanRequestRow + RegenOutline →
/// RegenRequest；PlanRequest 七字段逐值回填（与原请求一致）。
fn build_regen_body(
    meta: PlanRequestRow,
    outline: RegenOutline,
    phase_index: u32,
    chapter_index: Option<u32>,
) -> RegenRequest {
    RegenRequest {
        request: PlanRequest {
            goal: meta.goal,
            level: meta.level,
            weekly_hours: meta.weekly_hours,
            weeks: meta.weeks,
            interests: meta.interests,
            background: meta.background,
            study_days: meta.study_days,
        },
        outline,
        phase_index,
        chapter_index,
    }
}

/// regenerate 流事件分派：全阶段路径（chapter_index=None）处理 phase 事件
/// （upsert_phase → PhaseDone{index, total=阶段总数}）；单章路径处理 chapter
/// 事件（upsert_chapter → ChapterDone{phaseIndex, chapterIndex}，阶段联动由
/// upsert_chapter 事务内推进）；对侧路径事件 warn 忽略防错；done 终态包络
/// 直接回传（计划行收尾由 run_regen 统一做）。
struct RegenSink<'a> {
    pool: &'a PgPool,
    plan_id: Uuid,
    progress: Option<&'a Channel<PlanProgressEvent>>,
    req_id: &'a str,
    /// 目标阶段索引（命令入口已对 outline 范围校验）
    phase_index: u32,
    /// Some=单章重生成（目标章节索引）；None=全阶段重生成
    chapter_index: Option<u32>,
    /// PhaseDone 的 total：全阶段=计划阶段总数（单章路径不发 PhaseDone）
    total: u32,
}

impl StreamEventSink for RegenSink<'_> {
    fn handle_event<'a>(&'a mut self, event: StreamEvent) -> SinkFuture<'a> {
        Box::pin(async move {
            match event {
                StreamEvent::Phase { index, phase } if self.chapter_index.is_none() => {
                    // 槽位以入口校验后的目标值为准（事件回显仅作显示），与单章路径防御口径一致
                    let idx = i32::try_from(self.phase_index).unwrap_or(0);
                    if let Err(e) =
                        plan_store::upsert_phase(self.pool, self.plan_id, idx, &phase).await
                    {
                        warn!(%self.req_id, index, error = %e, "重生成阶段入库失败，不中断流");
                        notify(
                            self.progress,
                            PlanProgressEvent::StorageIssue {
                                message: format!("重生成阶段未能自动保存：{e}"),
                            },
                        );
                    }
                    notify(
                        self.progress,
                        PlanProgressEvent::PhaseDone {
                            index,
                            total: self.total,
                        },
                    );
                    Ok(None)
                }
                StreamEvent::Chapter {
                    index,
                    phase_index: _,
                    chapter,
                } => {
                    // 两路径共用（D1 后 Python 全阶段重生成也逐章发事件）：
                    // 单章路径目标章 = 入口校验的 self.chapter_index（事件值由
                    // Python 原样回显，仅作显示）；全阶段路径 = 事件回显 index
                    let ci = match self.chapter_index {
                        Some(ci) => ci,
                        None => index,
                    };
                    if let Err(e) = plan_store::upsert_chapter(
                        self.pool,
                        self.plan_id,
                        self.phase_index,
                        ci,
                        &chapter,
                    )
                    .await
                    {
                        warn!(%self.req_id, index, error = %e, "重生成章节入库失败，不中断流");
                        notify(
                            self.progress,
                            PlanProgressEvent::StorageIssue {
                                message: format!("重生成章节未能自动保存：{e}"),
                            },
                        );
                    } else {
                        // 章行确已落库才发章级进度（与 upsert_phase 失败不发
                        // PhaseDone 的既有语义对齐；失败只发 StorageIssue）
                        notify(
                            self.progress,
                            PlanProgressEvent::ChapterDone {
                                phase_index: self.phase_index,
                                chapter_index: ci,
                            },
                        );
                    }
                    Ok(None)
                }
                StreamEvent::ChapterStart {
                    phase_index: _,
                    chapter_index,
                } => {
                    // 逐章动画（2026-10-16 反馈④）：章开始生成即落库 generating
                    // （持久，退出重进保持）并推前端转圈；目标索引以入口校验值
                    // 为准（单章路径 chapter_index=Some(ci)，事件回显仅作参考）
                    let ci = self.chapter_index.unwrap_or(chapter_index);
                    if let Err(e) = plan_store::set_chapter_generating(
                        self.pool,
                        self.plan_id,
                        self.phase_index,
                        ci,
                    )
                    .await
                    {
                        warn!(%self.req_id, chapter_index = ci, error = %e, "章节置 generating 失败，不中断流");
                    }
                    notify(
                        self.progress,
                        PlanProgressEvent::ChapterStart {
                            phase_index: self.phase_index,
                            chapter_index: ci,
                        },
                    );
                    Ok(None)
                }
                StreamEvent::Phase { index, .. } => {
                    warn!(%self.req_id, index, "单章重生成流出现非预期 phase 事件，已忽略");
                    Ok(None)
                }
                StreamEvent::Outline { phase_count, .. } => {
                    warn!(%self.req_id, phase_count, "重生成流出现非预期 outline 事件，已忽略");
                    Ok(None)
                }
                StreamEvent::Done { response: envelope } => Ok(Some(envelope)),
            }
        })
    }
}

/// regenerate 双命令的共享编排（regenerate_phase / regenerate_chapter 仅
/// chapter_index 与 mark pending 粒度不同）：
/// ①plans 七字段 + 大纲骨架读回，重建 PlanRequest / RegenOutline；
/// ②目标行置 generating（前端即时转「生成中」，落库持久可跨页面保持）；
/// ③POST /api/regenerate/stream 并消费流（共享 consume_ndjson_stream 骨架）；
/// ④done(plan) → 计划行章级收尾（finalize_regen_complete：全章节 complete
/// 才翻 complete，含 failed 行——2026-10-16 修复重生成补齐后 failed 卡死）；
/// done(error) → finalize_failed（pending 行翻 failed，
/// 前端按钮恢复可点，详情刷新可见失败态）；Err（传输断/超时/流提前中断）→
/// finalize_failed + Err 透传。收尾失败仅告警，不影响返回。
async fn run_regen(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    channel_id: Option<JavaScriptChannelId>,
    plan_id: String,
    phase_index: u32,
    chapter_index: Option<u32>,
) -> Result<PlanResponse, AgentError> {
    ensure_llm_ready(&app)?;
    ensure_agent_running(&app).await?;
    sync_saved_settings_once(&app).await;
    ensure_llm_connectivity().await?;

    let started = std::time::Instant::now();
    let req_id = new_request_id();
    info!(
        %req_id,
        %plan_id,
        phase_index,
        chapter_index = chapter_index.map(|c| c.to_string()).as_deref().unwrap_or("-"),
        "收到重生成请求（流式）"
    );

    // 前端传了 Channel 则还原为进度推送通道；不传则优雅降级为不发进度
    let channel = channel_id.map(|id| id.channel_on(webview));
    let progress = channel.as_ref();

    // 计划 id：IPC 传字符串，非法即 Err（不碰库不发 HTTP）
    let plan_id = Uuid::parse_str(&plan_id)
        .map_err(|_| AgentError::Service(format!("无效的计划 id：{plan_id}")))?;

    // 云端库必经（上下文在库里），不可用直接 Err——与 generate_plan 的
    // 「入库可降级」不同，重生成没有库就没有上下文
    let config = load_db_config();
    let pool = db::pool(&config).await?;

    // ① 上下文重建：plans 七字段 + 大纲骨架（计划不存在即 Err）
    let Some(meta) = plan_store::load_plan_request_meta(&pool, plan_id).await? else {
        return Err(AgentError::Service(format!(
            "计划不存在，无法重生成：{plan_id}"
        )));
    };
    let Some(outline_row) = plan_store::load_plan_outline(&pool, plan_id).await? else {
        return Err(AgentError::Service(format!(
            "计划大纲缺失，无法重生成：{plan_id}"
        )));
    };
    let outline = RegenOutline {
        title: outline_row.title,
        overview: outline_row.overview,
        estimated_total_hours: outline_row.estimated_total_hours,
        phases: outline_row.phases,
    };

    // 入口越界校验：直接 Err 不发 HTTP
    validate_regen_targets(&outline, phase_index, chapter_index)?;

    // 单章：projectBrief 读既有 project 行 name 充当简介（任务 15 裁定 1）；
    // 全阶段：projectBrief 保持缺省（project 随全阶段重生成，不读旧行）
    let outline = match chapter_index {
        None => outline,
        Some(_) => {
            let brief = plan_store::load_phase_project_brief(&pool, plan_id, phase_index)
                .await?
                .unwrap_or_default();
            let mut outline = outline;
            if let Some(p) = outline.phases.get_mut(phase_index as usize) {
                p.project_brief = (!brief.trim().is_empty()).then_some(brief);
            }
            outline
        }
    };

    // ② 目标行置 generating（前端即时转「生成中」且落库持久——退出再进入
    //    保持，2026-10-16 反馈①；失败仅提示不阻塞——与 generate_plan
    //    入库容错口径一致，后续成功入库 / 失败收尾均会覆写状态）
    let mark_result = match chapter_index {
        None => plan_store::mark_phase_generating(&pool, plan_id, phase_index).await,
        Some(ci) => plan_store::mark_chapter_generating(&pool, plan_id, phase_index, ci).await,
    };
    if let Err(e) = mark_result {
        warn!(%req_id, error = %e, "重生成目标状态置 generating 失败，不影响生成");
        notify(
            progress,
            PlanProgressEvent::StorageIssue {
                message: format!("重生成状态未能即时更新：{e}"),
            },
        );
    }

    // ③ 请求 + 消费：逐行空闲超时（重生成全阶段按章串行调用 LLM，
    //    "总超时"会把正常的长任务误判为超时；这里改看流是否还有新事件）
    let mut sink = RegenSink {
        pool: &pool,
        plan_id,
        progress,
        req_id: &req_id,
        phase_index,
        chapter_index,
        total: if chapter_index.is_some() {
            1
        } else {
            outline.phases.len() as u32
        },
    };
    let body = build_regen_body(meta, outline, phase_index, chapter_index);
    let consumed = match post_stream(
        "/api/regenerate/stream",
        &body,
        STREAM_MAX_TIMEOUT,
        Some(&req_id),
    )
    .await
    {
        Ok(mut response) => {
            consume_ndjson_stream(&mut response, &req_id, &mut sink, STREAM_IDLE_TIMEOUT).await
        }
        Err(e) => {
            error!(%req_id, error = %e, "重生成流请求发送失败");
            Err(e)
        }
    };

    // ④ 终态分派结果 + 收尾：done 包络整体透传（前端只判成败，详情刷新走
    //    fetchPlanRecord）；未收到 done（传输错误/超时/提前 EOF）→ Err
    let result = match consumed {
        Ok(Some(envelope)) => {
            info!(
                %req_id,
                status = %envelope.status,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "重生成流式完成"
            );
            Ok(envelope)
        }
        Ok(None) => {
            error!(%req_id, "重生成流提前结束：未收到 done 终态事件");
            Err(AgentError::Service(
                "重生成流提前中断，未收到最终结果，请重试".into(),
            ))
        }
        Err(e) => Err(e),
    };
    match &result {
        // done(plan)：计划行章级收尾（全章 complete 才翻 complete，含 failed 行）
        Ok(envelope) if envelope.status == "plan" => {
            if let Err(e) = plan_store::finalize_regen_complete(&pool, plan_id).await {
                warn!(%req_id, error = %e, "重生成计划行收尾失败");
                notify(
                    progress,
                    PlanProgressEvent::StorageIssue {
                        message: format!("重生成结果收尾未能保存：{e}"),
                    },
                );
            }
        }
        // done(error)（及防御性其他状态）/ 传输 Err：pending 行翻 failed
        // （complete 不动），前端按钮恢复可点、详情刷新显示失败态
        _ => {
            if let Err(e) = plan_store::finalize_failed(&pool, plan_id).await {
                warn!(%req_id, error = %e, "重生成失败态收尾失败");
                notify(
                    progress,
                    PlanProgressEvent::StorageIssue {
                        message: format!("重生成失败态收尾未成功：{e}"),
                    },
                );
            }
        }
    }
    result
}

/// 重生成整个阶段（历史详情页失败阶段旁「重新生成此阶段」）：上下文从入库
/// plans 七字段 + 大纲骨架回填，POST /api/regenerate/stream（chapterIndex
/// 缺省），done 包络整体透传（前端只判成败，详情刷新走 fetchPlanRecord）。
///
/// channel 形态说明同 [generate_plan]：可选 `JavaScriptChannelId` + webview
/// 注入还原，语义等同可选 Channel。
#[tauri::command]
pub async fn regenerate_phase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    plan_id: String,
    phase_index: u32,
    channel: Option<JavaScriptChannelId>,
) -> Result<PlanResponse, AgentError> {
    run_regen(app, webview, channel, plan_id, phase_index, None).await
}

/// 重生成单个章节（失败章节旁「重新生成此章」）：同 [regenerate_phase]，body
/// 携带 chapterIndex；成功后所属阶段状态在 upsert_chapter 事务内联动推进。
#[tauri::command]
pub async fn regenerate_chapter(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    plan_id: String,
    phase_index: u32,
    chapter_index: u32,
    channel: Option<JavaScriptChannelId>,
) -> Result<PlanResponse, AgentError> {
    run_regen(
        app,
        webview,
        channel,
        plan_id,
        phase_index,
        Some(chapter_index),
    )
    .await
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
    // 前端启动即 ping：这里是启动静默同步的主入口（服务就绪即把持久化
    // 设置热推 Python，用户无需再到设置页手动点保存）
    sync_saved_settings_once(&app).await;

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

/// 启动静默同步：本进程是否已把持久化设置推给 Agent 服务。
/// 失败不置位——下个 Agent 命令重试（服务此刻可能尚未就绪）。
static AGENT_CONFIG_SYNCED: OnceLock<()> = OnceLock::new();

/// 把设置推给 Agent 服务（POST /api/config），成功返回 true。
/// 从 save_settings 提取共用：设置页保存热链与启动静默同步同源。
async fn push_agent_config(settings: &AgentSettings) -> bool {
    match send(
        reqwest::Method::POST,
        "/api/config",
        PING_TIMEOUT,
        Some(settings),
        None,
    )
    .await
    {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            warn!(
                status = %response.status(),
                "配置热推失败（服务返回非 2xx）"
            );
            false
        }
        Err(e) => {
            debug!(error = %e, "Agent 服务未运行，配置热推未完成");
            false
        }
    }
}

/// 首个 Agent 命令前把 settings.json 静默热推给 Python（每进程一次）。
/// 接缝背景：任务 29 退役环境变量冷启动后，Python 端 LLM 配置唯一入口是
/// /api/config 热链，但启动后无人补位推送（持久化设置只有设置页手动保存
/// 才会触发），导致每次重启都要手动点一次热加载——本函数补齐该缺口。
async fn sync_saved_settings_once(app: &tauri::AppHandle) {
    if AGENT_CONFIG_SYNCED.get().is_some() {
        return;
    }
    let settings = load_settings(app);
    if push_agent_config(&settings).await {
        let _ = AGENT_CONFIG_SYNCED.set(());
        info!("持久化设置已启动同步到 Agent 服务（/api/config）");
    }
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

    if push_agent_config(&settings).await {
        // 本进程后续无需再启动同步（配置已在运行时热链上）
        let _ = AGENT_CONFIG_SYNCED.set(());
        info!("配置已热加载到 Agent 服务");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 云端数据库（学习计划历史）：db.rs 的薄壳，每次从 config.yaml 读取连接配置
// ---------------------------------------------------------------------------

/// 计划列表（不含整份计划）：收藏置顶 + 创建时间倒序；query 非空时模糊过滤标题/目标
#[tauri::command]
pub async fn list_plans(query: Option<String>) -> Result<Vec<PlanSummary>, AgentError> {
    let config = load_db_config();
    db::list_plans(&config, query.as_deref().unwrap_or("")).await
}

/// 读取完整计划记录（含整份计划）
#[tauri::command]
pub async fn get_plan(id: String) -> Result<PlanRecord, AgentError> {
    let config = load_db_config();
    db::get_plan(&config, &id).await
}

/// 删除计划（幂等）
#[tauri::command]
pub async fn delete_plan(id: String) -> Result<(), AgentError> {
    let config = load_db_config();
    db::delete_plan(&config, &id).await
}

/// 切换收藏状态，返回切换后的值
#[tauri::command]
pub async fn toggle_favorite(id: String) -> Result<bool, AgentError> {
    let config = load_db_config();
    db::toggle_favorite(&config, &id).await
}

/// 开始学习（打卡日历真数据需求 1）：提交每周学习日，写学习状态 active /
/// 开始日期今天，全部章节按学习日分配落 study_schedule；不打卡今天。
#[tauri::command]
pub async fn start_learning(id: String, days: Vec<u8>) -> Result<(), AgentError> {
    let config = load_db_config();
    let pool = db::pool(&config).await?;
    schedule::start_learning(&pool, db::parse_id(&id)?, &days).await
}

/// 修改学习日（需求 3，仅学习中）：今天之前与今天已完成的日程不动，
/// 今天未完成与未来的章节按新学习日从今天起重排。
#[tauri::command]
pub async fn update_learning_days(id: String, days: Vec<u8>) -> Result<(), AgentError> {
    let config = load_db_config();
    let pool = db::pool(&config).await?;
    schedule::update_learning_days(&pool, db::parse_id(&id)?, &days).await
}

/// 行级打卡开关（需求 7，每章独立勾选；仅允许 date ≤ 今天）。
/// date 为本地日期 'YYYY-MM-DD'，解析失败归一 Db 中文文案。
#[tauri::command]
pub async fn toggle_schedule_item(
    id: String,
    date: String,
    phase_idx: u32,
    chapter_idx: u32,
    done: bool,
) -> Result<(), AgentError> {
    let parsed_date = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .map_err(|_| AgentError::Db(format!("无效的日期：{date}（应为 YYYY-MM-DD）")));
    let config = load_db_config();
    let pool = db::pool(&config).await?;
    schedule::toggle_schedule_item(
        &pool,
        db::parse_id(&id)?,
        parsed_date?,
        phase_idx as i32,
        chapter_idx as i32,
        done,
    )
    .await
}

/// 月历查询（需求 5）：当前月日程按天组织（只含有数据的天）。
#[tauri::command]
pub async fn query_month_calendar(
    year: i32,
    month: u32,
) -> Result<Vec<MonthCalendarDay>, AgentError> {
    let config = load_db_config();
    let pool = db::pool(&config).await?;
    schedule::query_month(&pool, year, month).await
}

/// 年热力图查询（需求 6）：按天聚合 total/done 强度。
#[tauri::command]
pub async fn query_year_heatmap(year: i32) -> Result<Vec<YearHeatmapDay>, AgentError> {
    let config = load_db_config();
    let pool = db::pool(&config).await?;
    schedule::query_year(&pool, year).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::OutlinePhasePayload;

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

    /// Task E：PlanRequest.studyDays serde 语义——缺字段/显式 null 反序列化为
    /// None（serde default 兼容旧载荷）；序列化时 None 归一为全周显式数组
    /// （Python 端 CamelModel 不再兑底 null），Some 透传指定值（wire 名 studyDays）
    #[test]
    fn plan_request_study_days_serde_defaults_full_week() {
        // 1. 缺字段 → None
        let req: PlanRequest = serde_json::from_str(
            r#"{"goal":"学 Rust","level":"beginner","weeklyHours":10,"weeks":8}"#,
        )
        .unwrap();
        assert_eq!(req.study_days, None);
        // 2. None 序列化 → 全周显式数组（转发 Python 的 wire 恒为数组）
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(
            value["studyDays"],
            serde_json::json!([1, 2, 3, 4, 5, 6, 7]),
            "None 序列化应归一为全周数组"
        );
        // 3. 显式 null → None；指定值透传（去重排序由 Python 校验层/DB 层负责）
        let req: PlanRequest = serde_json::from_str(
            r#"{"goal":"g","level":"beginner","weeklyHours":10,"weeks":8,"studyDays":null}"#,
        )
        .unwrap();
        assert_eq!(req.study_days, None);
        let req = PlanRequest {
            study_days: Some(vec![1, 3, 5]),
            ..req
        };
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["studyDays"], serde_json::json!([1, 3, 5]));
    }

    /// 重生成请求体重建（任务 15）：PlanRequestRow 七字段逐值回填，
    /// 序列化 wire 走 camelCase（phaseIndex / chapterIndex）
    #[test]
    fn build_regen_body_refills_request_fields() {
        let meta = PlanRequestRow {
            goal: "学 Rust 并发".into(),
            level: "intermediate".into(),
            weeks: 8,
            weekly_hours: 12,
            interests: vec!["异步".into(), "线程池".into()],
            background: "写过 Go".into(),
            study_days: Some(vec![1, 3, 5]),
        };
        let outline = RegenOutline {
            title: "Rust 并发计划".into(),
            overview: "概览".into(),
            estimated_total_hours: 96,
            phases: vec![OutlinePhasePayload {
                name: "阶段一".into(),
                week_range: "第 1-2 周".into(),
                chapter_titles: vec!["甲章".into(), "乙章".into()],
                project_brief: None,
            }],
        };

        // 全阶段：chapterIndex = None
        let body = build_regen_body(meta.clone(), outline.clone(), 0, None);
        assert_eq!(body.request.goal, "学 Rust 并发");
        assert_eq!(body.request.level, "intermediate");
        assert_eq!(body.request.weeks, 8);
        assert_eq!(body.request.weekly_hours, 12);
        assert_eq!(
            body.request.interests,
            vec!["异步".to_string(), "线程池".to_string()]
        );
        assert_eq!(body.request.background, "写过 Go");
        assert_eq!(
            body.request.study_days,
            Some(vec![1, 3, 5]),
            "studyDays 逐值回填（重生成保持原学习日）"
        );
        assert_eq!(body.phase_index, 0);
        assert_eq!(body.chapter_index, None);
        assert_eq!(body.outline, outline);

        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["phaseIndex"], 0);
        assert!(
            value["chapterIndex"].is_null(),
            "全阶段重生成 chapterIndex 应为 null"
        );

        // 单章：chapterIndex 指定
        let body = build_regen_body(meta, outline, 0, Some(1));
        assert_eq!(body.chapter_index, Some(1));
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["chapterIndex"], 1);
    }

    /// 重生成目标越界校验：phase / chapter 越界直接 Err；边界内通过
    #[test]
    fn validate_regen_targets_bounds() {
        let outline = RegenOutline {
            phases: vec![
                OutlinePhasePayload {
                    name: "阶段一".into(),
                    week_range: "".into(),
                    chapter_titles: vec!["甲章".into()],
                    project_brief: None,
                },
                OutlinePhasePayload {
                    name: "阶段二".into(),
                    week_range: "".into(),
                    chapter_titles: vec![],
                    project_brief: None,
                },
            ],
            ..RegenOutline::default()
        };

        // 边界内：全阶段 + 单章均通过
        assert!(validate_regen_targets(&outline, 0, None).is_ok());
        assert!(validate_regen_targets(&outline, 1, None).is_ok());
        assert!(validate_regen_targets(&outline, 0, Some(0)).is_ok());

        // phase 越界（== len 也算越界）
        assert!(validate_regen_targets(&outline, 2, None).is_err());
        // chapter 越界：甲章只有 1 章，index 1 越界；空章节阶段的 index 0 同样越界
        assert!(validate_regen_targets(&outline, 0, Some(1)).is_err());
        assert!(validate_regen_targets(&outline, 1, Some(0)).is_err());
    }

    /// 进程内最小 WS 服务器：accept 升级后按序发送文本消息，随后正常关闭
    /// （仅服务回放侧，不模拟 Python 业务逻辑；足够验证信封消费循环）。
    async fn mini_ws_server(listener: tokio::net::TcpListener, messages: &[&str]) {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("upgrade");
        use futures_util::SinkExt;
        for m in messages {
            ws.send(Message::Text(m.to_string().into()))
                .await
                .expect("send");
        }
        let _ = ws.close(None).await;
    }

    /// 章级真处理决策快照（R4）：upsert Ok → ChapterDone wire 形态；
    /// upsert Err → StorageIssue；产出永不 PhaseDone（章事件仅推章级
    /// 进度，PhaseDone 时序保持 phase 事件专属，终态集齐口径不扰动）。
    #[test]
    fn chapter_progress_from_outcome_snapshots() {
        // Ok → ChapterDone（wire：tag + camelCase 字段逐字对齐）
        let event = chapter_progress_from_outcome(Ok(()), 2, 1, "乙章");
        assert!(matches!(
            event,
            PlanProgressEvent::ChapterDone {
                phase_index: 2,
                chapter_index: 1
            }
        ));
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "chapterDone");
        assert_eq!(value["phaseIndex"], 2);
        assert_eq!(value["chapterIndex"], 1);

        // Err → StorageIssue（生成不中断，前端降级提示）
        let event = chapter_progress_from_outcome(
            Err(AgentError::Service("连接失败".into())),
            0,
            3,
            "丁章",
        );
        assert!(matches!(event, PlanProgressEvent::StorageIssue { .. }));
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "storageIssue");
        assert_eq!(value["message"], "章节「丁章」未能自动保存：连接失败");

        // 章事件产出永不 PhaseDone（守卫终态计数链）
        assert!(!matches!(
            chapter_progress_from_outcome(Ok(()), 0, 0, "甲"),
            PlanProgressEvent::PhaseDone { .. }
        ));
    }

    /// WS 事件消费 capture sink：记录 (seq, 事件种类) 序列与终态标记
    #[derive(Default)]
    struct CaptureSink {
        events: Vec<(u64, &'static str)>,
        terminal: bool,
    }

    impl WsEventSink for CaptureSink {
        fn handle_event<'a>(&'a mut self, seq: u64, event: StreamEvent) -> WsFuture<'a> {
            Box::pin(async move {
                let tag = match &event {
                    StreamEvent::Outline { .. } => "outline",
                    StreamEvent::Phase { .. } => "phase",
                    StreamEvent::Chapter { .. } => "chapter",
                    StreamEvent::ChapterStart { .. } => "chapterStart",
                    StreamEvent::Done { .. } => "done",
                };
                self.events.push((seq, tag));
                if tag == "done" {
                    self.terminal = true;
                    return Ok(true); // done 即收束
                }
                Ok(false)
            })
        }
    }

    /// WS 信封流消费（任务 22）：outline/phase/done 依次分派、坏消息跳过不
    /// 中断、收到 done 即收束并返回已消费最大 seq（与 Python 信封 {"seq","event"}
    /// 的 JSON 形状逐字段对齐，对照 main/tasks 侧信封构造）。
    #[tokio::test]
    async fn read_ws_events_streams_envelopes_until_done() {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            mini_ws_server(
                listener,
                &[
                    r#"{"seq":1,"event":{"type":"outline","phaseCount":2,"title":"标题"}}"#,
                    "不是 JSON", // 坏消息：应告警跳过，不中断消费
                    r#"{"seq":2,"event":{"type":"phase","index":0,"phase":{"name":"阶段一"}}}"#,
                    r#"{"seq":3,"event":{"type":"done","response":{"status":"plan"}}}"#,
                ],
            )
            .await;
        });

        let url = format!("ws://127.0.0.1:{port}/api/generations/t/events?cursor=0");
        let (stream, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("connect");
        let mut sink = CaptureSink::default();
        let (last_seq, err) = read_ws_events(stream, "test-req", &mut sink).await;
        assert!(err.is_none(), "读循环应正常收束：{err:?}");
        server.await.expect("server task");

        assert_eq!(last_seq, 3, "已消费最大 seq 应为 done 的 seq");
        assert_eq!(
            sink.events,
            vec![(1, "outline"), (2, "phase"), (3, "done")],
            "信封按序分派，坏消息应被跳过"
        );
        assert!(sink.terminal, "done 分派后 sink 应标记终态");
    }

    /// D1 章级扇出后的混合事件流（chapter×N + phase×N + done）消费循环
    /// 验证（R2，只验证不重写）：WS 信封逐条按序分派、章/阶段事件共存
    /// 不中断消费、done 收束——generate 链新线序对消费基础设施是纯增量
    ///（章事件与重生成单章事件同构）。RegenSink 侧分派维持现状：Python
    /// 全阶段重生成仍只产 phase 事件（见 task-d2-report 需裁定）。
    #[tokio::test]
    async fn read_ws_events_handles_mixed_chapter_phase_stream() {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            mini_ws_server(
                listener,
                &[
                    r#"{"seq":1,"event":{"type":"outline","phaseCount":2,"title":"T","phases":[{"name":"甲","chapterTitles":["甲章","乙章"]}]}}"#,
                    r#"{"seq":2,"event":{"type":"chapter","index":0,"phaseIndex":0,"chapter":{"title":"甲章"}}}"#,
                    r#"{"seq":3,"event":{"type":"chapter","index":1,"phaseIndex":0,"chapter":{"title":"乙章"}}}"#,
                    r#"{"seq":4,"event":{"type":"phase","index":0,"phase":{"name":"甲"}}}"#,
                    r#"{"seq":5,"event":{"type":"chapter","index":0,"phaseIndex":1,"chapter":{"title":"丙章"}}}"#,
                    r#"{"seq":6,"event":{"type":"phase","index":1,"phase":{"name":"乙"}}}"#,
                    r#"{"seq":7,"event":{"type":"done","response":{"status":"plan"}}}"#,
                ],
            )
            .await;
        });

        let url = format!("ws://127.0.0.1:{port}/api/generations/t/events?cursor=0");
        let (stream, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("connect");
        let mut sink = CaptureSink::default();
        let (last_seq, err) = read_ws_events(stream, "test-req", &mut sink).await;
        assert!(err.is_none(), "混合流应正常收束：{err:?}");
        server.await.expect("server task");

        assert_eq!(last_seq, 7);
        assert_eq!(
            sink.events,
            vec![
                (1, "outline"),
                (2, "chapter"),
                (3, "chapter"),
                (4, "phase"),
                (5, "chapter"),
                (6, "phase"),
                (7, "done"),
            ],
            "混合流全事件按序分派，章事件不挤丢阶段事件"
        );
        assert!(sink.terminal);
    }

    /// WS 连接提前断开（未收 done）：返回已消费 seq 且不标记终态，由消费方
    /// 按游标重连补齐（重连循环本身的编排依赖 snapshot/DB/Emitter，不在此测）。
    #[tokio::test]
    async fn read_ws_events_returns_consumed_seq_on_premature_close() {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            mini_ws_server(
                listener,
                &[r#"{"seq":1,"event":{"type":"outline","phaseCount":1,"title":"T"}}"#],
            )
            .await;
        });

        let url = format!("ws://127.0.0.1:{port}/api/generations/t/events?cursor=0");
        let (stream, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("connect");
        let mut sink = CaptureSink::default();
        let (last_seq, err) = read_ws_events(stream, "test-req", &mut sink).await;
        assert!(err.is_none(), "读循环应正常收束：{err:?}");
        server.await.expect("server task");

        assert_eq!(last_seq, 1);
        assert!(!sink.terminal, "未收 done 不应标记终态（重连后补齐）");
    }
}

/// `db_config_from_url`：`database.url` → DbConfig 字段（任务 28 语义测试）
/// 启动进度查询：前端启动页轮询此命令，ready=true 后进入主界面。
///
/// 返回的是内存快照（启动序列在 setup 中 spawn，与本命令无 IO 交互），
/// 因此可高频轮询；降级原因随 degraded 字段一起返回，供启动页据实提示。
#[tauri::command]
pub fn startup_status() -> crate::startup::StartupStatus {
    crate::startup::status()
}

#[cfg(test)]
mod tests_db_config_from_url {
    use super::*;
    use crate::config::DbConfig;

    fn config_from(url: &str) -> DbConfig {
        db_config_from_url(url)
    }

    #[test]
    fn full_url_splits_all_fields() {
        let config = config_from("postgres://alice:secret@db.example.com:5433/praxis_prod");
        assert_eq!(config.host, "db.example.com");
        assert_eq!(config.port, 5433);
        assert_eq!(config.database, "praxis_prod");
        assert_eq!(config.user, "alice");
        assert_eq!(config.password, "secret");
    }

    /// config.example.json 模板形态（缺省端口 5432；无密码）
    #[test]
    fn defaults_apply_for_missing_parts() {
        let config = config_from("postgres://postgres@127.0.0.1/praxis");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 5432);
        assert_eq!(config.database, "praxis");
        assert_eq!(config.user, "postgres");
        assert_eq!(config.password, "");

        // 无 dbname：database 空串（同 config.yaml 字段语义）
        let minimal = config_from("postgres://db.local");
        assert_eq!(minimal.host, "db.local");
        assert_eq!(minimal.database, "");
        assert_eq!(minimal.user, "");
    }

    /// 百分号编码（密码含保留字符 `@/:` 等场景）按 UTF-8 解码
    #[test]
    fn percent_encoded_credentials_are_decoded() {
        let config = config_from("postgres://u:p%40ss%3Aword@h/db");
        assert_eq!(config.password, "p@ss:word");

        // 非法 % 序列按字面保留，不毁掉整串
        let rawish = config_from("postgres://u:100%@h/db");
        assert_eq!(rawish.password, "100%");
    }

    /// IPv6 主机（[::1]）不与端口分隔符冲突
    #[test]
    fn ipv6_host_is_supported() {
        let config = config_from("postgres://u:p@[::1]:5432/db");
        assert_eq!(config.host, "::1");
        assert_eq!(config.port, 5432);
        assert_eq!(config.database, "db");
    }

    /// 无 url → 空配置直通（「云端库未配置」StorageIssue 语义保持）；非 postgres scheme → 同样直通
    #[test]
    fn unparseable_urls_fall_back_to_default() {
        assert_eq!(config_from(""), DbConfig::default());
        assert_eq!(config_from("mysql://u:p@h/db"), DbConfig::default());

        let degraded = config_from("postgres://h:99999/db");
        assert_eq!(degraded.host, "", "解析失败应收口为空配置");
    }

    /// 数据库存文件的查询参数剥离（本层不透传；TLS/超时由 db.rs 显式控制）
    #[test]
    fn query_params_are_stripped() {
        let config = config_from("postgres://u:p@h/db?sslmode=require&connect_timeout=3");
        assert_eq!(config.database, "db");
        assert_eq!(config.host, "h", "查询参数不影响主机解析");
    }

    /// 缺省值字段语义：解析成功但 url 只有 scheme 无任何字段——空主机即直通
    #[test]
    fn bare_postgres_url_maps_to_empty_host() {
        assert_eq!(config_from("postgres://"), DbConfig::default());
    }
}
