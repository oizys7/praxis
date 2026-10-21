# Praxis · 学习路线规划 Agent

一个 **AI Agent 桌面应用**：输入学习目标、当前水平、可用时间等信息，Agent 生成一份「理论 + 实操 + 作业 + 项目」四位一体、可执行可验收的分阶段学习计划。

- **桌面框架**：Tauri 2（Rust）
- **前端**：Vue 3 + TypeScript + Vite + shadcn-vue 风格组件 + Tailwind CSS
- **Agent 核心**：Python + LangGraph（clarify → plan → review 状态图）+ FastAPI
- **通信**：Tauri IPC（前端 → Rust） + HTTP（Rust → Python）

---

## 架构

```text
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

```text
Praxis/
├── package.json            # 仓库级脚本入口（npm run dev / check / format / build）
├── scripts/                # 构建、格式化、自检等自动化脚本（见 scripts/README.md）
├── src-tauri/              # Tauri 2 Rust 后端
│   ├── src/
│   │   ├── main.rs         # 入口：注册 Tauri commands + 日志初始化 + 退出收割 sidecar
│   │   ├── commands.rs     # Tauri commands 薄壳（计划生成/健康检查/设置/云端库）
│   │   ├── protocol.rs     # 三端协议数据模型（camelCase）
│   │   ├── error.rs        # AgentError：统一序列化为 { code, message, hint }
│   │   ├── agent_client.rs # Python 服务的 HTTP 出口（req_id / 超时 / 日志）
│   │   ├── sidecar.rs      # 内置 Agent 子进程生命周期（release 动态端口）
│   │   ├── db.rs           # 云端 PostgreSQL 持久化（学习计划历史，首连自动建表）
│   │   ├── settings.rs     # settings.json（LLM 配置 + 云端库连接配置）
│   │   └── logging.rs      # 统一日志（格式契约 + 10MB×4 大小轮转）
│   ├── capabilities/       # Tauri 2 权限配置
│   ├── Cargo.toml
│   ├── build.rs
│   └── tauri.conf.json
├── docs/                   # 部署与运维文档（如 docs/deploy-ecs.md 云端数据库部署）
├── frontend/               # Vue 3 前端
│   └── src/
│       ├── App.vue         # 页面骨架：学习计划（列表 ⇄ 生成/查看两态）/ 设置
│       ├── components/     # PlanListView、PlanForm、PlanResult、PhaseCard、ClarificationPanel、Settings
│       │   └── ui/         # shadcn-vue 风格基础组件（button/card/input/…）
│       ├── composables/    # useAgent / useSettings / usePlanHistory
│       ├── types/agent.ts  # 三端统一类型（camelCase）
│       └── lib/utils.ts    # cn() 类名合并
└── agent-service/          # Python Agent 服务（仅任务编排与 LLM 调用）
    ├── app/
    │   ├── main.py         # FastAPI 入口 + 路由（统一异常包络见 errors.py）
    │   ├── graph.py        # LangGraph 状态图定义
    │   ├── nodes.py        # clarify / plan / review 薄节点
    │   ├── errors.py       # AgentServiceError + register_error_handlers
    │   ├── schemas.py      # Pydantic 请求/响应模型（camelCase alias）
    │   ├── prompts.py      # YAML 提示词加载与渲染
    │   ├── llm.py          # LLM 工厂 + invoke_structured（五提供商）
    │   ├── mock.py         # 离线演示模式的确定性计划
    │   ├── logging_config.py # 统一日志（与 Rust 格式契约对齐）
    │   └── config.py       # config.json 冷启动装载 + /api/config 热更新
    ├── prompts/            # 提示词 YAML（clarify / plan / review）
    ├── agent_entry.py      # 服务启动引导（sidecar 生产入口 + dev 热重载入口）
    └── pyproject.toml
