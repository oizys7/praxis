// Praxis 桌面应用入口
//
// 在 release 模式下隐藏 Windows 控制台窗口（Tauri 官方模板标准写法，勿删）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        // 注册前端可 invoke 的命令：
        // - generate_plan：转发学习计划生成请求到 Python Agent 服务
        // - ping_agent：健康检查，用于前端展示服务在线状态
        .invoke_handler(tauri::generate_handler![
            commands::generate_plan,
            commands::ping_agent,
        ])
        .run(tauri::generate_context!())
        .expect("Praxis 桌面应用启动失败");
}
