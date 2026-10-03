//! 应用设置：持久化到 Tauri app_data_dir/settings.json，并支持热推送给 Python。
//!
//! 职责：
//! 1. 定义三端协议的 AgentSettings（camelCase，与以下两处严格对齐）：
//!    - frontend/src/types/agent.ts
//!    - agent-service/app/schemas.py（AgentSettingsModel）
//! 2. settings.json 的读写（文件不存在 / 损坏时回退默认值，保证可用性）
//! 3. 生成注入 sidecar 子进程的环境变量（冷启动即生效；运行中由 /api/config 热更）

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Agent 运行配置。
///
/// `#[serde(default)]`（容器级）：旧版本 settings.json 缺字段时可正常加载，
/// 新增字段天然向后兼容。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettings {
    /// 当前提供商："openai" | "anthropic" | "deepseek" | "glm" | "opencode"
    pub llm_provider: String,
    /// 通用 OpenAI
    pub openai_api_key: String,
    pub openai_model: String,
    /// Anthropic（Claude）
    pub anthropic_api_key: String,
    pub anthropic_model: String,
    /// DeepSeek（官方 SDK）
    pub deepseek_api_key: String,
    pub deepseek_model: String,
    /// 智谱 GLM
    pub glm_api_key: String,
    pub glm_model: String,
    /// OpenCode Go 套餐（OpenAI 兼容端点）
    pub opencode_api_key: String,
    pub opencode_model: String,
    /// 离线演示模式：不调用 LLM，返回确定性 mock 计划
    pub allow_mock_llm: bool,
}

impl AgentSettings {
    /// 首次使用（无 settings.json）时的默认值：模型名与 Python 端 Settings 默认一致
    pub fn with_defaults() -> Self {
        Self {
            llm_provider: "openai".to_string(),
            openai_model: "gpt-4o-mini".to_string(),
            anthropic_model: "claude-3-5-sonnet-latest".to_string(),
            deepseek_model: "deepseek-chat".to_string(),
            glm_model: "glm-5.3-flash".to_string(),
            opencode_model: "grok-4.7".to_string(),
            ..Self::default()
        }
    }

    /// 注入 sidecar 子进程的环境变量（只注入非空值，避免清掉 dev 环境已有的配置）
    pub fn sidecar_envs(&self) -> Vec<(String, String)> {
        let mock = if self.allow_mock_llm { "1" } else { "0" };
        let mut vars = vec![
            ("LLM_PROVIDER", self.llm_provider.as_str()),
            ("OPENAI_API_KEY", self.openai_api_key.as_str()),
            ("OPENAI_MODEL", self.openai_model.as_str()),
            ("ANTHROPIC_API_KEY", self.anthropic_api_key.as_str()),
            ("ANTHROPIC_MODEL", self.anthropic_model.as_str()),
            ("DEEPSEEK_API_KEY", self.deepseek_api_key.as_str()),
            ("DEEPSEEK_MODEL", self.deepseek_model.as_str()),
            ("GLM_API_KEY", self.glm_api_key.as_str()),
            ("GLM_MODEL", self.glm_model.as_str()),
            ("OPENCODE_API_KEY", self.opencode_api_key.as_str()),
            ("OPENCODE_MODEL", self.opencode_model.as_str()),
            ("PRAXIS_ALLOW_MOCK_LLM", mock),
        ];
        vars.retain(|(_, value)| !value.is_empty());
        vars.into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
}

/// settings.json 路径：{app_data_dir}/settings.json（Tauri 统一管理：Roaming + 应用 identifier）
fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))?;
    Ok(dir.join("settings.json"))
}

/// 读取设置；文件不存在或损坏时返回默认值（不报错，保证应用可用）
pub fn load_settings(app: &tauri::AppHandle) -> AgentSettings {
    let Ok(path) = settings_path(app) else {
        return AgentSettings::with_defaults();
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "settings.json 解析失败，使用默认配置"
            );
            AgentSettings::with_defaults()
        }),
        Err(_) => AgentSettings::with_defaults(),
    }
}

/// 保存设置到 settings.json（父目录不存在时自动创建）
pub fn save_settings_to_disk(
    app: &tauri::AppHandle,
    settings: &AgentSettings,
) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建设置目录：{e}"))?;
    }
    let content =
        serde_json::to_string_pretty(settings).map_err(|e| format!("设置序列化失败：{e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("无法写入设置文件：{e}"))?;
    Ok(())
}
