//! Agent 服务的 HTTP 客户端：统一的请求发送出口。
//!
//! reqwest 客户端无全局超时，本项目按请求单独设置（ping 短超时 / plan 长超时）。
//! 所有对 Python 服务的请求都经 [send] 发出，集中处理 base-url 拼接、
//! X-Request-ID 关联头与 JSON body，命令层不再重复拼装。

use std::time::Duration;

use serde::Serialize;

use crate::error::AgentError;
use crate::sidecar::agent_service_url;

/// 开发模式的默认服务地址（scripts/dev.mjs 启动的 uvicorn）
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:8000";

/// 生成计划涉及多次 LLM 调用（规划 + 评审 + 可能的一次修订），给足超时时间
pub const PLAN_TIMEOUT: Duration = Duration::from_secs(600);
/// 健康检查与配置推送使用短超时，快速探测服务是否在线
pub const PING_TIMEOUT: Duration = Duration::from_secs(3);

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
/// - `timeout`：按端点语义指定（ping 用 PING_TIMEOUT，生成计划用 PLAN_TIMEOUT）；
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
