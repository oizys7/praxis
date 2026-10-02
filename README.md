# Praxis · 学习路线规划 Agent

一个 **AI Agent 桌面应用**：输入学习目标、当前水平、可用时间等信息，Agent 生成一份「理论 + 实操 + 作业 + 项目」四位一体、可执行可验收的分阶段学习计划。

- **桌面框架**：Tauri 2（Rust）
- **前端**：Vue 3 + TypeScript + Vite + shadcn-vue 风格组件 + Tailwind CSS
- **Agent 核心**：Python + LangGraph（clarify → plan → review 状态图）+ FastAPI
- **通信**：Tauri IPC（前端 → Rust） + HTTP（Rust → Python）

---

## 架构

```
┌─────────────────────────── Tauri 桌面应用 ───────────────────────────┐
│                                                                      │
│  ┌──────────────┐  Tauri IPC invoke   ┌───────────────────────────┐  │
│  │  Vue 3 前端   │ ─────────────────→ │  Rust 命令层 (commands.rs) │  │
│  │  (WebView)   │ ←───────────────── │  generate_plan / ping_agent│  │
│  └──────────────┘   PlanResponse JSON └──────────┬────────────────┘  │
│                                                   │ HTTP POST         │
└───────────────────────────────────────────────────┼──────────────────┘
                                                    ▼
                                     http://127.0.0.1:8000
                              ┌─────────────────────────────┐
                              │   Python FastAPI 服务        │
                              │   POST /api/generate-plan   │
                              │         │                   │
                              │   ┌─────▼──────┐            │
                              │   │  LangGraph │            │
                              │   │  clarify   │ 不完整→澄清问题
                              │   │     ↓      │            │
                              │   │   plan     │ LLM 结构化输出
                              │   │     ↓      │            │
                              │   │  review    │ 不通过→回炉一次
                              │   └────────────┘            │
                              └─────────────────────────────┘
```

## 目录结构

```
Praxis/
├── package.json            # 仓库级脚本入口（npm run dev / check / format / build）
├── scripts/                # 构建、格式化、自检等自动化脚本（见 scripts/README.md）
├── src-tauri/              # Tauri 2 Rust 后端
│   ├── src/
│   │   ├── main.rs         # 入口：注册 Tauri commands
│   │   └── commands.rs     # generate_plan / ping_agent，reqwest 转发 + 错误归一化
│   ├── capabilities/       # Tauri 2 权限配置
│   ├── Cargo.toml
│   ├── build.rs
│   └── tauri.conf.json
├── frontend/               # Vue 3 前端
│   └── src/
│       ├── App.vue         # 页面骨架：左表单 / 右结果
│       ├── components/     # PlanForm、PhaseCard、ClarificationPanel、PlanResult
│       │   └── ui/         # shadcn-vue 风格基础组件（button/card/input/…）
│       ├── composables/useAgent.ts   # invoke 封装：loading/error/result
│       ├── types/agent.ts  # 三端统一类型（camelCase）
│       └── lib/utils.ts    # cn() 类名合并
└── agent-service/          # Python Agent 服务
    ├── app/
    │   ├── main.py         # FastAPI 入口 + 统一异常处理
    │   ├── graph.py        # LangGraph 状态图定义
    │   ├── nodes.py        # clarify / plan / review 节点逻辑
    │   ├── schemas.py      # Pydantic 请求/响应模型（camelCase alias）
    │   ├── prompts.py      # YAML 提示词加载与渲染
    │   ├── llm.py          # LLM 工厂（OpenAI / Anthropic 可切换）
    │   ├── mock.py         # 离线演示模式的确定性计划
    │   └── config.py       # 环境变量配置
    ├── prompts/            # 提示词 YAML（clarify / plan / review）
    ├── requirements.txt
    └── .env.example
```

## 通信协议（三端统一）

所有 JSON 字段均为 **camelCase**，三处定义严格对齐：
`frontend/src/types/agent.ts` ⇔ `src-tauri/src/commands.rs` ⇔ `agent-service/app/schemas.py`。

