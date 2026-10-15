// Praxis 桌面应用入口
//
// 在 release 模式下隐藏 Windows 控制台窗口（Tauri 官方模板标准写法，勿删）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_client;
mod app_config;
mod commands;
mod config;
mod db;
mod db_ddl;
mod error;
mod logging;
mod plan_store;
mod protocol;
mod secret;
mod settings;
mod sidecar;

#[cfg(test)]
mod tests_app_config;
// plan_store 集成测试（需 config.json database.testUrl 桥接 DATABASE_URL，任务 28 起，见模块文档）
#[cfg(test)]
mod tests_plan_store;
// 测试进程配置适配器：启动时把 config.json testUrl 灌入 DATABASE_URL（sqlx 宏强制读 env 的唯一桥接点）
#[cfg(test)]
mod tests_support;

fn main() {
    // 统一配置装载（任务 28 接线）：先于 Builder（setup 内的日志初始化与四个消费点都依赖它）。
    // --config 覆盖由 load_config 内部自扫 std::env::args（CLI 参数语义，非内容 env 读取）。
    // 失败绝不 panic：桌面应用必须能起。eprintln 保 dev 终端可见，文案暂存供 setup 钩子
    // 弹窗链提示；应用按「无配置降级」运行：logging/secret 用默认值，DB 相关功能
    // 沿用现有 StorageIssue 直通语义。
    match app_config::load_config() {
        Ok(loaded) => app_config::store_global_config(loaded),
        Err(error) => {
            eprintln!("{error}");
            app_config::store_load_error(error.to_string());
        }
    }

    tauri::Builder::default()
        // 注册前端可 invoke 的命令：
        // - generate_plan：转发学习计划生成请求到 Python Agent 服务
        // - regenerate_phase / regenerate_chapter：阶段/章节级重生成（任务 15）
        // - ping_agent：健康检查，用于前端展示服务在线状态
        .invoke_handler(tauri::generate_handler![
            commands::generate_plan,
            commands::regenerate_phase,
            commands::regenerate_chapter,
            commands::ping_agent,
            commands::get_settings,
            commands::save_settings,
            commands::save_plan,
            commands::list_plans,
            commands::get_plan,
            commands::delete_plan,
            commands::toggle_favorite,
        ])
        .setup(|app| {
            use tauri::Manager;
            // 配置装载失败的延迟提示（main() 已 eprintln；setup 阶段弹窗链补 UI 可见性，
            // 文案含三候选路径）。降级策略：不阻断启动，logging/secret 用默认值。
            if let Some(message) = app_config::load_error() {
                tracing::error!("{message}");
                let dialog = format!(
                    "Praxis 配置装载失败\n\n{message}\n\n应用将以「无配置」模式降级运行：\n日志/主密钥用默认值，云端库相关功能暂不可用。"
                );
                // 弹窗经主窗口 alert（桌面端原生 WebView 消息框，零新依赖）；
                // serde_json 序列化防止文案中的引号/换行破坏 JS 字面量
                if let Some(main_window) = app.get_webview_window("main") {
                    let script = serde_json::to_string(&dialog)
                        .map(|literal| format!("window.alert({literal});"))
                        .unwrap_or_else(|_| "console.error('Praxis 配置装载失败（文案序列化异常）');".into());
                    let _ = main_window.eval(&script);
                }
            }

            // 日志在 setup 钩子初始化：目录由 Tauri app_log_dir() 统一管理，
            // 与设置（app_data_dir）同属应用 identifier 目录；并经 PRAXIS_LOG_DIR
            // 注入 sidecar，Python 端写同一目录。级别取 config.json logging.level
            // （任务 28 弃用 PRAXIS_LOG_LEVEL；未装载/非法/空串均回退 info）。
            // guard 挂到应用状态，进程退出时随 state 释放并触发异步刷盘。
            let log_dir = app
                .path()
                .app_log_dir()
                .map_err(|e| format!("无法定位日志目录：{e}"))?;
            let level = app_config::global()
                .map(|(_, config)| config.logging.level.trim().to_string())
                .filter(|level| !level.is_empty())
                .unwrap_or_else(|| "info".to_string());
            let guard = logging::init(&log_dir, &level);
            app.manage(guard);
            tracing::info!("Praxis 桌面壳启动");

            // 启动清扫（任务 22）：sidecar 重启后后台生成任务已随 Python 进程
            // 消失，上一进程遗留的 status='generating' 行永远等不到终态事件，
            // 全部翻 failed（幂等；仅 DB 模式，库不可用则跳过不影响启动）。
            tauri::async_runtime::spawn(async move {
                let config = commands::load_db_config();
                if config.host.trim().is_empty() {
                    tracing::debug!("云端库未配置，跳过启动清扫");
                    return;
                }
                match db::pool(&config).await {
                    Ok(pool) => match plan_store::sweep_stale_generating(&pool).await {
                        Ok(count) if count > 0 => {
                            tracing::info!(count, "启动清扫完成：存量 generating 计划已置 failed")
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "启动清扫执行失败"),
                    },
                    Err(e) => tracing::warn!(error = %e, "云端库不可用，跳过启动清扫"),
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Praxis 桌面应用启动失败")
        .run(|_app, event| {
            // 应用退出时收割 sidecar 子进程，避免遗留孤儿进程占用资源
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                tracing::info!("应用退出，收割 sidecar 子进程");
                sidecar::shutdown_agent();
            }
        });
}
