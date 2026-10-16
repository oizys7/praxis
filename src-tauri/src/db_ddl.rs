//! 数据库 DDL（建表 + 幂等演进）：与逻辑代码分离的统一管理入口。
//!
//! - 建表 DDL 对照 plan/2026-10-04_plan-persistence-pg.md；
//! - 章节化存储演化 DDL 对照任务 9 简报（子表时代的结构化拆解，已随任务 18 退役）；
//! - markdown 化演化 DDL 对照任务 18 简报：chapters 内容并入 content_md 列，
//!   四张内容子表（concepts/knowledge_points/diagrams/deep_dives）直接 DROP
//!   （用户裁定放弃旧数据兼容，不保留 legacy 兑底路径）；
//! - 两级状态机 + 大纲骨架演化 DDL 对照任务 13 简报（phases/chapters status 列 +
//!   plans 上下文三列）。
//!
//! 全部语句只用 CREATE/ADD COLUMN IF NOT EXISTS / DROP TABLE IF EXISTS，首连重放安全；
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

// ---- Task E 演进：每周学习日 + 当日已学打卡（幂等）----

/// plans 每周学习日列（JSONB 数组，1=周一..7=周日；默认全周，旧行语义不变）
pub(crate) const DDL_PLANS_STUDY_DAYS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS study_days JSONB NOT NULL DEFAULT '[1,2,3,4,5,6,7]'";

/// 当日已学打卡表：主键即 (plan_id, day)，存在即已学，无需时间戳；
/// 经外键 ON DELETE CASCADE 随 plans 行级联清理
pub(crate) const DDL_STUDY_DAY_MARKS_TABLE: &str = "CREATE TABLE IF NOT EXISTS study_day_marks (
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          day DATE NOT NULL,
          PRIMARY KEY (plan_id, day)
        )";

// ---- markdown 化演进（任务 18，幂等）----

/// 章节正文列：完整 markdown 全文（NOT NULL DEFAULT ''：行存在即含此列，
/// 空串是有意义的空正文，status 语义由该列 strip 后承载）
pub(crate) const DDL_CHAPTERS_CONTENT_MD_COL: &str =
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS content_md TEXT NOT NULL DEFAULT ''";

/// 退役子表 DROP（IF EXISTS 幂等）：外键存在于子表指向 chapters，删子表
/// 不破坏 chapters 自身及其余级联链；历史建过子表的 DB 与全新 DB 都收敛到
/// 同一终态。统一排在序列最后执行（先前建后删与只删等价，一样幂等）。
pub(crate) const DDL_DROP_CONCEPTS_TABLE: &str = "DROP TABLE IF EXISTS concepts";
pub(crate) const DDL_DROP_KNOWLEDGE_POINTS_TABLE: &str = "DROP TABLE IF EXISTS knowledge_points";
pub(crate) const DDL_DROP_DIAGRAMS_TABLE: &str = "DROP TABLE IF EXISTS diagrams";
pub(crate) const DDL_DROP_DEEP_DIVES_TABLE: &str = "DROP TABLE IF EXISTS deep_dives";

// ---- 五张子表：全部经外键 ON DELETE CASCADE 随 plans 行级联清理 ----

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

