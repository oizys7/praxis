//! 数据库 DDL（建表 + 幂等演进）：与逻辑代码分离的统一管理入口。
//!
//! - 建表 DDL 对照 plan/2026-10-04_plan-persistence-pg.md；
//! - 章节化存储演化 DDL 对照任务 9 简报（九表逐字定义，与 plan_store.rs 读写对齐）；
//! - 两级状态机 + 大纲骨架演化 DDL 对照任务 13 简报（phases/chapters status 列 +
//!   plans 上下文三列 + 旧数据兜底 UPDATE，幂等重放安全）。
//!
//! 全部语句只用 CREATE/ADD COLUMN IF NOT EXISTS / 幂等 UPDATE，首连重放安全；
//! 子表均经外键 ON DELETE CASCADE 随 plans 行级联清理。

use sqlx::PgPool;

use crate::db::fail;
use crate::error::AgentError;

// ---------------------------------------------------------------------------
// 单条 DDL 语句（pub(crate)：模块内测试校验数组完整性）
// ---------------------------------------------------------------------------

/// plans 主表（云端唯一真源）
pub(crate) const DDL_PLANS_TABLE: &str = "CREATE TABLE IF NOT EXISTS plans (
            id         UUID PRIMARY KEY,
            title      TEXT NOT NULL,
            goal       TEXT NOT NULL,
            overview   TEXT NOT NULL DEFAULT '',
            level      TEXT NOT NULL DEFAULT '',
            weeks      INT  NOT NULL DEFAULT 0,
            plan_json  JSONB NOT NULL,
            favorite   BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )";

/// 列表页排序索引
pub(crate) const DDL_PLANS_CREATED_AT_IDX: &str =
    "CREATE INDEX IF NOT EXISTS idx_plans_created_at ON plans (created_at DESC)";

// ---- 章节化存储演化（任务 9，幂等）----

/// 旧列 plan_json 保留只读兼容：规范化新行不再写整串 JSON（新写入一律 NULL），
/// 对已可空的列执行 DROP NOT NULL 幂等成功
pub(crate) const DDL_PLAN_JSON_NULLABLE: &str =
    "ALTER TABLE plans ALTER COLUMN plan_json DROP NOT NULL";

/// plans 生成状态列：generating | complete | failed（默认 'complete' 保持旧行语义）
pub(crate) const DDL_PLANS_STATUS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'complete'";

/// plans 预计总学时列
pub(crate) const DDL_PLANS_ESTIMATED_HOURS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS estimated_total_hours INT NOT NULL DEFAULT 0";

// ---- 任务 13 演进：两级状态机 + 生成上下文列 ----

/// 阶段状态列：pending（骨架已入库）| complete | failed
pub(crate) const DDL_PHASES_STATUS_COL: &str =
    "ALTER TABLE phases ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'pending'";

/// 章节状态列：pending | complete | failed
pub(crate) const DDL_CHAPTERS_STATUS_COL: &str =
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'pending'";

/// plans 上下文列：每周可用小时（regenerate 时重建 PlanRequest 用）
pub(crate) const DDL_PLANS_WEEKLY_HOURS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS weekly_hours INT NOT NULL DEFAULT 0";

/// plans 上下文列：偏好方向（JSONB 数组）
pub(crate) const DDL_PLANS_INTERESTS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS interests JSONB NOT NULL DEFAULT '[]'";

/// plans 上下文列：已有基础描述
pub(crate) const DDL_PLANS_BACKGROUND_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS background TEXT NOT NULL DEFAULT ''";

// ---- 旧数据兜底（控制者裁定，逐字）----
// ADD COLUMN DEFAULT 'pending' 会把升级前（无 status 列时代）已有数据的行误标，
// 靠「子表有数据 → complete」的兜底修正。幂等：只动 status='pending' 的行，
// 已 complete 的行不受影响，每次首连重放安全。

