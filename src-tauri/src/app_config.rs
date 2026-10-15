//! Praxis 统一配置装载层（config.json，计划 §2.1）。
//!
//! 与既有 config.rs 的分工（避免「AppSettings/config」概念混淆）：
//! - `config.rs`：**旧版** config.yaml（db 段云端库连接，容错直通默认值，迁移期保留）
//! - `app_config.rs`：**新版** config.json（PraxisConfig：database / logging / service /
//!   agent / secrets 五节；任务 28 起 logging/secret/db/sidecar 四个消费点从环境变量迁移到此）
//!
//! 查找顺序（首个命中的文件生效）：
//! 1. `--config <路径>` 命令行参数（sidecar 模式显式指定；main.rs 接线在任务 28）
//! 2. 开发模式：从可执行文件目录向上查工作区标记（`tauri.conf.json` 或含
//!    `src-tauri/tauri.conf.json` 的目录）→ 仓库根 `config.json`
//! 3. 应用数据目录 `config.json`（与 settings.json 同目录，identifier 目录）
//!
//! 找不到文件：错误信息按顺序列出全部候选路径；JSON 坏：错误信息带文件路径 + JSON 点路径。
#![allow(dead_code)] // load_config / config_cli_override 是任务 28 的接线点（logging/secret/db/sidecar），接线后移除

use std::path::{Path, PathBuf};

/// config.json 文件名（各候选目录下同名）
pub(crate) const FILE_NAME: &str = "config.json";

/// 应用数据目录所用的 bundle identifier（与 tauri.conf.json 的 identifier 保持一致；
/// 装载发生在 tauri Builder 拿到 AppHandle 之前，无法走 PathResolver，见 app_data_dir 注释）
pub(crate) const APP_IDENTIFIER: &str = "com.praxis.desktop";

/// config.json 顶层结构。
///
/// serde camelCase 与示例模板一致；未知字段忽略，缺省段落/字段用 Default 值（渐进填充）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PraxisConfig {
    /// 云端 PostgreSQL 连接（url=生产运行时读写；testUrl=测试，cargo test 专用）
    pub database: DatabaseConfig,
    /// 日志（level 全端生效；dir 空 = 各端默认路径）
    pub logging: LoggingConfig,
    /// Agent 服务监听地址（Rust 侧按 host:port 推导请求地址）
    pub service: ServiceConfig,
    /// Agent 冷启动默认（settings.json 仍是运行时权威）
    pub agent: AgentConfig,
    /// 主密钥目录与手动封存密码（空 = secret.rs 现有默认）
    pub secrets: SecretsConfig,
}

/// 云端数据库连接配置（url / testUrl 缺省空串 = 未配置）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DatabaseConfig {
    /// 生产库连接串（运行时读写）
    pub url: String,
    /// 测试库连接串（cargo test 专用，与生产库分离）
    pub test_url: String,
}

/// 日志配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoggingConfig {
    /// 日志级别（如 info / debug）
    pub level: String,
    /// 日志目录；空 = 各端默认路径（Rust 走 app_log_dir，Python 走推导目录）
    pub dir: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            dir: String::new(),
        }
    }
}

/// Agent 服务监听配置（缺省与示例模板一致：127.0.0.1:8000）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceConfig {
    /// 监听地址
    pub host: String,
    /// 监听端口
    pub port: u16,
}

/// 手写 Default：主机/端口非零值默认（127.0.0.1:8000），无法用派生
impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8000,
        }
    }
}

/// Agent 行为配置
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentConfig {
    /// 离线演示模式冷启动默认（settings.json 的 mock 开关仍是运行时权威）
    pub mock_mode: bool,
}

/// 敏感项配置
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretsConfig {
    /// 主密钥目录；空 = secret.rs 现有默认（应用数据目录）
    pub key_dir: String,
    /// 手动封存密码；空 = secret.rs 现有默认（交互生成）
    pub seal_password: String,
}

/// 配置装载错误（Display 为面向用户的中文文案）
#[derive(Debug)]
pub enum AppConfigError {
    /// 未找到 config.json（candidates 按查找顺序列出全部候选路径）
    NotFound { candidates: Vec<PathBuf> },
    /// 文件存在但读取失败
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// JSON 语法或字段类型错误（detail 带 JSON 点路径或行列信息）
    Parse { path: PathBuf, detail: String },
}

impl std::fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppConfigError::NotFound { candidates } => {
                let listed: Vec<String> = candidates
                    .iter()
                    .map(|p| format!("- {}", p.display()))
                    .collect();
                write!(
                    f,
                    "未找到配置文件 config.json：请复制仓库根 config.example.json 为 config.json 并填写。\n已按顺序查找以下位置，均未找到可读的 config.json：\n{}",  // 审查 Finding-4：存在但为目录/不可读的候选也算未命中
                    listed.join("\n")
                )
            }
            AppConfigError::Read { path, source } => {
                write!(f, "配置文件无法读取（{}）：{source}", path.display())
            }
            AppConfigError::Parse { path, detail } => {
                write!(f, "配置文件解析失败（{}）：{detail}", path.display())
            }
        }
    }
}

