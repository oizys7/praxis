//! Agent 服务的 HTTP/WS 客户端：统一的请求发送出口。
//!
//! reqwest 客户端无全局超时，本项目按请求单独设置（ping 短超时 / 流式长流按空闲判定）。
//! 所有对 Python 服务的请求都经 [send] 发出，集中处理 base-url 拼接、
//! X-Request-ID 关联头与 JSON body，命令层不再重复拼装。
//! 生成计划的任务化链路（提交 / 快照 / WS 事件通道）也沉淀在本模块。

use std::time::Duration;

use serde::Serialize;

use crate::error::AgentError;
use crate::protocol::{
    GenerationSubmit, GenerationSubmitResponse, GenerationTaskSnapshot, LlmCheckResponse,
};
use crate::sidecar::agent_service_url;

/// 开发模式的默认服务地址（scripts/dev.mjs 启动的 uvicorn）
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:8000";

/// 流式请求的总超时兜底：NDJSON 是长流（重生成全阶段按章串行，单章 LLM
/// 调用可达数分钟），健康判据见 [STREAM_IDLE_TIMEOUT]；这里只防"永不结束"
/// 的病态情况，不再承担按量级的正常时限
pub const STREAM_MAX_TIMEOUT: Duration = Duration::from_secs(3600);

/// NDJSON 逐行空闲超时：超过该时长没有任何新事件即判定流已死。
/// 重生成全阶段路径逐章串行调用 LLM，`chapterStart` 事件本身就是心跳，
/// 用"总超时"衡量必然造成假超时（旧实现 600s 总超时 vs N 章串行）
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(420);
/// 健康检查与配置推送使用短超时，快速探测服务是否在线
pub const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// 生成前 LLM 连通性探测的超时：只发一个极短请求，超过该时长即判定链路异常
pub const LLM_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// reqwest 客户端无全局超时，由每个请求单独设置（见上方两个常量）
pub fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// 请求关联 ID：随 X-Request-ID 头传给 Python，双端日志可按 id 对齐时间线
pub fn new_request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("req-{nanos:016x}")
}

/// 向 Agent 服务发送一个请求。
///
/// - `path`：以 `/` 开头的路径（如 `/api/generate-plan`）；
/// - `timeout`：按端点语义指定（ping 用 PING_TIMEOUT，流式用 STREAM_MAX_TIMEOUT）；
/// - `body`：JSON 请求体（GET 类请求传 `None::<&()>`）；
/// - `req_id`：需要日志关联的请求（生成计划）传入，其余传 None。
pub async fn send<T: Serialize + ?Sized>(
    method: reqwest::Method,
    path: &str,
    timeout: Duration,
    body: Option<&T>,
    req_id: Option<&str>,
) -> Result<reqwest::Response, AgentError> {
    let url = format!("{}{path}", agent_service_url().trim_end_matches('/'));

    let mut request = http_client().request(method, &url).timeout(timeout);
    if let Some(id) = req_id {
        request = request.header("X-Request-ID", id);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.map_err(Into::into)
}

/// 流式 POST：响应体为 NDJSON 长流，`timeout` 沿用 reqwest 总超时语义
/// （覆盖建连到响应体读完）。注意它是**总**超时、不适合给长流做正常时限：
/// 真实守护是命令层的逐行空闲超时（见 [STREAM_IDLE_TIMEOUT]）。
pub async fn post_stream<T: Serialize + ?Sized>(
    path: &str,
    body: &T,
    timeout: Duration,
    req_id: Option<&str>,
) -> Result<reqwest::Response, AgentError> {
    send(reqwest::Method::POST, path, timeout, Some(body), req_id).await
}

// ---------------------------------------------------------------------------
// 生成任务化链路（任务 22）：提交 / 快照 / WS 事件通道
// ---------------------------------------------------------------------------

/// WS 客户端连接（tokio-tungstenite）；TLS 特性未启用，仅 ws:// 明文本地通道
/// （localhost sidecar，无鉴权）。断线重连等编排由命令层负责。
pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 事件通道 URL 构造（纯函数，便于单测）：http(s) -> ws(s) 升级 + cursor 查询串。
/// 已是 ws(s):// 形式的 base 原样保留（开发环境 AGENT_SERVICE_URL 可直接给 ws://）。
fn ws_events_url(base: &str, task_id: &str, cursor: u64) -> String {
    let base = base.trim_end_matches('/');
    let upgraded = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{upgraded}/api/generations/{task_id}/events?cursor={cursor}")
}

/// 连接 WS 事件通道（握手超时沿用 PING_TIMEOUT，短探测快速失败）；
/// 连接后的回放/直播与重连编排由命令层负责。
pub async fn connect_generation_events(task_id: &str, cursor: u64) -> Result<WsStream, AgentError> {
    let url = ws_events_url(&agent_service_url(), task_id, cursor);
    let connect = tokio_tungstenite::connect_async(url.as_str());
    let (stream, _response) = tokio::time::timeout(PING_TIMEOUT, connect)
        .await
        .map_err(|_| {
            AgentError::Service(format!("事件通道握手超时（{} 秒）", PING_TIMEOUT.as_secs()))
        })?
        .map_err(|e| AgentError::Service(format!("事件通道连接失败：{e}")))?;
    Ok(stream)
}

/// 提交后台生成任务（POST /api/generations）：毫秒级返回 {taskId}。
/// 4xx/5xx 归一为业务错误，中文 detail 优先（Python 端 HTTPException 的
/// 429 并发占满 / 422 参数校验均携带可读 message）。
pub async fn submit_plan_generation(
    request: &crate::protocol::PlanRequest,
    req_id: &str,
) -> Result<String, AgentError> {
    let body = GenerationSubmit {
        kind: "plan".into(),
        plan_request: request.clone(),
    };
    let response = send(
        reqwest::Method::POST,
        "/api/generations",
        PING_TIMEOUT,
        Some(&body),
        Some(req_id),
    )
    .await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        // FastAPI HTTPException 产出 {"detail": "中文说明"}；取不出则退回状态码
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(str::to_string));
        return Err(AgentError::Service(
            detail.unwrap_or_else(|| format!("提交生成任务失败：HTTP {status}")),
        ));
    }
    let submitted: GenerationSubmitResponse = response.json().await?;
    Ok(submitted.task_id)
}

