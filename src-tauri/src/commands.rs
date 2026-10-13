//! Tauri commands：前端 `invoke()` 的入口层。
//!
//! 多数命令职责仅限「参数接收 + 委托 + 结果透传」，不承载细节逻辑：
//! - HTTP 转发与超时策略见 agent_client；
//! - sidecar 生命周期见 sidecar；
//! - 错误归一与序列化见 error；数据模型见 protocol。
//!
//! 例外是 generate_plan / regenerate_phase / regenerate_chapter：流式消费 +
//! 进度推送 + 自动入库的编排枢纽（共享 consume_ndjson_stream 骨架，见函数文档）。
//!
//! 数据结构与以下两处严格对齐（字段均为 camelCase）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;
use tauri::ipc::{Channel, JavaScriptChannelId};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::agent_client::{new_request_id, post_stream, send, PING_TIMEOUT, PLAN_TIMEOUT};
use crate::db;
use crate::error::AgentError;
use crate::plan_store::{self, PlanRequestRow};
use crate::protocol::{
    parse_ndjson_line, PlanProgressEvent, PlanRecord, PlanRequest, PlanResponse, PlanSummary,
    RegenOutline, RegenRequest, SavePlanRequest, StreamEvent,
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
    //    reqwest 请求侧同值总超时兜底（覆盖到响应体读完）；
    //    分行/解析/非 2xx 校验基础设施提炼为 consume_ndjson_stream 共享骨架
    //    （与 regenerate 双命令同源，防双份漂移），事件分派沉淀在 GeneratePlanSink
    let consumed = match post_stream(
        "/api/generate-plan/stream",
        &request,
        PLAN_TIMEOUT,
        Some(&req_id),
    )
    .await
    {
        Ok(mut response) => {
            let mut sink = GeneratePlanSink {
                storage: storage.as_ref(),
                progress,
                req_id: &req_id,
                weekly_hours: request.weekly_hours,
                phase_count: 0,
            };
            match tokio::time::timeout(
                PLAN_TIMEOUT,
                consume_ndjson_stream(&mut response, &req_id, &mut sink),
            )
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
        Ok(Some(envelope)) => {
            info!(
                %req_id,
                status = %envelope.status,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "学习计划流式生成完成"
            );
            Ok(envelope)
        }
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
    while let Some(line) = next_line(response, &mut buffer).await? {
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

/// generate_plan 的事件分派（原内联 match 提炼为 sink，与 regenerate 同走
/// [consume_ndjson_stream]）：outline → 元信息更新 + 骨架播种；phase →
/// upsert + PhaseDone；chapter → 非预期事件 warn 忽略防错；done → 终态入库
/// 收尾（settle_terminal）+ 回传包络。
struct GeneratePlanSink<'a> {
    storage: Option<&'a Storage>,
    progress: Option<&'a Channel<PlanProgressEvent>>,
    req_id: &'a str,
    /// 预计总学时估算的每周学时（outline 事件到达时 × 阶段数，粗估供列表页展示）
    weekly_hours: u32,
    /// PhaseDone 推送的阶段总数（outline 事件到达时更新）
    phase_count: u32,
}

impl StreamEventSink for GeneratePlanSink<'_> {
    fn handle_event<'a>(&'a mut self, event: StreamEvent) -> SinkFuture<'a> {
        Box::pin(async move {
            match event {
                StreamEvent::Outline {
                    phase_count: count,
                    title,
                    overview,
                    phases,
                } => {
                    self.phase_count = count;
                    if let Some(s) = self.storage {
                        // 预计总学时：大纲阶段数 × 每周学时（粗估，供云端列表页展示）
                        let hours = i64::from(self.weekly_hours).saturating_mul(i64::from(count));
                        if let Err(e) =
                            plan_store::update_outline_meta(&s.pool, s.plan_id, &title, hours).await
                        {
                            warn!(%self.req_id, error = %e, "计划大纲元信息入库失败，不影响生成");
                            notify(
                                self.progress,
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
                                warn!(%self.req_id, error = %e, "大纲骨架播种失败，不影响生成");
                                notify(
                                    self.progress,
                                    PlanProgressEvent::StorageIssue {
                                        message: format!("大纲骨架未能自动保存：{e}"),
                                    },
                                );
                            }
                        }
                    }
                    notify(
                        self.progress,
                        PlanProgressEvent::Outline {
                            phase_count: count,
                            title,
                            overview,
                            phases,
                        },
                    );
                    Ok(None)
                }
                StreamEvent::Phase { index, phase } => {
                    if let Some(s) = self.storage {
                        // Python 端 index 为小非负整数；防御性越界按 0 处理（实际不可达）
                        let idx = i32::try_from(index).unwrap_or(0);
                        if let Err(e) =
                            plan_store::upsert_phase(&s.pool, s.plan_id, idx, &phase).await
                        {
                            warn!(%self.req_id, index, error = %e, "阶段入库失败，不中断生成流");
                            notify(
                                self.progress,
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
                        self.progress,
                        PlanProgressEvent::PhaseDone {
                            index,
                            total: self.phase_count,
                        },
                    );
                    Ok(None)
                }
                // 初生链路不应出现 chapter 事件（单章重生成专属，任务 14）；
                // 出现即协议异常，忽略防错
                StreamEvent::Chapter {
                    index, phase_index, ..
                } => {
                    warn!(%self.req_id, index, phase_index, "初生流出现非预期 chapter 事件，已忽略");
                    Ok(None)
                }
                StreamEvent::Done { response: envelope } => {
                    if let Some(s) = self.storage {
                        if let Err(e) = settle_terminal(&s.pool, s.plan_id, &envelope.status).await
                        {
                            warn!(%self.req_id, error = %e, "终态入库收尾失败，不影响生成结果返回");
                            notify(
                                self.progress,
                                PlanProgressEvent::StorageIssue {
                                    message: format!("生成结果未能按终态入库：{e}"),
                                },
                            );
                        }
                    }
                    Ok(Some(envelope))
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
        },
        outline,
        phase_index,
        chapter_index,
    }
}

/// regenerate 流事件分派：全阶段路径（chapter_index=None）处理 phase 事件
/// （upsert_phase → PhaseDone{index, total=阶段总数}）；单章路径处理 chapter
/// 事件（upsert_chapter → PhaseDone{index=章节索引, total=1}，阶段联动由
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
    /// PhaseDone 的 total：全阶段=计划阶段总数；单章=1
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
                } if self.chapter_index.is_some() => {
                    // 守卫已确保单章路径；目标索引以命令入口校验后的值为准
                    // （事件值由 Python 原样回显，仅作 PhaseDone 显示）
                    let Some(ci) = self.chapter_index else {
                        return Ok(None);
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
                    }
                    notify(
                        self.progress,
                        PlanProgressEvent::PhaseDone { index, total: 1 },
                    );
                    Ok(None)
                }
                StreamEvent::Phase { index, .. } => {
                    warn!(%self.req_id, index, "单章重生成流出现非预期 phase 事件，已忽略");
                    Ok(None)
                }
                StreamEvent::Chapter {
                    index, phase_index, ..
                } => {
                    warn!(%self.req_id, index, phase_index, "全阶段重生成流出现非预期 chapter 事件，已忽略");
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
/// ②目标行置 pending（前端即时转「生成中」）；
/// ③POST /api/regenerate/stream 并消费流（共享 consume_ndjson_stream 骨架）；
/// ④done(plan) → 计划行守卫收尾（finalize_regen_complete：failed 行不强推
/// complete，裁定 3）；done(error) → finalize_failed（pending 行翻 failed，
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
    ensure_agent_running(&app).await?;

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
    let config = load_db_config(&app);
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

    // ② 目标行置 pending（前端即时可见；失败仅提示不阻塞——与 generate_plan
    //    入库容错口径一致，后续成功入库 / 失败收尾均会覆写状态）
    let mark_result = match chapter_index {
        None => plan_store::mark_phase_pending(&pool, plan_id, phase_index).await,
        Some(ci) => plan_store::mark_chapter_pending(&pool, plan_id, phase_index, ci).await,
    };
    if let Err(e) = mark_result {
        warn!(%req_id, error = %e, "重生成目标状态置 pending 失败，不影响生成");
        notify(
            progress,
            PlanProgressEvent::StorageIssue {
                message: format!("重生成状态未能即时更新：{e}"),
            },
        );
    }

    // ③ 请求 + 消费：总超时（PLAN_TIMEOUT，与生成同值，单阶段/单章 + 入库余量足够）
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
    let consumed = match post_stream("/api/regenerate/stream", &body, PLAN_TIMEOUT, Some(&req_id))
        .await
    {
        Ok(mut response) => {
            match tokio::time::timeout(
                PLAN_TIMEOUT,
                consume_ndjson_stream(&mut response, &req_id, &mut sink),
            )
            .await
            {
                Ok(inner) => inner,
                Err(_) => {
                    error!(%req_id, timeout_secs = PLAN_TIMEOUT.as_secs(), "重生成流式消费总超时");
                    Err(AgentError::Service(format!(
                        "重生成超时（{} 秒），请稍后重试",
                        PLAN_TIMEOUT.as_secs()
                    )))
                }
            }
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
        // done(plan)：计划行守卫收尾（failed 行不强推 complete，任务 15 裁定 3）
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
}
