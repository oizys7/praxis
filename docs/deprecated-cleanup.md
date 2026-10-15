# docs/deprecated-cleanup.md — 弃用代码台账（v0.1 G1 清理）

> 用户裁定（2026-10-15）：已弃用代码统一以语言原生弃用机制打标，v0.1 第一版发布后
> 统一清理。本文件是三端弃用对象的汇总台账与删除步骤索引。
>
> **统一锚点**：所有标记（原生弃用注解的 note／配套注释）均含 `v0.1 G1 清理` 字样，
> 全仓检索：`grep -rn "v0.1 G1 清理"`。
>
> **前置条件**：v0.1 发布完成、无依赖这些入口的生产消费者。

## 一、前端（commit 7761d16，任务 30c）

| 对象 | 位置 | 弃用机制 | 清理思路 |
| --- | --- | --- | --- |
| 旧结构伦理论段 `theory?: string[]` | `frontend/src/types/agent.ts` | `@deprecated` JSDoc | 删除字段；PlanRecord 读取处同步 |
| PhaseCard 旧理论分支 | `PhaseCard.vue`（3 处，含模板 HTML 注释锚点） | 注释锚点 | 删除 legacy 理论渲染分支，chapters 为唯一渲染路径 |

## 二、Rust（commit 553ad75，任务 30a）

| 对象 | 位置 | 弃用机制 | 清理思路 |
| --- | --- | --- | --- |
| `commands::save_plan`（IPC 薄壳） | `commands.rs:1312` | `#[deprecated]` | 删命令 + invoke_handler 注册点（`main.rs:46` 语句级 allow 同删） |
| `SavePlanRequest` | `protocol.rs:201` | `#[deprecated]` | 删结构体 + 两处 use（`commands.rs:38`、`db.rs:17`，均带 allow） |
| `db::save_plan`（落库实现） | `db.rs:136` | `#[deprecated]` | 删函数 |
| `StudyPlanAlias`（type alias） | `db.rs:266` | `#[deprecated]` | 删别名 + `get_plan_by_pool` 内 plan_json 只读兼容分支（`db.rs:218-221`，let 级 allow） |
| config.rs / config.yaml 迁移期整链（含配套单测） | `config.rs`（dead_code 防御） | 注释锚点 | config.json 链稳定运行一个发布周期后整体删除 |
| `plan_json` 数据列 + db_ddl.rs 列迁移语句 | DB | 注释锚点（历史行读完清库后） | 最后一轮删列迁移（其余表结构不在本批） |

⚠️ 勿误删：`protocol::StudyPlan` 为规范化路径共享类型（`plan_store::load_study_plan`
仍在用），不属于本批。

## 三、Python（任务 30b 补全）

## 三、Python（commit acb4df8，任务 30b）

| 对象 | 位置 | 弃用机制 | 清理思路 |
| --- | --- | --- | --- |
| `POST /api/generate-plan`（非流式，D3） | `app/main.py:129` | `@warnings.deprecated`（route 内层包 fn） | 删 route + handler + 对应 test_stream 用例 |
| `POST /api/generate-plan/stream`（NDJSON，D4） | `app/main.py:163` | `@warnings.deprecated` | 删 route + handler + 用例 |
| `_stream_plan_events`（D4 独占薄包装） | `app/main.py:199` | `@warnings.deprecated` | 随 D4 端点一并删 |
| `_ndjson_line`（与 regenerate 共用） | `app/main.py` | 注释锚点（未打标） | regenerate 退役时一并清；G1 时改单消费者注释 |
| `_plan_event_dicts`（WS 主链共享） | `app/tasks.py` | docstring 消费面注释（未打标） | G1 时仅减消费面（后台任务仍用） |

⚠️ 勿误删：`/api/regenerate/stream`（Rust 重生成链仍在消费，非 D3/D4）。

**装饰器顺序语义（实测，CPython 3.13.12）**：`@warnings.deprecated` 在 `@app.post`
**下层**（先应用）——运行态 HTTP 调用会发 DeprecationWarning；`iscoroutinefunction`
保留、签名经 `__wrapped__` 还原、FastAPI 校验/OpenAPI 不受影响。


## 四、删除顺序建议

1. 全仓 `grep -rn "v0.1 G1 清理"` 拉最新清单（行号以届时 HEAD 为准）。
2. 按端独立清理与验证（前端 vue-tsc / Rust cargo test+clippy / Python pytest），三端可并行。
3. 每端删除后同步协议层（若涉及 wire 字段：agent.ts / protocol.rs / schemas.py 三处对齐）。
4. 数据项（plan_json 列）放最后一轮：确认线上旧行已读完（历史 plan_json 行清库）后
   再删列与迁移语句。
5. 收尾删除本台账文件。