/// 生成前探测 LLM 是否可用（Key 有效 / 网络可达 / 模型名存在）。
///
/// 目的：把「Key 填错、模型名写错、网络不通」这类问题在**提交生成之前**
/// 就变成一句可读的提示，而不是让用户在生成流跑到第一次模型调用时才失败。
/// 结论由调用方归一为前置错误（AgentError::Llm），不进入生成逻辑。
pub async fn check_llm() -> Result<LlmCheckResponse, AgentError> {
    let response = send(
        reqwest::Method::POST,
        "/api/llm-check",
        LLM_CHECK_TIMEOUT,
        None::<&()>,
        None,
    )
    .await?;
    if !response.status().is_success() {
        return Err(AgentError::Service(format!(
            "LLM 连通性探测失败：HTTP {}",
            response.status()
        )));
    }
    Ok(response.json::<LlmCheckResponse>().await?)
}

/// 读取任务快照：Some=存在（诊断与游标依据）；None=任务不存在/已驱逐（404）；
/// Err=网络层失败（快照失败不阻断连接尝试，由调用方降级处理）。
pub async fn get_task_snapshot(
    task_id: &str,
    req_id: &str,
) -> Result<Option<GenerationTaskSnapshot>, AgentError> {
    let response = send(
        reqwest::Method::GET,
        &format!("/api/generations/{task_id}/snapshot"),
        PING_TIMEOUT,
        None::<&()>,
        Some(req_id),
    )
    .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AgentError::Service(format!(
            "任务快照获取失败：HTTP {}",
            response.status()
        )));
    }
    Ok(Some(response.json().await?))
}

/// 断线重连游标（纯函数，便于单测）：重连回放下界取「已消费最大 seq」与
/// 快照 lastSeq 的较小者——
/// - 正常路径：lastSeq >= last_consumed，cursor = last_consumed，Python 回放
///   seq > cursor，不丢不重；
/// - 防御（审查残留①）：cursor 不得超过 snapshot.lastSeq（cursor > lastSeq 时
///   Python 侧回放窗口为空、任务已终态会直接关闭且不补发 done），去到 lastSeq
///   避免静默关闭；该不可达状态若真发生，本轮空收由重试耗尽的内容语义收尾兜底
///   （非重放兜底——事件处理幂等仅保证重复回放不脏，非空窗口才生效）。
///   首连（last_consumed=0）恒回 0：全量回放，不丢任何已缓存事件。
pub fn reconnect_cursor(last_consumed: u64, snapshot_last_seq: Option<u64>) -> u64 {
    match snapshot_last_seq {
        Some(last_seq) => last_consumed.min(last_seq),
        None => last_consumed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件通道 URL：http→ws / https→wss 升级、尾斜杠去除与 cursor 查询串
    #[test]
    fn ws_events_url_upgrades_scheme_and_appends_cursor() {
        assert_eq!(
            ws_events_url("http://127.0.0.1:8000", "task-1", 0),
            "ws://127.0.0.1:8000/api/generations/task-1/events?cursor=0"
        );
        assert_eq!(
            ws_events_url("https://agent.example.com/", "task-2", 7),
            "wss://agent.example.com/api/generations/task-2/events?cursor=7"
        );
        // 已是 ws:// 形式（开发环境 AGENT_SERVICE_URL 直接给 ws://）原样保留
        assert_eq!(
            ws_events_url("ws://127.0.0.1:8000", "task-3", 12),
            "ws://127.0.0.1:8000/api/generations/task-3/events?cursor=12"
        );
    }

    /// 断线重连游标（审查残留①防御）：正常路径取已消费最大 seq（Python 回放
    /// seq>cursor，不丢不重）；cursor 超过快照 lastSeq 时鈾到 lastSeq——
    /// 避免超过游标被 Python 静默关闭（该状态下回放窗口为空、已终态连接
    /// 直接关闭且不补 done，本轮空收由重试耗尽走内容语义收尾，非重放兜底）；
    /// 快照缺失退回已消费值；首连（消费 0）恒回 0。
    #[test]
    fn reconnect_cursor_clamps_to_snapshot_last_seq() {
        // 正常重连：不丢不重
        assert_eq!(reconnect_cursor(3, Some(5)), 3);
        assert_eq!(reconnect_cursor(5, Some(5)), 5);
        // 防御：cursor 不得超过快照 lastSeq（cursor>lastSeq 会被 Python 静默关闭）
        assert_eq!(reconnect_cursor(7, Some(5)), 5);
        // 快照缺失（网络失败降级）：退回已消费值
        assert_eq!(reconnect_cursor(3, None), 3);
        // 首连：全量回放
        assert_eq!(reconnect_cursor(0, Some(5)), 0);
        assert_eq!(reconnect_cursor(0, None), 0);
        assert_eq!(reconnect_cursor(0, Some(0)), 0);
    }
}
