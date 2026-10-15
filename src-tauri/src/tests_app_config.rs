//! app_config 单元测试：解析容错 / CLI 覆盖解析 / 查找顺序三态。
//!
//! 查找顺序测试不触碰真实环境（current_exe / env args），一律通过可注入的
//! `search_candidates` / `dev_repo_root` / `config_cli_override` 纯函数验证。

use crate::app_config::{
    config_cli_override, dev_repo_root, parse_config, search_candidates, AppConfigError,
    PraxisConfig,
};
use std::path::{Path, PathBuf};

/// 独立临时目录（按用例名 + 进程号隔离，结束后清理）
fn temp_base(tag: &str) -> PathBuf {
    let base = std::env::temp_dir()
        .join("praxis-app-config-test")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

// ---------- parse_config：解析与类型容错 ----------

#[test]
fn empty_string_is_an_error() {
    assert!(parse_config("").is_err());
    assert!(parse_config("   \n ").is_err());
}

#[test]
fn top_level_must_be_object() {
    let err = parse_config("[1,2]").unwrap_err();
    assert!(err.contains("顶层"), "实际错误：{err}");
}

#[test]
fn empty_object_yields_template_defaults() {
    let cfg = parse_config("{}").unwrap();
    let d = PraxisConfig::default();
    assert_eq!(cfg.database.url, d.database.url);
    assert_eq!(cfg.database.test_url, d.database.test_url);
    assert_eq!(cfg.logging.level, "info");
    assert_eq!(cfg.logging.dir, "");
    assert_eq!(cfg.service.host, "127.0.0.1");
    assert_eq!(cfg.service.port, 8000);
    assert!(!cfg.agent.mock_mode);
    assert_eq!(cfg.secrets.key_dir, "");
    assert_eq!(cfg.secrets.seal_password, "");
}

#[test]
fn full_config_parses_every_field() {
    let cfg = parse_config(
        r#"{
            "database": { "url": "postgres://u:p@h:5432/db", "testUrl": "postgres://u:p@h:5432/db_test" },
            "logging": { "level": "debug", "dir": "D:/logs" },
            "service": { "host": "0.0.0.0", "port": 9000 },
            "agent": { "mockMode": true },
            "secrets": { "keyDir": "D:/keys", "sealPassword": "pw" }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.database.url, "postgres://u:p@h:5432/db");
    assert_eq!(cfg.database.test_url, "postgres://u:p@h:5432/db_test");
    assert_eq!(cfg.logging.level, "debug");
    assert_eq!(cfg.logging.dir, "D:/logs");
    assert_eq!(cfg.service.host, "0.0.0.0");
    assert_eq!(cfg.service.port, 9000);
    assert!(cfg.agent.mock_mode);
    assert_eq!(cfg.secrets.key_dir, "D:/keys");
    assert_eq!(cfg.secrets.seal_password, "pw");
}

#[test]
fn unknown_fields_are_ignored() {
    // 未知顶层字段（含 _comment 说明字段）与未知段落字段都容忍
    let cfg = parse_config(
        r#"{
            "_comment": "说明文字",
            "futureSection": { "x": 1 },
            "logging": { "level": "warn", "extra": true }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.logging.level, "warn");
    assert_eq!(cfg.service.port, 8000);
}

#[test]
fn null_field_and_null_section_fall_back_to_defaults() {
    let cfg = parse_config(
        r#"{ "database": null, "logging": { "level": null, "dir": null }, "service": { "port": null } }"#,
    )
    .unwrap();
    assert_eq!(cfg.database.url, "");
    assert_eq!(cfg.logging.level, "info");
    assert_eq!(cfg.logging.dir, "");
    assert_eq!(cfg.service.port, 8000);
}

#[test]
fn missing_sections_fall_back_to_defaults() {
    let cfg = parse_config(r#"{ "service": { "port": 8080 } }"#).unwrap();
    assert_eq!(cfg.service.port, 8080);
    assert_eq!(cfg.service.host, "127.0.0.1");
    assert_eq!(cfg.logging.level, "info");
}

#[test]
fn section_must_be_object() {
    let err = parse_config(r#"{ "database": [] }"#).unwrap_err();
    assert!(err.contains("database"), "实际错误：{err}");
}

#[test]
fn string_type_error_carries_dot_path() {
    let err = parse_config(r#"{ "logging": { "level": 5 } }"#).unwrap_err();
    assert!(err.contains("logging.level"), "实际错误：{err}");
    assert!(err.contains("字符串"), "实际错误：{err}");
}

#[test]
fn port_type_error_carries_dot_path() {
    let err = parse_config(r#"{ "service": { "port": "x" } }"#).unwrap_err();
    assert!(err.contains("service.port"), "实际错误：{err}");
}

#[test]
fn port_out_of_range_is_an_error() {
    let err = parse_config(r#"{ "service": { "port": 70000 } }"#).unwrap_err();
    assert!(err.contains("service.port"), "实际错误：{err}");
}

#[test]
fn port_zero_is_an_error() {
    // 任务 29 审查 F4：0 为预留端口，与 Python _port（1-65535）对齐；
    // 归入本轮任务 28 审查收尾修复
    let err = parse_config(r#"{ "service": { "port": 0 } }"#).unwrap_err();
    assert!(err.contains("service.port"), "实际错误：{err}");
}

#[test]
fn port_float_is_an_error() {
    let err = parse_config(r#"{ "service": { "port": 8000.5 } }"#).unwrap_err();
    assert!(err.contains("service.port"), "实际错误：{err}");
}

#[test]
fn bool_type_error_carries_dot_path() {
    let err = parse_config(r#"{ "agent": { "mockMode": "yes" } }"#).unwrap_err();
    assert!(err.contains("agent.mockMode"), "实际错误：{err}");
}

#[test]
fn test_url_is_independent_from_prod_url() {
    // 只填 testUrl 不影响生产 url（双字段语义：url=生产，testUrl=cargo test 专用）
    let cfg = parse_config(r#"{ "database": { "testUrl": "postgres://t" } }"#).unwrap();
    assert_eq!(cfg.database.test_url, "postgres://t");
    assert_eq!(cfg.database.url, "");
}

#[test]
fn json_syntax_error_reports_position() {
    let err = parse_config("{ \"logging\": }").unwrap_err();
    assert!(err.contains("语法"), "实际错误：{err}");
}

// ---------- config_cli_override：参数形态 ----------

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_override_dash_config_with_value() {
    assert_eq!(
        config_cli_override(args(&["praxis.exe", "--config", "a.json"])),
        Some(PathBuf::from("a.json"))
    );
}

#[test]
fn cli_override_dash_config_equals() {
    assert_eq!(
        config_cli_override(args(&["praxis.exe", "--config=b.json"])),
        Some(PathBuf::from("b.json"))
    );
}

#[test]
fn cli_override_ignores_unrelated_flags() {
    assert_eq!(
        config_cli_override(args(&[
            "praxis.exe",
            "--verbose",
            "--other",
            "v",
            "--config",
            "c.json"
        ])),
        Some(PathBuf::from("c.json"))
    );
}

#[test]
fn cli_override_missing_value_is_none() {
    assert_eq!(config_cli_override(args(&["praxis.exe", "--config"])), None);
}

#[test]
fn cli_override_similar_flag_is_none() {
    assert_eq!(
        config_cli_override(args(&["praxis.exe", "--configuration", "x.json"])),
        None
    );
}

#[test]
fn cli_override_no_args_is_none() {
    assert_eq!(config_cli_override(args(&[])), None);
}

#[test]
fn cli_override_empty_value_is_none() {
    assert_eq!(
        config_cli_override(args(&["praxis.exe", "--config="])),
        None
    );
}

// ---------- dev_repo_root：工作区标记 ----------

#[test]
fn dev_root_found_via_src_tauri_marker() {
    // 布局：repo/src-tauri/tauri.conf.json；exe 在 repo/src-tauri/target/debug 下
    let base = temp_base("dev-src-tauri");
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join("src-tauri/target/debug")).unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{}").unwrap();
    let root = dev_repo_root(&repo.join("src-tauri/target/debug")).unwrap();
    assert_eq!(root, repo);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dev_root_found_via_tauri_conf_json_marker() {
    // 标记形态二：某级目录直接含 tauri.conf.json（即 src-tauri/）→ 仓库根为其上一级
    let base = temp_base("dev-conf-marker");
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join("src-tauri")).unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{}").unwrap();
    let root = dev_repo_root(&repo.join("src-tauri")).unwrap();
    assert_eq!(root, repo);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dev_root_absent_without_marker() {
    let base = temp_base("dev-no-marker");
    assert!(dev_repo_root(&base.join("somewhere")).is_none());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dev_root_absent_for_src_tauri_dir_without_marker_file() {
    // 只有 src-tauri 目录但没有 tauri.conf.json：不算工作区（防误判）
    let base = temp_base("dev-empty-src-tauri");
    std::fs::create_dir_all(base.join("repo/src-tauri")).unwrap();
    assert!(dev_repo_root(&base.join("repo/src-tauri/target/debug")).is_none());
    let _ = std::fs::remove_dir_all(&base);
}

// ---------- search_candidates：查找顺序三态 ----------

#[test]
fn cli_override_wins_over_dev_and_data_dir() {
    let base = temp_base("order-cli");
    let repo = base.join("repo");
    let data = base.join("data");
    std::fs::create_dir_all(repo.join("src-tauri/target/debug")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{}").unwrap();
    std::fs::write(repo.join("config.json"), "{}").unwrap();
    std::fs::write(data.join("config.json"), "{}").unwrap();

    let candidates = search_candidates(
        Some(Path::new("override.json")),
        Some(&repo.join("src-tauri/target/debug")),
        Some(&data),
    );
    assert_eq!(candidates[0], PathBuf::from("override.json"));
    assert_eq!(candidates[1], repo.join("config.json"));
    assert_eq!(candidates[2], data.join("config.json"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dev_root_config_used_without_cli() {
    let base = temp_base("order-dev");
    let repo = base.join("repo");
    let data = base.join("data");
    std::fs::create_dir_all(repo.join("src-tauri/target/debug")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{}").unwrap();

    let candidates = search_candidates(
        None,
        Some(&repo.join("src-tauri/target/debug")),
        Some(&data),
    );
    assert_eq!(candidates[0], repo.join("config.json"));
    assert_eq!(candidates[1], data.join("config.json"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn data_dir_is_fallback_without_cli_and_dev_marker() {
    let base = temp_base("order-data");
    let plain = base.join("plain"); // 无任何工作区标记
    let data = base.join("data");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let candidates = search_candidates(None, Some(&plain), Some(&data));
    assert_eq!(candidates, vec![data.join("config.json")]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn not_found_error_lists_all_candidates() {
    // 无 CLI / 无工作区 / 无数据目录文件：错误按顺序列出三个候选路径
    let base = temp_base("order-missing");
    let repo = base.join("repo");
    let data = base.join("data");
    std::fs::create_dir_all(repo.join("src-tauri/target/debug")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{}").unwrap();

    let candidates = search_candidates(
        None,
        Some(&repo.join("src-tauri/target/debug")),
        Some(&data),
    );
    let err = AppConfigError::NotFound {
        candidates: candidates.clone(),
    };
    let msg = err.to_string();
    assert!(msg.contains("config.example.json"), "实际错误：{msg}");
    for p in &candidates {
        assert!(
            msg.contains(p.display().to_string().as_str()),
            "缺候选路径：{}",
            p.display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn duplicate_candidates_are_collapsed() {
    // CLI 覆盖路径与数据目录路径相同：去重保序
    let base = temp_base("order-dup");
    let data = base.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let same = data.join("config.json");
    let candidates = search_candidates(Some(&same), None, Some(&data));
    assert_eq!(candidates, vec![same]);
    let _ = std::fs::remove_dir_all(&base);
}