/// 章节表（content_md 由演进段追加，避免新表 DDL 与演进语句双份漂移）
pub(crate) const DDL_CHAPTERS_TABLE: &str = "CREATE TABLE IF NOT EXISTS chapters (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL REFERENCES phases(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '',
          UNIQUE (phase_id, idx)
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
    // 3. 五张子表（phases/chapters 建表先于 status/content_md 演进列）
    DDL_PHASES_TABLE,
    DDL_PROJECTS_TABLE,
    DDL_CHAPTERS_TABLE,
    DDL_RESOURCES_TABLE,
    // 4. 任务 13 演进：两级状态机 + 生成上下文列
    DDL_PHASES_STATUS_COL,
    DDL_CHAPTERS_STATUS_COL,
    DDL_PLANS_WEEKLY_HOURS_COL,
    DDL_PLANS_INTERESTS_COL,
    DDL_PLANS_BACKGROUND_COL,
    // 4.5 Task E 演进：每周学习日列 + 当日已学打卡表（均只依赖 plans 表）
    DDL_PLANS_STUDY_DAYS_COL,
    DDL_STUDY_DAY_MARKS_TABLE,
    // 5. 任务 18 演进：chapters 正文列 + 退役子表 DROP（统一收尾，
    //    历史 DB 与全新 DB 都收敛到同一终态）
    DDL_CHAPTERS_CONTENT_MD_COL,
    DDL_DROP_CONCEPTS_TABLE,
    DDL_DROP_KNOWLEDGE_POINTS_TABLE,
    DDL_DROP_DIAGRAMS_TABLE,
    DDL_DROP_DEEP_DIVES_TABLE,
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
    /// （content_md 演进列在 chapters 建表之后；四条退役子表 DROP 在序列末段；
    /// markdown 化后不再保留任何兜底 UPDATE 与子表 CREATE）
    #[test]
    fn ddl_statements_covers_all_and_keeps_order() {
        let find = |needle: &str| -> usize {
            DDL_STATEMENTS
                .iter()
                .position(|s| s.contains(needle))
                .unwrap_or_else(|| panic!("DDL 数组缺少语句: {needle}"))
        };
        let phases_table = find("CREATE TABLE IF NOT EXISTS phases");
        let chapters_table = find("CREATE TABLE IF NOT EXISTS chapters");
        // status / content_md 列在对应表建立之后
        let phases_status = find("ALTER TABLE phases ADD COLUMN IF NOT EXISTS status");
        let chapters_status = find("ALTER TABLE chapters ADD COLUMN IF NOT EXISTS status");
        let chapters_md = find("ALTER TABLE chapters ADD COLUMN IF NOT EXISTS content_md");
        assert!(
            phases_table < phases_status,
            "phases 建表必须先于其 status 列"
        );
        assert!(
            chapters_table < chapters_status,
            "chapters 建表必须先于其 status 列"
        );
        assert!(
            chapters_table < chapters_md,
            "chapters 建表必须先于其 content_md 演进列"
        );
        // 上下文三列
        find("weekly_hours");
        find("interests");
        find("background");
        // Task E：study_days 列与 study_day_marks 表都在 plans 建表之后
        let plans_table = find("CREATE TABLE IF NOT EXISTS plans");
        let study_days_col = find("ALTER TABLE plans ADD COLUMN IF NOT EXISTS study_days");
        let marks_table = find("CREATE TABLE IF NOT EXISTS study_day_marks");
        assert!(
            plans_table < study_days_col,
            "study_days 演进列必须在 plans 建表之后"
        );
        assert!(
            plans_table < marks_table,
            "study_day_marks 建表必须在 plans 建表之后（外键依赖）"
        );
        // 四张退役子表 DROP 恰好各一条且都在 content_md 演进列之后
        for (needle, label) in [
            ("DROP TABLE IF EXISTS concepts", "concepts"),
            ("DROP TABLE IF EXISTS knowledge_points", "knowledge_points"),
            ("DROP TABLE IF EXISTS diagrams", "diagrams"),
            ("DROP TABLE IF EXISTS deep_dives", "deep_dives"),
        ] {
            let pos = find(needle);
            assert!(pos > chapters_md, "退役子表 DROP 应在演进列之后: {label}");
        }
        // 序列末四条恰为四条 DROP（历史 DB 与全新 DB 都收敛到这一终态）
        for (pos, needle) in [
            (DDL_STATEMENTS.len() - 4, "concepts"),
            (DDL_STATEMENTS.len() - 3, "knowledge_points"),
            (DDL_STATEMENTS.len() - 2, "diagrams"),
            (DDL_STATEMENTS.len() - 1, "deep_dives"),
        ] {
            assert!(
                DDL_STATEMENTS[pos].contains(needle),
                "序列第 {pos} 条应为 {needle} 的 DROP"
            );
        }
        assert!(
            !DDL_STATEMENTS.iter().any(|s| s.contains("UPDATE ")),
            "markdown 化后不再保留任何兜底 UPDATE 语句"
        );
        assert!(
            !DDL_STATEMENTS
                .iter()
                .any(|s| s.contains("CREATE TABLE IF NOT EXISTS concepts")),
            "退役子表不得再出现在建表序列"
        );
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
