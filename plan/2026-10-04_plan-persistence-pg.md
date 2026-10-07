# 学习计划云端持久化（PostgreSQL 直连）— 规格与实现计划

> 2026-10-04 · 架构决策：**云端唯一真源**（无本地库、无同步逻辑）· Python 零知情 · 协议零改动

## 1. 目标与范围

- 计划生成后由用户点「保存到历史」显式入库；历史页提供搜索 / 收藏置顶 / 查看 / 删除
- 存储：阿里云 ECS 自建 PostgreSQL 17（Docker），**pgvector 预留**（将来语义搜索零迁移）
- 云端为唯一真源：断网时历史功能不可用（可接受——核心的生成链路本就依赖 LLM 网络）；保存失败时计划仍在前端内存，提示重试不丢数据
- 架构约束：数据库归 Rust 管，Python 与三端协议（plan 请求/响应）零改动

## 2. 云端部署（ECS，Docker）

镜像用 `pgvector/pgvector:pg17`（官方 PG 17 + vector 扩展，比纯 postgres 镜像多一步都不用做）：

```bash
docker run -d --name praxis-pg --restart always \
  -e POSTGRES_DB=praxis -e POSTGRES_USER=praxis -e POSTGRES_PASSWORD=<强密码> \
  -p 5432:5432 pgvector/pgvector:pg17
```

安全清单（自建公网 PG 必做）：

| 措施 | 说明 |
|---|---|
| ECS 安全组放行 5432 | 家宽 IP 会变 → 对 0.0.0.0/0 放行，靠下列措施收口 |
| 专用账号 | `praxis` 仅授 `praxis` 库全权（含建表），禁 root/超级用户直连 |
| 强密码 + 高位端口（可选） | 改映射 `-p 15432:5432` 躲扫描噪音 |
| SSL | v1 默认 `sslmode=disable`（自签证书配置繁琐）；升级路径写入 docs，后续补 |
| 备份 | 阿里云自动快照（每日）+ `pg_dump` 周备 |

## 3. 表结构（Rust 首连自动建表，零手工 DDL 步骤）

```sql
CREATE TABLE IF NOT EXISTS plans (
  id          UUID PRIMARY KEY,          -- Rust 端 uuid v4 生成
  title       TEXT NOT NULL,
  goal        TEXT NOT NULL,
  overview    TEXT NOT NULL DEFAULT '',  -- 学习整体描述（协议已有字段）
  level       TEXT NOT NULL DEFAULT '',
  weeks       INT  NOT NULL DEFAULT 0,
  plan_json   JSONB NOT NULL,            -- 整份 StudyPlan
  favorite    BOOLEAN NOT NULL DEFAULT FALSE,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_plans_created_at ON plans (created_at DESC);
```

搜索用 `LIKE`（个人数据量级，tsvector/GIN 属过度设计）；向量列为将来 `ALTER TABLE plans ADD COLUMN embedding vector(...)` 一步到位。

## 4. Rust 侧

| 项 | 内容 |
|---|---|
| 依赖 | `sqlx = { version = "0.8", features = ["postgres", "runtime-tokio", "chrono", "uuid"] }`、`uuid = { features = ["v4"] }`（chrono 已有）；v1 不加 TLS 特性 |
| `settings.rs` | 新增 `DbConfig { enabled, host, port, database, user, password, ssl }`，`AgentSettings.db`（serde default 向后兼容旧 settings.json）；仅本地存储，**不进 `/api/config` 推送载荷**（Pydantic 默认忽略额外字段，Python 零改动） |
| `db.rs`（新） | 惰性 `PgPool`（acquire_timeout 10s）+ `ensure_schema`（首连建表）+ CRUD：`save_plan / list_plans / get_plan / delete_plan / toggle_favorite`；每查询 10s 超时 |
| `error.rs` | `AgentError::Db(String)` 变体，hint「无法连接云端数据库，请检查 ECS 状态、网络与设置页配置」 |
| `commands.rs` | 6 个薄壳 command：`save_plan / list_plans / get_plan / delete_plan / toggle_favorite / test_db_connection` |
| `protocol.rs` | `PlanRecord`（id/title/goal/overview/level/weeks/favorite/createdAt/plan 复用 StudyPlan）、`PlanSummary`（不含 plan） |

## 5. 前端

| 项 | 内容 |
|---|---|
| `types/agent.ts` | `DbConfig` / `PlanSummary` / `PlanRecord` 接口；`AgentSettings.db`；`defaultAgentSettings` 补默认 |
| `usePlanHistory.ts`（新） | 六个 command 封装 + 复用 `normalizeError` 归一 |
| `PlanResult.vue` | 「保存到历史」按钮：保存中禁用；成功显示已保存状态（含时间） |
| `HistoryPage.vue`（新） | 进入「学习计划」页默认显示**计划列表**：搜索框（防抖 LIKE）· 卡片（标题/goal/overview 摘要/level/weeks/生成时间/★收藏/删除）· 「新建」按钮 → 切换到生成表单 · 点击查看 → 注入表单页展示 · 连接失败友好提示 + 重试 |
| `App.vue` | 学习计划页改为两态容器（`list` 默认 ↔ `create` 即现有表单+结果）：点新建进生成流程，生成/保存后可返回列表 |
| `Settings.vue` | 云端数据库区块：enabled 开关 + host/port/database/user/password + 「测试连接」按钮 |

## 6. 错误处理矩阵

| 场景 | 行为 |
|---|---|
| db 未启用 | 前端隐藏历史入口；保存按钮提示「先在设置页启用」 |
| 连接失败/超时 | `AgentError::Db` → 前端红色提示 + 重试按钮 |
| 首连建表失败 | 同上（praxis 账号缺权限时会在此暴露） |
| 删除/收藏目标不存在 | 幂等处理，返回成功 |

## 7. 实施批次

1. **Rust**：Cargo.toml + settings.rs DbConfig + db.rs + error.rs + commands.rs + main.rs 注册 → `cargo check/clippy`
2. **前端**：types + usePlanHistory + 学习计划页两态改造（列表默认/新建进表单）+ PlanResult 保存按钮 + 设置页区块 → `vue-tsc`
3. **部署文档**：`docs/deploy-ecs.md`（含 Docker 命令、安全组、账号授权 SQL、快照建议）
4. **验证**：`npm run check` 三端绿 → 用户按文档部署 ECS 后连真实库端到端冒烟

## 8. 演进注记（不在本次范围）

- 向量搜索：pgvector `ALTER TABLE ... ADD COLUMN embedding vector(1024)` + HNSW 索引
- 多用户发布：ECS 上加服务端 API 层（届时再议），客户端 Rust commands 接口形状不变，前端零改动