```

仓库根另有共享配置：`config.example.json`（入库模板）/ `config.json`
（本机文件，gitignore 覆盖，Python/Rust 两端共用，见下文「配置」）。

## 通信协议（三端统一）

所有 JSON 字段均为 **camelCase**，三处定义严格对齐：
`frontend/src/types/agent.ts` ⇔ `src-tauri/src/protocol.rs` ⇔ `agent-service/app/schemas.py`。

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
{ "status": "ok", "service": "praxis-agent", "version": "0.0.1" }
```

## 配置（设置页优先）

**推荐方式**：应用内置「设置」页（⚙️ 菜单）随时配置五家 LLM 提供商，保存即写盘（Tauri 应用数据目录 `%APPDATA%/com.praxis.desktop/settings.json`，与日志 `%LOCALAPPDATA%/com.praxis.desktop/logs/` 同属 identifier 目录）并**热加载**到 Agent 服务，无需重启应用。五家提供商的 API Key 与模型名各自独立，切换互不覆盖：

| 提供商      | 接入方式                          | 默认模型                   |
| ----------- | --------------------------------- | -------------------------- |
| OpenAI      | 官方 API                          | `gpt-4o-mini`              |
| Anthropic   | 官方 SDK                          | `claude-3-5-sonnet-latest` |
| DeepSeek    | 官方 SDK（api.deepseek.com）      | `deepseek-chat`            |
| 智谱 GLM    | OpenAI 兼容（open.bigmodel.cn）   | `glm-5.3-flash`            |
| OpenCode Go | OpenAI 兼容（opencode.ai/zen/go） | `grok-4.7`                 |

**冷启动分两层**：LLM 十二字段不存在环境变量冷启动——只有 settings.json → `/api/config` 一条链（设置页保存即热加载）；服务监听 / 日志 / 离线演示开关等运维项则收敛在仓库根共享的 `config.json`（Python 与 Rust 两端各取所需，字段表与查找顺序见下文「config.json：应用级配置文件」）。Python 端启动未配置任何 Key 且 `agent.mockMode` 未开时：服务仍正常启动（健康检查可用），生成接口返回明确错误提示到应用「设置」页配置。

### config.json：应用级配置文件

Rust 端与 Python 端共享的启动配置文件（模板见仓库根 `config.example.json`，`config.json` 本机维护不入库）。**查找顺序**（首个命中者生效，找不到则启动报错并列出全部候选路径）：

1. `--config <路径>` 命令行参数（sidecar 模式显式指定）
2. 开发模式：仓库根 `config.json`（可执行文件路径检测到工作区标记时；标记要求 `src-tauri/tauri.conf.json` **文件**存在——含 `tauri.conf.json` 文件的目录或含 `src-tauri/` 子目录的根，非仅存在 `src-tauri` 目录）
3. 应用数据目录 `config.json`（与 settings.json 同目录，如 `%APPDATA%/com.praxis.desktop/config.json`）

| 字段                   | 默认值      | 说明                                                                    |
| ---------------------- | ----------- | ----------------------------------------------------------------------- |
| `database.url`         | 空          | **生产库**连接串（`postgres://user:password@host:5432/db`）；运行时读写 |
| `database.testUrl`     | 空          | **测试库**连接串，`cargo test` 专用（与生产库分离，双字段落位）         |
| `logging.level`        | `info`      | 日志级别（Rust / Python 全端生效）                                      |
| `logging.dir`          | 空          | 日志目录；空 = 各端默认路径（Rust 走 app_log_dir，Python 走推导目录）   |
| `service.host`         | `127.0.0.1` | Agent 服务监听地址                                                      |
| `service.port`         | `8000`      | Agent 服务监听端口                                                      |
| `agent.mockMode`       | `false`     | 离线演示模式**冷启动默认**（settings.json 的 mock 开关仍是运行时权威）  |
| `secrets.keyDir`       | 空          | 主密钥目录；空 = `secret.rs` 现有默认（应用数据目录）                   |
| `secrets.sealPassword` | 空          | 手动封存密码；空 = `secret.rs` 现有默认（交互生成）                     |

