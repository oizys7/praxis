//! 应用设置：持久化到 Tauri app_data_dir/settings.json，并支持热推送给 Python。
//!
//! 职责：
//! 1. 定义三端协议的 AgentSettings（camelCase，与以下两处严格对齐）：
//!    - frontend/src/types/agent.ts
//!    - agent-service/app/schemas.py（AgentSettingsModel）
//! 2. settings.json 的读写（文件不存在 / 损坏时回退默认值，保证可用性）
//!    - 敏感字段（各提供商 API Key）落盘自动加密（见 crate::secret），
//!      读取时解密；历史明文在加载时自动迁移为密文（明文不留盘）
//! 3. 运行时热链：保存后经 POST /api/config 热推给 Python
//!    （审查 F1 收尾：原冷启动环境变量注入已退役——Python 任务 29 起
//!    零环境变量读取，冷启动只读 config.json，本模块不再注入子进程）

use std::path::Path;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::secret;

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

    // 审查 F1 收尾：sidecar_envs() 已删除——LLM 配置冷启动走 config.json
    // （或设置页首次保存后的 /api/config 热链），不再注入选程 env
}

/// settings.json 与主密钥目录（Tauri app_data_dir 统一管理，全平台一致）
fn paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录：{e}"))?;
    Ok((dir.join("settings.json"), dir))
}

/// 读取设置；文件不存在或损坏时返回默认值（不报错，保证应用可用）。
/// 敏感字段解密为内存明文（IPC / sidecar 使用）；发现历史明文则立即回写密文。
pub fn load_settings(app: &tauri::AppHandle) -> AgentSettings {
    let (path, key_dir) = match paths(app) {
        Ok(v) => v,
        Err(_) => return AgentSettings::with_defaults(),
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return AgentSettings::with_defaults();
    };
    let mut settings = match serde_json::from_str::<AgentSettings>(&content) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "settings.json 解析失败，使用默认配置"
            );
            return AgentSettings::with_defaults();
        }
    };

    let mut had_plain = false;
    unseal_field(&mut settings.openai_api_key, &key_dir, &mut had_plain);
    unseal_field(&mut settings.anthropic_api_key, &key_dir, &mut had_plain);
    unseal_field(&mut settings.deepseek_api_key, &key_dir, &mut had_plain);
    unseal_field(&mut settings.glm_api_key, &key_dir, &mut had_plain);
    unseal_field(&mut settings.opencode_api_key, &key_dir, &mut had_plain);
    if had_plain {
        match save_settings_to_disk(app, &settings) {
            Ok(()) => tracing::info!("settings.json 敏感字段已由明文迁移为加密存储"),
            Err(e) => {
                tracing::warn!(error = %e, "敏感字段加密迁移失败（下次保存时自动重试）")
            }
        }
    }
    settings
}

/// 保存设置到 settings.json（父目录不存在时自动创建）。
/// 敏感字段加密后落盘；内存中的 AgentSettings 始终为明文（供 IPC / sidecar 使用）。
pub fn save_settings_to_disk(
    app: &tauri::AppHandle,
    settings: &AgentSettings,
) -> Result<(), String> {
    let (path, key_dir) = paths(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建设置目录：{e}"))?;
    }
    let sealed = sealed_for_disk(settings, &key_dir)?;
    let content =
        serde_json::to_string_pretty(&sealed).map_err(|e| format!("设置序列化失败：{e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("无法写入设置文件：{e}"))?;
    Ok(())
}

/// 敏感字段加密副本：返回可安全序列化到磁盘的 AgentSettings（密文替换明文）
fn sealed_for_disk(settings: &AgentSettings, key_dir: &Path) -> Result<AgentSettings, String> {
    Ok(AgentSettings {
        openai_api_key: secret::seal(key_dir, &settings.openai_api_key)?,
        anthropic_api_key: secret::seal(key_dir, &settings.anthropic_api_key)?,
        deepseek_api_key: secret::seal(key_dir, &settings.deepseek_api_key)?,
        glm_api_key: secret::seal(key_dir, &settings.glm_api_key)?,
        opencode_api_key: secret::seal(key_dir, &settings.opencode_api_key)?,
        ..settings.clone()
    })
}

/// 解密单个敏感字段；发现历史明文时置 had_plain（触发一次性迁移回写）
fn unseal_field(field: &mut String, key_dir: &Path, had_plain: &mut bool) {
    if !field.is_empty() && !secret::is_sealed(field) {
        *had_plain = true;
    }
    *field = secret::open(key_dir, field);
}
