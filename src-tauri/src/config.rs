//! 云端 PostgreSQL 连接配置的内存形态。
//!
//! 历史（2026-10-22 架构清理）：本模块原承载 config.yaml 的完整装载链
//! （`AppConfig::load` / `config_paths` / 密码解密）。任务 28 起运行时配置
//! 统一走 config.json（见 app_config.rs），那条链已无任何消费方——它带着
//! `#[allow(dead_code)]` 在迁移期被保留了很久，此次连同配套单测一并删除。
//!
//! 保留的 `DbConfig` 仍是活的：由 `commands::db_config_from_url` 从
//! config.json 的 `database.url` 解析产生，db.rs 与各命令广泛使用。

use serde::Deserialize;

/// 云端 PostgreSQL 连接配置（学习计划持久化；PartialEq 供测试断言用）
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
}
