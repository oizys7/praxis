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

// plan_store 集成测试（需 DATABASE_URL 指向本机可建的 PostgreSQL，见模块文档）
#[cfg(test)]
mod tests_app_config;
#[cfg(test)]
mod tests_plan_store;

fn main() {
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
            // 日志在 setup 钩子初始化：目录由 Tauri app_log_dir() 统一管理，
            // 与设置（app_data_dir）同属应用 identifier 目录；并经 PRAXIS_LOG_DIR
            // 注入 sidecar，Python 端写同一目录。guard 挂到应用状态，
            // 进程退出时随 state 释放并触发异步刷盘。
            let log_dir = app
                .path()
                .app_log_dir()
                .map_err(|e| format!("无法定位日志目录：{e}"))?;
            let guard = logging::init(&log_dir);
            app.manage(guard);
            tracing::info!("Praxis 桌面壳启动");

            // 启动清扫（任务 22）：sidecar 重启后后台生成任务已随 Python 进程
            // 消失，上一进程遗留的 status='generating' 行永远等不到终态事件，
            // 全部翻 failed（幂等；仅 DB 模式，库不可用则跳过不影响启动）。
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let config = commands::load_db_config(&app_handle);
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
