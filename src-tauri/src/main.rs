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
mod plan_state;
mod plan_store;
mod protocol;
mod schedule;
mod secret;
mod settings;
mod sidecar;
mod startup;

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
        // - start_learning / update_learning_days / toggle_schedule_item：
        //   开始学习 / 修改学习日 / 行级打卡（打卡日历真数据）
        // - query_month_calendar / query_year_heatmap：月历按天组织 / 年热力图
        .invoke_handler(tauri::generate_handler![
            commands::generate_plan,
            commands::regenerate_phase,
            commands::regenerate_chapter,
            commands::ping_agent,
            commands::get_settings,
            commands::save_settings,
            commands::list_plans,
            commands::get_plan,
            commands::delete_plan,
            commands::toggle_favorite,
            commands::start_learning,
            commands::update_learning_days,
            commands::toggle_schedule_item,
            commands::query_month_calendar,
            commands::query_year_heatmap,
            commands::startup_status,
        ])
        .setup(|app| {
            use tauri::Manager;
            // 日志在 setup 钩子初始化（审查 F2：先建 logger，后报配置装载失败，
            // 使错误日志可被接收）：目录由 Tauri app_log_dir() 统一管理，与设置
            // （app_data_dir）同属应用 identifier 目录；Python 端日志目录由 config.json
            // logging.dir/默认推导对齐（审查 F1：PRAXIS_LOG_DIR 注入已退役）。
            // 级别取 config.json logging.level（未装载/非法/空串均回退 info）。
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

            // 配置装载失败的延迟提示（main() 已 eprintln；setup 阶段日志+弹窗链补可见性，
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
            tracing::info!("Praxis 桌面壳启动");

            // 启动序列（2026-10-22）：主动预热云端库并起 Agent 服务，阶段与
            // 耗时经 startup_status 暴露给前端启动页。此前连接池是惰性的，
            // 首个 DB 命令才触发 12.8s 建连，期间所有页面都只是转圈，观感像
            // 应用坏了。清扫（幂等）并入序列：① generating 滑口行置 failed；
            // ② failed 自愈（2026-10-16 裁定）：章节已全部 complete 的 failed
            // 行翻回 complete，真失败行（尚有非 complete 章）保持 failed。
            let startup_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                startup::run(startup_handle).await;
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
