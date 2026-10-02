# scripts/ 工具脚本

仓库级的自动化脚本（Node 编写，Windows / macOS / Linux 通用）。统一从仓库根目录通过 npm scripts 调用：

| 命令 | 脚本 | 作用 |
| --- | --- | --- |
| `npm run dev` | `dev.mjs` | 一键启动开发环境：同时拉起 Python Agent 服务（uvicorn --reload）与 Tauri 桌面端（自动带起 Vite），Ctrl+C 一次性停止全部子进程 |
| `npm run check` | `check.mjs` | 提交前自检：Python 语法检查 → 提示词 YAML 校验 → 前端类型检查（vue-tsc） |
| `npm run format` | `format.mjs` | 用 Prettier 格式化前端源码、脚本、根目录 Markdown（Python 侧建议用 `ruff format agent-service`） |
| `npm run build` | `build.mjs` | 生产构建：前端构建 + Tauri 桌面打包 |

## 注意事项

- 首次使用前需安装依赖：根目录 `npm install`（提供 Prettier）、`frontend/` 下 `npm install`。
- `dev.mjs` 启动前会检查 `agent-service/.env` 是否存在，缺失时打印配置提示（不会阻塞启动）。
- Windows 下子进程通过 `taskkill /T /F` 清理进程树；Unix 下使用进程组信号。