### 请求 `POST /api/generate-plan`

```json
{
  "goal": "三个月内成为能独立开发 Web 应用的后端工程师",
  "level": "beginner",
  "weeklyHours": 10,
  "weeks": 12,
  "interests": ["后端 / 系统", "Web 全栈"],
  "background": "会一点 Python，没做过完整项目"
}
```

### 响应统一包络 `PlanResponse`

响应永远是同一种包络结构，靠 `status` 区分三种业务结果：

```jsonc
// ① 生成成功
{
  "status": "plan",
  "plan": {
    "title": "…",
    "overview": "…",
    "estimatedTotalHours": 120,
    "phases": [
      {
        "name": "第一阶段 · 打基础",
        "weekRange": "第 1-4 周",
        "objectives": ["…"],
        "theory": ["…"],        // 理论
        "practice": ["…"],      // 实操
        "assignments": ["…"],   // 作业
        "project": {
          "name": "…",
          "description": "…",
          "acceptanceCriteria": ["可验证的验收标准 1", "…"]
        }
      }
    ],
    "resources": [{ "title": "…", "type": "book", "note": "…" }],
    "reviewNotes": "评审节点备注"
  },
  "clarifications": null,
  "error": null
}

// ② 输入不完整，需要澄清
{
  "status": "clarification",
  "plan": null,
  "clarifications": ["你的学习目标可以再具体一些吗？…", "…"],
  "error": null
}

// ③ 出错（HTTP 仍为 200，便于前端统一解析；仅未捕获异常返回 500 + 同样包络）
{
  "status": "error",
  "plan": null,
  "clarifications": null,
  "error": { "code": "service", "message": "…" }
}
```

### 前端 → Rust（Tauri IPC）

```ts
import { invoke } from "@tauri-apps/api/core";

const response = await invoke<PlanResponse>("generate_plan", { request });
const online = await invoke<boolean>("ping_agent");
```

command 抛错时前端捕获到 `{ code, message, hint }`：`code ∈ connection | timeout | http | service | internal`，`hint` 是面向用户的可操作提示。

### 健康检查 `GET /health`

```json
{ "status": "ok", "service": "praxis-agent", "version": "0.1.0" }
```

## 环境变量

复制 `agent-service/.env.example` 为 `agent-service/.env` 后按需填写：

| 变量                    | 默认值                     | 说明                                               |
| ----------------------- | -------------------------- | -------------------------------------------------- |
| `LLM_PROVIDER`          | `openai`                   | `openai` 或 `anthropic`                            |
| `OPENAI_API_KEY`        | 空                         | OpenAI API Key（provider=openai 时必填）           |
| `OPENAI_MODEL`          | `gpt-4o-mini`              | 模型名                                             |
| `ANTHROPIC_API_KEY`     | 空                         | Anthropic API Key（provider=anthropic 时必填）     |
| `ANTHROPIC_MODEL`       | `claude-3-5-sonnet-latest` | 模型名                                             |
| `AGENT_SERVICE_HOST`    | `127.0.0.1`                | FastAPI 监听地址                                   |
| `AGENT_SERVICE_PORT`    | `8000`                     | FastAPI 监听端口                                   |
| `AGENT_SERVICE_URL`     | `http://127.0.0.1:8000`    | Rust 端请求的 Agent 服务地址（进程环境变量）       |
| `PRAXIS_ALLOW_MOCK_LLM` | `false`                    | **离线演示模式**：不调用 LLM，返回确定性 Mock 计划 |

## 安装步骤