/// 有章节（即有数据）的升级前旧阶段行批量修正为 complete
pub(crate) const DDL_LEGACY_PHASES_BACKFILL: &str = "UPDATE phases p SET status='complete' WHERE status='pending' AND EXISTS(SELECT 1 FROM chapters c WHERE c.phase_id=p.id)";

/// 四张内容子表任一有数据的升级前旧章节行批量修正为 complete
pub(crate) const DDL_LEGACY_CHAPTERS_BACKFILL: &str = "UPDATE chapters ch SET status='complete' WHERE status='pending' AND (
       EXISTS(SELECT 1 FROM concepts x WHERE x.chapter_id=ch.id) OR EXISTS(SELECT 1 FROM knowledge_points x WHERE x.chapter_id=ch.id)
       OR EXISTS(SELECT 1 FROM diagrams x WHERE x.chapter_id=ch.id) OR EXISTS(SELECT 1 FROM deep_dives x WHERE x.chapter_id=ch.id))";

// ---- 八张子表：全部经外键 ON DELETE CASCADE 随 plans 行级联清理 ----

/// 阶段表
pub(crate) const DDL_PHASES_TABLE: &str = "CREATE TABLE IF NOT EXISTS phases (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, name TEXT NOT NULL, week_range TEXT NOT NULL DEFAULT '',
          objectives JSONB NOT NULL DEFAULT '[]', practice JSONB NOT NULL DEFAULT '[]',
          assignments JSONB NOT NULL DEFAULT '[]',
          UNIQUE (plan_id, idx)
        )";

/// 阶段收尾项目表（每阶段至多一行）
pub(crate) const DDL_PROJECTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS projects (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL UNIQUE REFERENCES phases(id) ON DELETE CASCADE,
          name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', background TEXT NOT NULL DEFAULT '',
          steps JSONB NOT NULL DEFAULT '[]', acceptance_criteria JSONB NOT NULL DEFAULT '[]',
          estimated_hours INT NOT NULL DEFAULT 0
        )";

/// 章节表
pub(crate) const DDL_CHAPTERS_TABLE: &str = "CREATE TABLE IF NOT EXISTS chapters (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL REFERENCES phases(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '',
          UNIQUE (phase_id, idx)
        )";

/// 章节概念表
pub(crate) const DDL_CONCEPTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS concepts (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, name TEXT NOT NULL, definition TEXT NOT NULL DEFAULT ''
        )";

/// 章节知识点表
pub(crate) const DDL_KNOWLEDGE_POINTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS knowledge_points (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, detail TEXT NOT NULL DEFAULT ''
        )";

/// 章节图示表
pub(crate) const DDL_DIAGRAMS_TABLE: &str = "CREATE TABLE IF NOT EXISTS diagrams (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL DEFAULT '', mermaid TEXT NOT NULL DEFAULT '', caption TEXT NOT NULL DEFAULT ''
        )";

/// 章节深入内容表
pub(crate) const DDL_DEEP_DIVES_TABLE: &str = "CREATE TABLE IF NOT EXISTS deep_dives (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, topic TEXT NOT NULL, angle TEXT NOT NULL DEFAULT '', content TEXT NOT NULL
        )";

/// 推荐学习资源表
pub(crate) const DDL_RESOURCES_TABLE: &str = "CREATE TABLE IF NOT EXISTS resources (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, type TEXT NOT NULL DEFAULT 'docs', note TEXT NOT NULL DEFAULT ''
        )";

// ---------------------------------------------------------------------------
// 执行序列
// ---------------------------------------------------------------------------

