# 计划：配置文件化改造 + 弃用标记（任务 27–30）

> 基线：`d652302`（v7 背景生成与进度已收口）。日期：2026-10-14。
> 用户裁定（四问）：① 单一 `config.json`；② 敏感项明文入 config、目录不入库；③ 子进程引导改命令行参数；④ 豁免清单「全部清零」（sqlx 框架例外见 §3）。

## 1. 目标

1. **配置文件化**：放弃所有运行时环境变量读取，收拢到应用数据目录 `config.json`（仓库只放 `config.example.json` 模板）。
2. **弃用标记**：已弃用代码用各语言原生机制打标（Rust `#[deprecated]` / Python `typing.deprecated` / TS `@deprecated`），第一版完成后统一清理；本计划不动行为、不删除。

## 2. 配置文件设计

### 2.1 文件与 schema

```jsonc
// config.example.json（入库模板，全占位）
// 同一字段区分生产/测试：url=生产，testUrl=测试（用户裁定）
{
  "database": {
    "url": "postgres://user:password@host:5432/db",        // 生产：运行时读写
    "testUrl": "postgres://user:password@host:5432/praxis_test" // 测试：cargo test 专用
  },
  "logging":  { "level": "info", "dir": "" },          // dir 空 = 各端默认路径
  "service":  { "host": "127.0.0.1", "port": 8000 },
  "agent":    { "mockMode": false },                      // 冷启动默认（settings.json 运行时权威）
  "secrets":  { "keyDir": "", "sealPassword": "" }     // 空 = secret.rs 现有默认
}
```

- **位置（release）**：应用数据目录 `config.json`（与 settings.json 同目录， Rosa `config.rs` 现有 app_data_dir 逻辑复用）。
- **位置（dev）**：仓库根 `config.json`（`.gitignore` 覆盖；`dev.mjs` 启动时若缺失则从模板复制并提示填写——替代现在零散的 export 环节）。
- **查找顺序**：`--config <路径>`（sidecar 模式）→ 仓库根（dev，仅 Rust 检测 workspace marker）→ 应用数据目录。找不到 = 致命错：日志明确打印三个查找路径后退出（Rust 弹窗、Python sys.exit(2) 信息）。
- **共享文件两端各取所需**：Rust 读 database/logging/secrets/service；Python 读 logging/service（Python 不用数据库；LLM 设置仍走 settings.json→/api/config 运行时热链，不变）。

### 2.2 现有环境变量 → 去向表（唯一权威清单）

| 变量 | 现读点 | 去向 |
| --- | --- | --- |
| Rust `PRAXIS_LOG_LEVEL` | logging.rs:84 | `logging.level` |
| Rust `PRAXIS_KEY_DIR` / `PRAXIS_SEAL_PASSWORD` | secret.rs:161 | `secrets.keyDir` / `secrets.sealPassword` |
| Rust `DATABASE_URL`（运行时） | db.rs load_db_config 链 | `database.url` |
| Rust `AGENT_SERVICE_URL` | sidecar.rs:31 | 删除（dev 用 service.port 推导 URL；release 用侧车动态端口，本就优先 env，现直接优先动态端口） |
| Rust→Python `AGENT_SERVICE_PORT` 写入 | sidecar.rs:88 | 命令行参数 `--port N` + `--config <路径>` |
| Python `LLM_PROVIDER`/5×KEY/5×MODEL | config.py:80-100 | **删除冷启动**——LLM 设置只有 settings.json 一条链（设置页保存 → /api/config）。无 Key 且 mock 未开 = 报错提示进设置页 |
| Python `AGENT_SERVICE_HOST`/`AGENT_SERVICE_PORT` | config.py:103 + uvicorn 调用 | `service.host`/`service.port`（config 文件）+ `--port` 参数（子进程模式） |
| Python `PRAXIS_ALLOW_MOCK_LLM` | config.py:107 | `agent.mockMode` 字段入 config（settings.json 的 mock 开关仍是运行时权威，config 值仅冷启动默认） |
| Python `PRAXIS_LOG_DIR` | logging_config.py:41 | `logging.dir` |
| Python `LOCALAPPDATA`/`XDG_DATA_HOME` | logging_config.py:45/47 | **删除 fallback**——`logging.dir` 有默认推导（config 加载方统一提供），不再读 OS 约定变量 |
| Python `load_dotenv()` / `.env*` | config.py:28 | 全部移除；`.env.example` 删除，README/AGENTS.md 改述 config |
| 前端 | （无使用，`vite.config.ts` 仅 envPrefix 声明） | 不动 |

### 2.3 测试/生产双字段落位（用户裁定取代 hydration 例外）

