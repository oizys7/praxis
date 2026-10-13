// Praxis 桌面应用入口
//
// 在 release 模式下隐藏 Windows 控制台窗口（Tauri 官方模板标准写法，勿删）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_client;
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
mod tests_plan_store;

fn main() {
    tauri::Builder::default()
        // 注册前端可 invoke 的命令：
        // - generate_plan：转发学习计划生成请求到 Python Agent 服务
        // - ping_agent：健康检查，用于前端展示服务在线状态
        .invoke_handler(tauri::generate_handler![
            commands::generate_plan,
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
