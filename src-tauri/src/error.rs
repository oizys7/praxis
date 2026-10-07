//! 错误类型：统一序列化为 { code, message, hint } 供前端展示。
//!
//! message 给技术信息（进日志 / 控制台），hint 给用户可操作的提示。

use serde::Serialize;

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