impl std::error::Error for AppConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppConfigError::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ---- 实现 ----

/// 解析 config.json 文本（纯函数，不碰文件系统与全局状态）。
///
/// 错误 detail 含 JSON 点路径（如 `service.port`）或行列信息；文件路径由
/// 调用方 load_config 包成 AppConfigError::Parse 时补上（本函数无从得知）。
///
/// 容错策略：未知字段忽略（含模板 `_comment` 说明字段）；缺省段落/字段/显式
/// null 一律回退 Default 值（渐进填充）；类型不符才报错（带点路径 + 实际类型）。
pub fn parse_config(raw: &str) -> Result<PraxisConfig, String> {
    if raw.trim().is_empty() {
        return Err("配置文件为空：请参考 config.example.json 填写".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("JSON 语法错误：{e}"))?;
    if !value.is_object() {
        return Err(format!(
            "顶层必须是 JSON 对象，实际是 {}",
            json_kind(&value)
        ));
    }
    let obj = value.as_object().expect("上面已断言 object");
    Ok(PraxisConfig {
        database: parse_database(obj.get("database"))?,
        logging: parse_logging(obj.get("logging"))?,
        service: parse_service(obj.get("service"))?,
        agent: parse_agent(obj.get("agent"))?,
        secrets: parse_secrets(obj.get("secrets"))?,
    })
}

/// 从命令行参数扫描 `--config` 覆盖项：仅识别 `--config <值>` 与 `--config=<值>`
/// 两种形态（首个命中即返回），其余参数一律跳过；`--config` 后无值视为未指定。
pub fn config_cli_override<I>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if let Some(rest) = arg.strip_prefix("--config=") {
            return (!rest.is_empty()).then(|| PathBuf::from(rest));
        }
        if arg == "--config" {
            let value = iter.next()?;
            return (!value.is_empty()).then(|| PathBuf::from(value));
        }
    }
    None
}

/// 开发模式检测：从可执行文件所在目录向上逐级查找工作区标记，命中时返回
/// 仓库根（config.json 所在目录）。两种标记形态：
/// - 某级目录直接含 `tauri.conf.json`（即 src-tauri/）→ 仓库根为其上一级；
/// - 某级目录含 `src-tauri/tauri.conf.json`（即仓库根）→ 该级目录即仓库根。
///
/// release 分发环境无上述标记 → 返回 None，跳过开发模式候选。
pub(crate) fn dev_repo_root(exe_dir: &Path) -> Option<PathBuf> {
    let mut dir = exe_dir.to_path_buf();
    loop {
        if dir.join("tauri.conf.json").is_file() {
            return dir.parent().map(Path::to_path_buf);
        }
        if dir.join("src-tauri").join("tauri.conf.json").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// 应用数据目录（与 settings.json 同目录）：复刻 Tauri `app_data_dir()` 的
/// `dirs::data_dir()/{identifier}` 默认语义（identifier 与 tauri.conf.json 保持一致）。
/// 装载可能发生在 tauri Builder 拿到 AppHandle 之前（任务 28 的 --config 接线在
/// Builder::run 之前扫描参数），无法走 PathResolver，故用同源 dirs crate 推导
/// （未使用 tauri 的 appDirectoriesOverride，本应用未配置该覆盖项）。
pub(crate) fn app_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// 按查找顺序拼出候选路径（保序去重）：① CLI 覆盖 ② 开发模式仓库根 ③ 应用数据目录。
/// 三个来源均可注入（测试不触碰 current_exe / 真实命令行参数）。
pub(crate) fn search_candidates(
    cli: Option<&Path>,
    exe_dir: Option<&Path>,
    data_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = cli {
        candidates.push(p.to_path_buf());
    }
    if let Some(root) = exe_dir.and_then(dev_repo_root) {
        candidates.push(root.join(FILE_NAME));
    }
    if let Some(dir) = data_dir {
        candidates.push(dir.join(FILE_NAME));
    }
    candidates.dedup();
    candidates
}

/// 装载 config.json：按 search_candidates 顺序取首个存在的文件并解析。
/// 文件路径不存在 → NotFound 列出全部候选；文件坏 → Parse/Read 带文件路径。
/// （任务 28 接线 logging/secret/db/sidecar 后成为实际消费入口）
pub fn load_config() -> Result<(PathBuf, PraxisConfig), AppConfigError> {
    let cli = config_cli_override(std::env::args().skip(1));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let data_dir = app_data_dir();
    let candidates = search_candidates(cli.as_deref(), exe_dir.as_deref(), data_dir.as_deref());
    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(path).map_err(|source| AppConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let cfg = parse_config(&raw).map_err(|detail| AppConfigError::Parse {
            path: path.clone(),
            detail,
        })?;
        return Ok((path.clone(), cfg));
    }
    Err(AppConfigError::NotFound { candidates })
}

// ---- 按节解析：从 Value 手动取已知字段，错误信息自带 JSON 点路径 ----

fn parse_database(value: Option<&serde_json::Value>) -> Result<DatabaseConfig, String> {
    let map = section(value, "database")?;
    Ok(DatabaseConfig {
        url: field_string(map, "database.url", "url")?.unwrap_or_default(),
        test_url: field_string(map, "database.testUrl", "testUrl")?.unwrap_or_default(),
    })
}

fn parse_logging(value: Option<&serde_json::Value>) -> Result<LoggingConfig, String> {
    let map = section(value, "logging")?;
    // 缺省回退 Default（模板默认值，非字段型零值）
    let d = LoggingConfig::default();
    Ok(LoggingConfig {
        level: field_string(map, "logging.level", "level")?.unwrap_or(d.level),
        dir: field_string(map, "logging.dir", "dir")?.unwrap_or(d.dir),
    })
}

fn parse_service(value: Option<&serde_json::Value>) -> Result<ServiceConfig, String> {
    let map = section(value, "service")?;
    let d = ServiceConfig::default();
    Ok(ServiceConfig {
        host: field_string(map, "service.host", "host")?.unwrap_or(d.host),
        port: field_u16(map, "service.port", "port")?.unwrap_or(d.port),
    })
}

fn parse_agent(value: Option<&serde_json::Value>) -> Result<AgentConfig, String> {
    let map = section(value, "agent")?;
    Ok(AgentConfig {
        mock_mode: field_bool(map, "agent.mockMode", "mockMode")?.unwrap_or(false),
    })
}

fn parse_secrets(value: Option<&serde_json::Value>) -> Result<SecretsConfig, String> {
    let map = section(value, "secrets")?;
    Ok(SecretsConfig {
        key_dir: field_string(map, "secrets.keyDir", "keyDir")?.unwrap_or_default(),
        seal_password: field_string(map, "secrets.sealPassword", "sealPassword")?
            .unwrap_or_default(),
    })
}

/// 取一节：缺省 / null → None（回退默认值）；非对象 → 带点路径报错
fn section<'a>(
    value: Option<&'a serde_json::Value>,
    name: &str,
) -> Result<Option<&'a serde_json::Map<String, serde_json::Value>>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v @ serde_json::Value::Object(_)) => Ok(Some(v.as_object().expect("已断言 object"))),
        Some(other) => Err(format!(
            "{name}：必须是对象，实际是 {} `{}`",
            json_kind(other),
            truncate(other)
        )),
    }
}