`#[sqlx::test]` 宏自身从环境变量读 `DATABASE_URL`（框架机制不可改）。处置：配置文件直接开辟 **双字段落位** ——`database.url`（生产）与 `database.testUrl`（测试，字段名自带“测试”语义）；`tests_support` 模块里的 **test-only 适配器** 启动时读 config 的 `testUrl` 灌入 env（`std::env::set_var` unsafe + 注释「仅测试进程内为 sqlx 宏桥接 config.json」）。效果：连接串从 env 手工粘贴永久退役——测试库与生产库均持久化在 config 文件，换库改文件不改命令。`PRAXIS_KEY_DIR`/`PRAXIS_SEAL_PASSWORD` 手动测试同理由适配器从 `secrets.*` 供给（无需双字段，仅一份）。

## 3. 弃用标记（现状盘点 → 原生打标）

| # | 代码 | 位置 | 打标方式 |
| --- | --- | --- | --- |
| D1 | `save_plan` 命令 + `SavePlanRequest` + `db::save_plan` | commands.rs:1177 / db.rs:130 | `#[deprecated(note="任务 11 后前端已不再调用；v0.1 G1 清理")]`；注册点 `#[allow(deprecated)]` 注明意图 |
| D2 | plan_json 旧读取分支 + `StudyPlanAlias` | db.rs:191-240 | 结构体 `#[deprecated(note="plan_json 列只读兼容层；v0.1 G1 清理")]`；分支处 `#[allow(deprecated)]` |
| D3 | `POST /api/generate-plan`（非流式） | main.py:126 | 处理函数 `typing.deprecated`/`typing_extensions.deprecated` + docstring「Rust 已走 WS（任务 22），v0.1 G1 清理」 |
| D4 | `POST /api/generate-plan/stream` | main.py:154 | 同上（注意：**/api/regenerate/stream 仍在用**，不标；v0.1 G1 清理后 regen 是否 WS 化另议.entryPoint 任务 22 残留） |
| D5 | `theory?: string[]` 兼容分支 | agent.ts（已标）/ PhaseCard.vue | TS 已标；PhaseCard legacy theory 分支补 `@deprecated` 注释锚点 |
| D6 | 方案文档台账 | `docs/deprecated-cleanup.md`（新建） | 全清单 + 各标记代码坐标 + 「v0.1 第一版完成后统一清理」 |

规则：**所有标记必须.layer 可供 grep 批量定位**（`deprecated` 关键字统一），注释中文说明弃用原因与清理计划。

## 4. 任务拆分（SDD 轮）

- **任务 27（Rust 配置装载层）**：`config.rs` 扩展 schema 结构（serde camelCase 全占位默认值）+ 查找顺序 + 缺失致命错（含路径提示文案）；`config.example.json` 入库 + `.gitignore`；验收：单元测试覆盖查找顺序/schema 容错。
- **任务 28（Rust 运行 env 清零）**：logging/secret/db(load_db_config)/sidecar 四点切 config；sidecar 子进程改 `--port`/`--config` 参数（agent_entry 对应承接是任务 29，本任务先发射参数保持兼容读 env 的过渡？——**不做过渡**，任务 28+29 同轮联调）；验收：`grep -rn "env::var" src/`（除 tests_support 适配器）零命中；cargo test + clippy 零警告。
- **任务 29（Python 配置装载 + CLI）**：config.py 重写（`--config`/`--port` argparse 前置解析 + 默认搜索 + 缺失致命错）；logging_config 删 OS 约定 fallback；agent_entry.py 承接参数；`load_dotenv`/`.env*` 移除、`.env.example` 删除；tests 里的 env 装置改 config 装置；`dev.mjs` 改为生成/复制 config 并带 `--config` 启动；验收：`grep -rn "os.environ\|getenv" app/`（除测试装置）零命中；pytest 全绿；npm run check 全绿。
- **任务 30（弃用标记 + 收口 e2e）**：D1–D6 打标 + 台账文档；全量回归（cargo test/pytest/npm run check + mock 全链 GUI 冒烟）；顺带补跑任务 26 移交的真库 `--ignored` 积压（用户届时粘 DATABASE_URL 进 config.json 即可，**这本就是本次改造的收益**：测试 DB 连接从 env 粘贴改为 config 文件持久化，不用再临时 export）。

## 5. 不改动（明确排除）

- settings.json / AgentSettings 十二字段链路、`/api/config` 热更新（运行时权威不变，config 文件只接管**冷启动与运维项**）。
- LangGraph 图拓扑、NDJSON/WS 协议、持久化 schema——零协议变更。
- `.env` 不再需要但已存在的文件留给用户自行删除（`.gitignore` 同步覆盖 `config.json`）。

## 6. 提交约定

中文 commit；任务 27-30 各自完成后 `npm run check` 三端全绿才过；AGENTS.md「关键约定 §3 LLM 配置」与 README 命令表同步改述（迁移任务 29 内完成，文档属改动范围）。