前置依赖：[Rust](https://rustup.rs/)（含 `cargo`）、Node.js ≥ 18、Python ≥ 3.11、[uv](https://docs.astral.sh/uv/)（推荐，用于 Python 虚拟环境）。

```bash
# 1) 前端依赖
cd frontend
npm install

# 2) 根目录脚本依赖（Prettier）
cd ..
npm install

# 3) Python Agent 服务（uv 创建虚拟环境 + 安装依赖）
cd agent-service
uv venv
uv pip install -r requirements.txt

# 4) 配置环境变量
cp .env.example .env      # 填入 OPENAI_API_KEY；没有 Key 可先设 PRAXIS_ALLOW_MOCK_LLM=true
```

> Tauri 在 Windows 上还需要 WebView2（Win10/11 一般自带）与 Visual Studio C++ 构建工具。

## 开发启动

### 方式一：一键启动（推荐）

```bash
# 仓库根目录：同时拉起 Python 服务（uvicorn --reload）和 Tauri 桌面端（自动带起 Vite）
npm run dev
```

### 方式二：分终端启动

```bash
# 终端 1 —— Python Agent 服务（uv run 自动发现并使用 .venv，无需手动激活）
cd agent-service
uv run uvicorn app.main:app --reload --port 8000

# 终端 2 —— Tauri 桌面端（beforeDevCommand 会自动启动 Vite）
# 注意：tauri-cli 只在 cwd 子树内查找 tauri.conf.json，必须经转发脚本从仓库根运行
npm run tauri -- dev
```

### 纯前端样式调试

```bash
cd frontend && npm run dev      # 浏览器打开 http://localhost:5173
```

注意：浏览器里没有 Tauri IPC，`invoke()` 调用会失败（属预期）；调 UI 布局用，完整流程请在桌面窗口内验证。

## 常用脚本（仓库根目录）

| 命令             | 作用                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------ |
| `npm run dev`    | 一键启动开发环境（Python 服务 + Tauri）                                              |
| `npm run check`  | 三端自检：Python（语法 + ruff lint）→ YAML → Rust（check + clippy）→ 前端（vue-tsc） |
| `npm run format` | 三端格式化：ruff（Python）+ cargo fmt（Rust）+ Prettier（前端 / 脚本 / Markdown）    |
| `npm run build`  | 三端构建：Python 健全性 → 前端构建 → Tauri 打包（含 Rust release 编译）              |

## 打包发布

```bash
# 打包（产物在 src-tauri/target/release/bundle/）
npm run build
```

默认应用图标已内置（源图：根目录 `app-icon.png`，1024×1024 紫底白 P）。替换品牌图标时，
用新图覆盖 `app-icon.png` 后重新生成全套：`npm run tauri -- icon app-icon.png`。

## Agent 工作流（LangGraph）

```
clarify ──信息不完整──→ END（返回澄清问题，前端高亮展示）
   │ 规则校验通过（goal 长度 / 小时数 / 周数 / 已有基础）
   ▼
  plan ──→ review ──未通过且未修订──→ plan（注入评审意见，最多回炉一次）
（LLM 结构化输出）  │ 通过或已修订
                   ▼
                  END（计划 + 评审备注一并返回）
```

- 所有提示词集中在 `agent-service/prompts/*.yaml`，改文案不需要动代码；
- LLM 输出通过 `with_structured_output(PydanticModel)` 强制 JSON，解析失败会自动重试；
- 无 API Key 时设 `PRAXIS_ALLOW_MOCK_LLM=true`，全链路照常可跑（用于 UI / 联调）。

## 常见问题

- **提示"无法连接 Python Agent 服务"**：先启动 agent-service（见上），并确认端口与 `AGENT_SERVICE_URL` 一致；前端顶栏徽标可实时查看在线状态。
- **编译报 `icons/icon.ico not found`**：项目已内置默认图标；若误删 `src-tauri/icons/`，执行 `npm run tauri -- icon app-icon.png` 重新生成。
- **`npm run dev` 报 No module named uvicorn 或 tauri 找不到配置**：前者先执行 `cd agent-service && uv venv && uv pip install -r requirements.txt`；后者是因为 tauri-cli 必须从仓库根运行，请统一使用 `npm run tauri -- <命令>`（已内置处理）。
- **Windows pip 报 GBK 编码错误**：本项目推荐使用 uv（`uv pip install`）；`requirements.txt` 保持纯 ASCII。
- **更换 LLM 提供商**：`.env` 中设 `LLM_PROVIDER=anthropic` 并填对应 Key，无需改代码。

## 开源协议

[MIT](./LICENSE)