/// 取字符串字段：缺省 / null → None；类型不符 → 带点路径报错
fn field_string(
    map: Option<&serde_json::Map<String, serde_json::Value>>,
    dot_path: &str,
    key: &str,
) -> Result<Option<String>, String> {
    match field_value(map, key) {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(type_error(dot_path, "字符串", other)),
    }
}

/// 取 u16 字段：支持整数越界与浮点两种报错形态
fn field_u16(
    map: Option<&serde_json::Map<String, serde_json::Value>>,
    dot_path: &str,
    key: &str,
) -> Result<Option<u16>, String> {
    match field_value(map, key) {
        None => Ok(None),
        Some(n @ serde_json::Value::Number(num)) if num.is_u64() => {
            u16::try_from(num.as_u64().expect("已断言 u64"))
                .map(Some)
                .map_err(|_| format!("{dot_path}：超出 u16 范围（0-65535），实际是 {n}"))
        }
        Some(other) => Err(type_error(dot_path, "非负整数", other)),
    }
}

/// 取布尔字段：缺省 / null → None；类型不符 → 带点路径报错
fn field_bool(
    map: Option<&serde_json::Map<String, serde_json::Value>>,
    dot_path: &str,
    key: &str,
) -> Result<Option<bool>, String> {
    match field_value(map, key) {
        None => Ok(None),
        Some(serde_json::Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(type_error(dot_path, "布尔值", other)),
    }
}

/// 字段定位：map 缺省 / 字段缺失 / 显式 null → None
fn field_value<'a>(
    map: Option<&'a serde_json::Map<String, serde_json::Value>>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    let value = map.and_then(|m| m.get(key))?;
    (value != &serde_json::Value::Null).then_some(value)
}

fn type_error(dot_path: &str, expected: &str, actual: &serde_json::Value) -> String {
    format!(
        "{dot_path}：期望{expected}，实际是 {} `{}`",
        json_kind(actual),
        truncate(actual)
    )
}

/// Value 的类型中文名（用于中文错误文案）
fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "布尔值",
        serde_json::Value::Number(_) => "数字",
        serde_json::Value::String(_) => "字符串",
        serde_json::Value::Array(_) => "数组",
        serde_json::Value::Object(_) => "对象",
    }
}

/// 错误文案里的实际值摘要（截断到 60 字符，避免巨型字符串刷屏）
fn truncate(value: &serde_json::Value) -> String {
    let raw = value.to_string();
    if raw.chars().count() <= 60 {
        raw
    } else {
        let mut head: String = raw.chars().take(57).collect();
        head.push_str("...");
        head
    }
}