> 模板另含 `_comment` 说明字段（JSON 不支持注释，解析时会被忽略）；未知字段一律忽略，缺省段落/字段/显式 `null` 回退内置默认值。各环境变量向 config.json 的迁移清单见计划文档任务 28–29（当前双链过渡期）。

## 安装步骤

前置依赖：[Rust](https://rustup.rs/)（含 `cargo`）、Node.js ≥ 18、Python ≥ 3.11、[uv](https://docs.astral.sh/uv/)（推荐，用于 Python 虚拟环境）。

```bash
# 1) 前端依赖
cd frontend
npm install

# 2) 根目录脚本依赖（Prettier）
cd ..
npm install

# 3) Python Agent 服务（uv sync：自动创建 .venv 并按 pyproject.toml 安装依赖）
cd agent-service
uv sync

# 4) 配置：LLM 五家提供商在应用内置「设置」页配置（保存即热加载，无需改文件）；
#    服务监听 / 日志 / 离线演示开关等运维项在仓库根 config.json
#    （首次 npm run dev 时自动从 config.example.json 复制模板，可直接用默认值）
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
# 前提：仓库根已有 config.json（config.py 沿 cwd 向上按 src-tauri 标记自动找到它）
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

## 分发与安装包

安装包内置 PyInstaller 打包的 Agent 服务（sidecar 模式），**其他设备无需安装
Python / uv / Node**，双击安装即可使用（唯一系统依赖 WebView2，Win10/11 自带）：

| 模式           | Agent 服务来源                                            | 端口         | 适用场景   |
| -------------- | --------------------------------------------------------- | ------------ | ---------- |
| 开发（debug）  | `scripts/dev.mjs` 启动的 uvicorn，或手动 `uv run uvicorn` | 固定 8000    | 本仓库开发 |
| 生产（安装包） | 应用自动拉起内置 `agent/praxis-agent.exe`                 | 动态随机分配 | 终端用户   |

- 生产模式下 `praxis.exe` 启动时自动拉起 sidecar 子进程（随机空闲端口经 CLI
  参数 `--port N --config <路径>` 传给子进程），轮询 `/health` 就绪后才开始服务；
  应用退出时自动收割子进程，无孤儿进程。
- `AGENT_SERVICE_URL` 环境变量在开发模式仍可覆盖服务地址；生产模式不适用。
- 安装包体积约 35-50 MB（内置 CPython 运行时与依赖，压缩后）。
- PyInstaller 不支持交叉编译：Windows 包在 Windows 上构建，macOS/Linux
  需在对应系统执行 `npm run build`。
- 构建时可用 `PRAXIS_SKIP_AGENT=1 npm run build` 跳过 sidecar 打包
  （仅调试桌面壳时使用）。

## Agent 工作流（LangGraph）

```text
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
- 无 API Key 时在 `config.json` 设 `agent.mockMode=true`（或在应用设置页开启
  离线演示开关），全链路照常可跑（用于 UI / 联调）。

## 常见问题

- **提示"无法连接 Python Agent 服务"**：先启动 agent-service（见上），并确认端口与 `AGENT_SERVICE_URL` 一致；前端顶栏徽标可实时查看在线状态。
- **编译报 `icons/icon.ico not found`**：项目已内置默认图标；若误删 `src-tauri/icons/`，执行 `npm run tauri -- icon app-icon.png` 重新生成。
- **`npm run dev` 报 No module named uvicorn 或 tauri 找不到配置**：前者先执行 `cd agent-service && uv sync`；后者是因为 tauri-cli 必须从仓库根运行，请统一使用 `npm run tauri -- <命令>`（已内置处理）。
- **Windows pip 报 GBK 编码错误**：本项目已全面改用 uv（`uv sync`），不再经过 pip；依赖统一声明在 `agent-service/pyproject.toml`。
- **更换 LLM 提供商**：应用「设置」页直接切换提供商并填对应 Key，保存即生效，无需改代码。

## 开源协议

[MIT](./LICENSE)
