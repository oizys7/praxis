# scripts/ 工具脚本

仓库级自动化脚本，统一从**仓库根目录**通过 npm scripts 调用（Windows / macOS / Linux 通用）。

首次使用前：根目录与 `frontend/` 各执行一次 `npm install`。

## 总览

| 文件 | 调用方式 | 职责 |
| --- | --- | --- |
| `dev.mjs` | `npm run dev` | 一键启动开发环境：uvicorn + tauri dev（自动带起 Vite） |
| `check.mjs` | `npm run check` | 三端自检：Python（语法 + ruff lint）→ YAML → Rust（check + clippy）→ 前端（vue-tsc） |
| `format.mjs` | `npm run format` | 三端格式化：ruff（Python）+ cargo fmt（Rust）+ Prettier（前端 / 脚本 / Markdown） |
| `build.mjs` | `npm run build` | 四步构建：Python 健全性 → 前端构建 → Agent sidecar 打包（PyInstaller）→ Tauri 打包（含 Rust release 编译） |
| `tauri.mjs` | `npm run tauri -- <子命令>` | tauri-cli 转发器，固定以仓库根为 cwd |
| `check_prompts.py` | 被 `check.mjs` 内部调用 | 校验提示词 YAML 可解析；不单独运行 |

## 用法说明

### `npm run dev` — dev.mjs

1. 配置预检：仓库根无 `config.json` 时从 `config.example.json` 复制模板，
   并提示按需编辑（LLM API Key 在应用「设置」页配置，不入 config.json）；
2. 优先用 `agent-service/.venv` 的解释器启动 uvicorn（`.venv` 缺失时报错退出，并给出
   `uv sync` 指引）；
3. 从仓库根启动 tauri dev，其 `beforeDevCommand` 自动带起 Vite（5173）；
4. Ctrl+C 一次性清理全部子进程（Windows 用 `taskkill /T /F`，Unix 用进程组信号）。

### `npm run check` — check.mjs

三端依次执行六步，任一步失败即以非零码退出：

1. `python -m compileall -q agent-service/app`（Python 语法）
2. `uvx ruff@0.16.9 check agent-service/app scripts/check_prompts.py`（Python lint，uvx 临时运行无需预装，版本锁定保证结果可复现）
3. `python scripts/check_prompts.py`（YAML 可解析，强制 UTF-8）
4. `cargo check`（Rust 编译检查，src-tauri/）
5. `cargo clippy`（Rust lint）
6. `npx vue-tsc --noEmit`（前端类型，权威判据——`.vue` 的 "Cannot find module" LSP 报告是误报）

### `npm run format` — format.mjs

三端依次格式化：

1. `uvx ruff@0.16.9 format agent-service scripts/check_prompts.py`（Python，版本锁定保证格式化可复现）
2. `cargo fmt`（Rust，rustfmt 默认风格）
3. Prettier：`frontend/src`、前端配置、`scripts/*.mjs`、根目录 `*.md`

### `npm run build` — build.mjs

三端依次执行：

1. `python -m compileall -q agent-service/app`（Python 语法健全性，解释型无构建产物）
2. 前端构建（vue-tsc + vite，产物 `frontend/dist`）
3. Agent sidecar 打包：`uv run pyinstaller --name praxis-agent --onedir agent_entry.py`
   （cwd `agent-service/`，产物 `dist/praxis-agent/`，经 tauri resources 进安装包；
   `PRAXIS_SKIP_AGENT=1` 可跳过）
4. `tauri build`（内含 Rust release 编译，安装包在 `src-tauri/target/release/bundle/`）

### `npm run tauri -- <子命令>` — tauri.mjs

tauri-cli 只在当前目录子树内查找 `tauri.conf.json`，因此必须固定以仓库根为 cwd、
调用 `frontend/node_modules/.bin` 下的 cli。本脚本封装了这两点。示例：

```bash
npm run tauri -- dev                      # 开发调试（一般用 npm run dev 即可）
npm run tauri -- icon app-icon.png        # 从根目录源图重新生成全套图标
npm run tauri -- build                    # 仅打包桌面应用（不含前端构建）
```
