//! 敏感字段落盘加密（API Key / 数据库密码）。
//!
//! 方案：AES-256-GCM 对称加密；主密钥（32 字节）不落 settings.json，
//! 单独存放于应用数据目录下的 master.key（路径由 Tauri 统一管理，全平台一致）。
//! 密文格式 `enc:v1:<base64(nonce || ciphertext)>`；无前缀的历史明文在读取时
//! 原样返回，由 settings 层在加载时自动迁移为密文（明文不留盘）。

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

/// 密文前缀：识别「已加密」与「历史明文」
pub const PREFIX: &str = "enc:v1:";
/// 主密钥文件名（与 settings.json 同目录）
const KEY_FILE: &str = "master.key";
const NONCE_LEN: usize = 12;

/// 判断字符串是否为密文格式
pub fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

/// 加密；空串原样返回（空值不产生密文，避免无意义膨胀）。
/// key_dir：主密钥所在目录（应用数据目录，与 settings.json 同级）。
pub fn seal(key_dir: &Path, plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
    use aes_gcm::Aes256Gcm;

    let cipher = Aes256Gcm::new_from_slice(&master_key(key_dir)?)
        .map_err(|e| format!("加密初始化失败：{e}"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plain.as_bytes())
        .map_err(|e| format!("加密失败：{e}"))?;

    let mut packed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    packed.extend_from_slice(nonce.as_slice());
    packed.extend_from_slice(&ciphertext);
    Ok(format!("{PREFIX}{}", BASE64.encode(packed)))
}

/// 解密；无前缀视为历史明文，原样返回；密钥丢失 / 密文损坏返回空串（不
/// panic，应用以空配置继续运行，用户在设置页重新填写即可）。
pub fn open(key_dir: &Path, stored: &str) -> String {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let Some(encoded) = stored.strip_prefix(PREFIX) else {
        return stored.to_string();
    };
    let Ok(packed) = BASE64.decode(encoded) else {
        return String::new();
    };
    let Some(nonce_bytes) = packed.get(..NONCE_LEN) else {
        return String::new();
    };
    let Ok(cipher) = Aes256Gcm::new_from_slice(&master_key(key_dir).unwrap_or_default()) else {
        return String::new();
    };
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), &packed[NONCE_LEN..])
        .ok()
        .and_then(|plain| String::from_utf8(plain).ok())
        .unwrap_or_default()
}

/// 主密钥（32 字节）：已存在则读取，否则生成并写入（全平台同一逻辑）。
fn master_key(key_dir: &Path) -> Result<[u8; 32], String> {
    let path = key_dir.join(KEY_FILE);
    if let Ok(encoded) = std::fs::read_to_string(&path) {
        return decode_key(encoded.trim());
    }
    let key = rand::random::<[u8; 32]>();
    std::fs::create_dir_all(key_dir).map_err(|e| format!("无法创建密钥目录：{e}"))?;
    std::fs::write(&path, BASE64.encode(key)).map_err(|e| format!("无法写入密钥文件：{e}"))?;
    // Unix 权限加固（Windows 目录权限由系统默认继承，无需处理）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

/// base64 -> 32 字节密钥（长度不符报错）
fn decode_key(encoded: &str) -> Result<[u8; 32], String> {
    let bytes = BASE64
        .decode(encoded.trim())
        .map_err(|_| "主密钥格式损坏".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "主密钥长度异常（期望 32 字节）".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 独立临时目录（按测试名隔离，避免共用密钥互相干扰）
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("praxis-secret-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn seal_open_roundtrip() {
        let dir = temp_dir("roundtrip");
        let sealed = seal(&dir, "sk-abc123").unwrap();
        assert!(is_sealed(&sealed));
        assert_ne!(sealed, "sk-abc123"); // 落盘的不是明文
        assert_eq!(open(&dir, &sealed), "sk-abc123");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_stays_empty() {
        let dir = temp_dir("empty");
        assert_eq!(seal(&dir, "").unwrap(), "");
        assert_eq!(open(&dir, ""), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_text_passthrough() {
        let dir = temp_dir("plain");
        // 历史明文（无前缀）原样返回，交由 settings 层触发迁移
        assert!(!is_sealed("sk-legacy"));
        assert_eq!(open(&dir, "sk-legacy"), "sk-legacy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_yields_empty_not_panic() {
        let a = temp_dir("key-a");
        let b = temp_dir("key-b");
        let sealed = seal(&a, "secret").unwrap();
        // 换了密钥目录（模拟主密钥丢失）：返回空串而非 panic
        assert_eq!(open(&b, &sealed), "");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// 手动工具（不进常规测试流）：生成密文密码供 config.yaml db.password 兼容层使用
    /// （config.json 敏感项是明文，不需要本工具；仅迁移期兼容旧 yaml 链时可能用到）。
    /// 任务 28 弃用 PRAXIS_KEY_DIR / PRAXIS_SEAL_PASSWORD 环境变量，参数改从 config.json
    /// 的 secrets 节供给（keyDir 空 = 应用数据目录默认；sealPassword 空 = 未填）。
    /// 用法：
    ///   1. config.json 填好 secrets.keyDir / secrets.sealPassword
    ///   2. cargo test manual_seal_password -- --ignored --nocapture
    ///
    /// 输出的 `password: enc:v1:…` 行直接粘贴到 config.yaml。
    #[test]
    #[ignore = "手动工具：需 config.json secrets.keyDir / secrets.sealPassword 已填写"]
    fn manual_seal_password() {
        let (_, config) = crate::app_config::load_config()
            .expect("config.json 装载失败：请先在查找路径之一放好配置（见模块/API 文档）");
        let key_dir = if config.secrets.key_dir.trim().is_empty() {
            crate::app_config::app_data_dir()
                .expect("无法定位应用数据目录：请改在 config.json secrets.keyDir 显式填写")
        } else {
            PathBuf::from(&config.secrets.key_dir)
        };
        let plain = if config.secrets.seal_password.is_empty() {
            panic!("请在 config.json secrets.sealPassword 填入手动封存密码");
        } else {
            config.secrets.seal_password.clone()
        };
        let sealed = seal(&key_dir, &plain).expect("加密失败");
        println!("password: {sealed}");
    }
}
