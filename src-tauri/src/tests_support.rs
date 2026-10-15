//! 测试进程配置适配器（仅 #[cfg(test)] 参与，运行时零影响）。
//!
//! 自任务 28 起 src-tauri 运行时配置全部来自 config.json（环境变量内容读取清零）；
//! 唯一例外是 `#[sqlx::test]` 宏——它由框架强制从环境变量读 `DATABASE_URL`，行为不可改。
//! 本模块在**测试进程启动时**（ctor 钩子）读 config.json 的 `database.testUrl`
//! 灌入 `DATABASE_URL`，作为这一处框架桥接的唯一适配点：
//! 测试库与生产库连接串都持久化在 config 文件里，换库改文件不改命令行。
//!
//! 集成测试运行方式（tests_plan_store）：在 config.json 填好
//! `database.testUrl` 后直接 `cargo test -- --ignored`，无需再手工 export。
//! config.json 缺失 / testUrl 空：桥接静默跳过（eprintln 提示），依赖库的
//! 集成测试自身会因缺库失败——不影响其它单测。

use crate::app_config::{load_config, PraxisConfig};

/// 测试进程启动钩子：一次性桥接 config.json → DATABASE_URL 桥接
#[ctor::ctor]
fn bridge_test_db_url() {
    let config = match load_config() {
        Ok((_, config)) => config,
        Err(message) => {
            eprintln!("tests_support：config.json 未装载（{message}）");
            return;
        }
    };
    let Some(test_db_url) = test_db_url(&config) else {
        eprintln!(
            "tests_support：config.json 的 database.testUrl 为空，跳过 DATABASE_URL 框架桥接；\
             依赖集成测试库的用例（cargo test -- --ignored）将因无库而失败"
        );
        return;
    };
    // 仅测试进程内为 #[sqlx::test] 宏桥接 config.json——框架强制读 env
    //（运行时 src/ 零 env 读取的豁免点，见计划「环境变量 → 去向表」§2.3）
    std::env::set_var("DATABASE_URL", test_db_url);
}

/// 由 config 提取测试库连接串：testUrl 非空返回 Some，空串返回 None
fn test_db_url(config: &PraxisConfig) -> Option<String> {
    (!config.database.test_url.trim().is_empty()).then(|| config.database.test_url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_test_url_yields_none() {
        let mut config = PraxisConfig::default();
        assert!(test_db_url(&config).is_none());
        config.database.test_url = "   ".to_string();
        assert!(test_db_url(&config).is_none(), "纯空白视同未配置");
    }

    #[test]
    fn filled_test_url_yields_connection() {
        let mut config = PraxisConfig::default();
        config.database.test_url =
            "postgres://postgres:postgres@127.0.0.1:5432/praxis_test".to_string();
        assert_eq!(
            test_db_url(&config).as_deref(),
            Some("postgres://postgres:postgres@127.0.0.1:5432/praxis_test")
        );
    }
}