/// 首连建表/演进全序列：顺序敏感（phases/chapters 表必须先建，status 列与
/// 兜底 UPDATE 才有落点），浓缩为位数组由 ensure_schema 逐条执行。
pub(crate) const DDL_STATEMENTS: &[&str] = &[
    // 1. 主表 + 索引
    DDL_PLANS_TABLE,
    DDL_PLANS_CREATED_AT_IDX,
    // 2. 章节化演化（任务 9）
    DDL_PLAN_JSON_NULLABLE,
    DDL_PLANS_STATUS_COL,
    DDL_PLANS_ESTIMATED_HOURS_COL,
    // 3. 八张子表（phases/chapters 先于 status 列，兜底 UPDATE 才有对象）
    DDL_PHASES_TABLE,
    DDL_PROJECTS_TABLE,
    DDL_CHAPTERS_TABLE,
    DDL_CONCEPTS_TABLE,
    DDL_KNOWLEDGE_POINTS_TABLE,
    DDL_DIAGRAMS_TABLE,
    DDL_DEEP_DIVES_TABLE,
    DDL_RESOURCES_TABLE,
    // 4. 任务 13 演进：两级状态机 + 生成上下文列
    DDL_PHASES_STATUS_COL,
    DDL_CHAPTERS_STATUS_COL,
    DDL_PLANS_WEEKLY_HOURS_COL,
    DDL_PLANS_INTERESTS_COL,
    DDL_PLANS_BACKGROUND_COL,
    // 5. 旧数据兜底（控制者裁定）：子表有数据的行回标 complete，幂等重放安全
    DDL_LEGACY_PHASES_BACKFILL,
    DDL_LEGACY_CHAPTERS_BACKFILL,
];

/// 执行全部 DDL（逐条跑，任一失败即整体失败并归一为 Db 错误）。
/// 幂等语义：全部语句可重放，因此同一批（组件升级/网络重连重建池）首连多次执行安全。
pub(crate) async fn run(pool: &PgPool) -> Result<(), AgentError> {
    for (i, sql) in DDL_STATEMENTS.iter().enumerate() {
        sqlx::query(sql)
            .execute(pool)
            .await
            .map_err(|e| fail(&format!("初始化/演化表结构（第 {} 条 DDL）", i + 1), e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 数组完整性：关键语句必须全部在执行序列内且顺序正确
    /// （phases/chapters status 列在子表建表之后、兜底 UPDATE 在最后）
    #[test]
    fn ddl_statements_covers_all_and_keeps_order() {
        let find = |needle: &str| -> usize {
            DDL_STATEMENTS
                .iter()
                .position(|s| s.contains(needle))
                .unwrap_or_else(|| panic!("DDL 数组缺少含「{needle}」的语句"))
        };
        // 建表在前
        assert!(find("CREATE TABLE IF NOT EXISTS plans") < find("gen_random_uuid"));
        // status 列在对应表建立之后（表 DDL 里含列名，判别用表演进字样）
        let phases_table = find("phases");
        let chapters_table = find("chapters");
        let phases_status = find("ALTER TABLE phases ADD COLUMN IF NOT EXISTS status");
        let chapters_status = find("ALTER TABLE chapters ADD COLUMN IF NOT EXISTS status");
        assert!(
            phases_table < phases_status,
            "phases 建表必须先于其 status 列"
        );
        assert!(
            chapters_table < chapters_status,
            "chapters 建表必须先于其 status 列"
        );
        // 上下文三列
        find("weekly_hours");
        find("interests");
        find("background");
        // 兜底 UPDATE 在最后且恰好两条
        assert!(
            find("SET status='complete' WHERE status='pending' AND EXISTS(SELECT 1 FROM chapters")
                == DDL_STATEMENTS.len() - 2,
            "第一条兜底 UPDATE 应在倒数第二条"
        );
        assert_eq!(
            find("OR EXISTS(SELECT 1 FROM deep_dives"),
            DDL_STATEMENTS.len() - 1
        );
        // 兜底UPDATE之后不能还有其他语句
        assert_eq!(DDL_STATEMENTS.last().unwrap().contains("UPDATE"), true);
    }

    /// 逐条语句的方可执行性由集成测试在真库验证；本模块只校验无空白 SQL 混入
    #[test]
    fn statements_are_non_empty() {
        assert!(
            DDL_STATEMENTS.iter().all(|s| !s.trim().is_empty()),
            "DDL 数组不得混入空语句"
        );
    }
}
