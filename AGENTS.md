# AGENTS.md — AI 编码代理指南

本文件为 AI 编码代理（以及人类协作者）提供在本仓库工作的关键约定。动手改代码前请先读完。

## 项目概述

Praxis 是一个学习路线规划 Agent 桌面应用：用户输入目标 / 水平 / 每周时长 / 周数 / 偏好方向，Agent 返回分阶段的学习计划（理论 / 实操 / 作业 / 项目）。

```text
┌──────────────────────────┐  Tauri IPC  ┌───────────────────┐   HTTP    ┌──────────────────────────┐
│  frontend/ (Vue 3 + TS)  │ ──────────> │  src-tauri/ (Rust)│ ────────> │ agent-service/ (Python)  │
│  表单 / 结果展示          │   invoke()  │  generate_plan    │  reqwest  │  FastAPI + LangGraph     │
└──────────────────────────┘             └───────────────────┘           └──────────────────────────┘
```

- 前端 → Rust：Tauri IPC（`invoke()`）
- Rust → Python：HTTP `POST http://127.0.0.1:8000/api/generate-plan`（`GET /health` 健康检查）
- Python 内部：LangGraph 工作流 `clarify → plan → review`（review 未通过时 plan 最多回炉一次）

## 目录结构

```text
frontend/              Vue 3 + TS + Tailwind（Vite，dev 端口 5173）
  src/types/agent.ts     三端协议的前端定义（权威来源之一）
  src/components/        业务组件（PlanForm/PlanResult/PhaseCard/ClarificationPanel）
  src/components/ui/     手写 shadcn 风格基础组件（不依赖 shadcn CLI）
  src/composables/       useAgent.ts（IPC 调用与错误归一封装）
src-tauri/             Tauri 2 Rust 壳
  src/commands.rs        IPC 命令 + 协议的 Rust 定义 + HTTP 转发与错误归一
  tauri.conf.json        注意：beforeDevCommand 路径相对【仓库根】，不是 src-tauri/
agent-service/         Python FastAPI + LangGraph
  app/schemas.py         Pydantic 模型（CamelModel 基类，camelCase 别名）
  app/nodes.py           clarify / plan / review 节点实现
  app/graph.py           StateGraph 装配（条件边 + 回炉上限）
  app/mock.py            离线 mock 计划（PRAXIS_ALLOW_MOCK_LLM=true）
  prompts/*.yaml         所有 LLM 提示词（改文案不需要动代码）
scripts/               仓库级脚本（dev / check / format / build / tauri 转发）
```

## 必须遵守的约定

### 1. 三端协议对齐（最重要）

任何涉及请求/响应字段的修改，必须同步以下三处，字段全部 **camelCase**：

- `frontend/src/types/agent.ts`
- `src-tauri/src/commands.rs`
- `agent-service/app/schemas.py`

统一响应包络：`{ status: "plan" | "clarification" | "error", plan?, clarifications?, error? }`。
业务错误走 HTTP 200 + `status=error`（不抛 4xx/5xx）；网络层错误才由 Rust 归一为 `{ code, message, hint }` 抛给前端。

### 2. 工具链

- **Python 一律用 uv，不要用 pip**：`cd agent-service && uv venv && uv pip install -r requirements.txt`
- `requirements.txt` 保持纯 ASCII（Windows pip 按 GBK 解码该文件，中文注释会直接 UnicodeDecodeError）
- tauri-cli 必须从仓库根运行：统一走 `npm run tauri -- <子命令>`（`scripts/tauri.mjs` 已处理 cwd 与 `.bin` 解析）
- Python 服务用 `.venv` 解释器启动；手动方式：`cd agent-service && uv run uvicorn app.main:app --reload`（`uv run` 自动发现 `.venv`，无需 activate）

### 3. 提示词与 LLM

- 提示词只写在 `agent-service/prompts/*.yaml`，模板变量用 `{{ var }}` 渲染；**禁止把提示词硬编码进 Python**
- LLM Key 一律走环境变量（`OPENAI_API_KEY` / `ANTHROPIC_API_KEY`），配置项见 `agent-service/.env.example`
- 无 Key 开发/联调：设 `PRAXIS_ALLOW_MOCK_LLM=true` 走离线 mock，全链路可跑
- `.env` / `.env.*` 不入库（.gitignore 已覆盖）；`.env.example` 是唯一可提交的模板

### 4. 代码风格

- 注释、文档、commit message 用中文；标识符/代码用英文
- 前端格式化用 `npm run format`（Prettier）；Python 兼容 ruff
- JSON 配置文件（`tauri.conf.json` / `capabilities/*.json`）**不能写注释**，说明写进 README

## 常用命令

| 命令 | 作用 |
| --- | --- |
| `npm run dev` | 一键启动开发环境（uvicorn + tauri dev，tauri 会自动带起 Vite） |
| `npm run check` | 自检：Python 语法编译 + 提示词 YAML 校验 + vue-tsc 类型检查 |
| `npm run format` | Prettier 格式化（前端 / 脚本 / Markdown） |
| `npm run build` | 生产构建（前端构建 + tauri build） |
| `npm run tauri -- <cmd>` | 任意 tauri 子命令（icon / dev / build），**必须走这个入口** |

## 提交前验证（缺一不可）

1. `npm run check` 全绿
2. 改了 Rust：`cd src-tauri && cargo check && cargo clippy`，零警告
3. 改了 Python：`cd agent-service && uv run python -c "import app.main"` 烟测导入
4. 警告也要清零：防御性保留的代码用 `#[allow(dead_code)]` + 注释说明意图，而不是放任警告

## 已知陷阱

- `.vue` 文件的 "Cannot find module" LSP 报错是误报，以 `npx vue-tsc --noEmit` 为权威判据
- `tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 路径相对**仓库根**（调用 tauri-cli 的 cwd），不是相对 `src-tauri/`
- tauri-cli 只在 cwd 子树内查找 `tauri.conf.json`——在 `frontend/` 里直接跑 `tauri dev` 会 panic（找不到兄弟目录 `src-tauri/`）
- `src-tauri/icons/` 被误删后：`npm run tauri -- icon app-icon.png` 从根目录源图重新生成全套
- reqwest 默认无全局超时，本项目按请求单独设置（ping 3s / plan 180s），新增 HTTP 调用记得带超时

## Git 约定

- 分支：`main`；远程：`github.com/oizys7/praxis`
- commit message 用中文，首行 `类型: 摘要`（如 `init:` / `fix:` / `feat:` / `docs:`）
- 协议：MIT（见 LICENSE）
