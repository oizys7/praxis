//! 应用级 YAML 配置（config.yaml，不入库）。
//!
//! 约定：
//! - 文件位置：release 与 exe 同目录分发；开发模式回退到 src-tauri/（CARGO_MANIFEST_DIR）
//! - 此文件被 .gitignore 忽略，机器各自维护；格式模板见 config.example.yaml
//! - db.password 存密文（enc:v1:…，生成方式见 secret.rs 的 manual_seal_password）；
//!   读取时解密为内存明文。写入明文也能工作（容错直用），但不会自动转密文
//! - 修改后重启应用生效（无热加载；连接池按配置指纹自动重建）

use std::path::Path;

use serde::Deserialize;

use crate::secret;

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

/// config.yaml 顶层结构
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 云端数据库（缺省段落 = 未配置，历史功能给出友好提示）
    pub db: DbConfig,
}

// 任务 28 起 config.yaml 链不再被运行时消费（load_db_config 已切 config.json），
// 迁移期整链保留（含配套单测），任务 30 统一清理——防御性 dead_code，非废弃半成品。
#[allow(dead_code)]
impl AppConfig {
    /// 读取配置；文件不存在 / 解析失败时返回默认值（空配置，不报错，保证可用）。
    /// key_dir：主密钥目录（app_data_dir），用于 db.password 解密。
    pub fn load(key_dir: &Path) -> Self {
        for candidate in config_paths() {
            if candidate.exists() {
                return Self::load_from(&candidate, key_dir);
            }
        }
        tracing::info!("未找到 config.yaml，云端数据库未配置");
        Self::default()
    }

    /// 从指定文件加载；db.password（enc:v1: 密文）解密为内存明文
    fn load_from(path: &Path, key_dir: &Path) -> Self {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut cfg = match serde_yaml::from_str::<AppConfig>(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "config.yaml 解析失败，云端数据库视为未配置"
                );
                return Self::default();
            }
        };
        // 密文解密为明文（明文值原样通过，容错直用）
        cfg.db.password = secret::open(key_dir, &cfg.db.password);
        cfg
    }
}

/// 候选路径：exe 同目录优先（release 分发随包携带），其次源码目录（cargo dev）
/// （任务 28 起运行时不再消费，随 AppConfig 链迁移期保留）
#[allow(dead_code)]
fn config_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("config.yaml"));
        }
    }
    paths.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.yaml"));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立临时目录（config 文件 + 主密钥目录，按测试名隔离）
    fn temp_paths(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir()
            .join("praxis-config-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("key")).unwrap();
        (base.join("config.yaml"), base.join("key"))
    }

    #[test]
    fn sealed_password_is_decrypted() {
        let (cfg_path, key_dir) = temp_paths("sealed");
        let sealed = secret::seal(&key_dir, "real-secret").unwrap();
        std::fs::write(
            &cfg_path,
            format!("# 注释保留\ndb:\n  host: 1.2.3.4\n  password: {sealed}\n"),
        )
        .unwrap();

        let loaded = AppConfig::load_from(&cfg_path, &key_dir);
        assert_eq!(loaded.db.password, "real-secret");
        assert_eq!(loaded.db.host, "1.2.3.4");
        let _ = std::fs::remove_dir_all(cfg_path.parent().unwrap());
    }

    #[test]
    fn plain_password_passthrough() {
        let (cfg_path, key_dir) = temp_paths("plain");
        std::fs::write(&cfg_path, "db:\n  host: h\n  password: raw-plain\n").unwrap();
        // 明文（无 enc:v1: 前缀）容错直用，不做任何转换
        let loaded = AppConfig::load_from(&cfg_path, &key_dir);
        assert_eq!(loaded.db.password, "raw-plain");
        let _ = std::fs::remove_dir_all(cfg_path.parent().unwrap());
    }
}
