//! 数据库 DDL（建表 + 幂等演进 + 时间戳规范 + 注释）：与逻辑代码分离的统一管理入口。
//!
//! - 建表 DDL 对照 plan/2026-10-04_plan-persistence-pg.md；
//! - 章节化存储演化 DDL 对照任务 9 简报（子表时代的结构化拆解，已随任务 18 退役）；
//! - markdown 化演化 DDL 对照任务 18 简报：chapters 内容并入 content_md 列，
//!   四张内容子表（concepts/knowledge_points/diagrams/deep_dives）直接 DROP
//!   （用户裁定放弃旧数据兼容，不保留 legacy 兑底路径）；
//! - 两级状态机 + 大纲骨架演化 DDL 对照任务 13 简报（phases/chapters status 列 +
//!   plans 上下文三列）。
//! - 时间戳规范（2026-10-07 用户指令）：全部非关联业务表统一 created_at /
//!   updated_at，updated_at 由 BEFORE UPDATE 触发器自动维护，业务 SQL 无需
//!   手写（既有的显式 SET 与触发器同值共存，无害）；
//! - 注释规范（2026-10-07 用户指令）：全部表与列带中文 COMMENT，随 DDL 幂等重放。
//!
//! 全部语句只用 CREATE/ADD COLUMN IF NOT EXISTS / DROP TABLE IF EXISTS / CREATE OR
//! REPLACE / COMMENT ON（天然幂等），首连重放安全；触发器先 DROP IF EXISTS 再 CREATE。
//! 子表均经外键 ON DELETE CASCADE 随 plans 行级联清理。
//! 注意：sqlx prepared statement 不支持多语句，每条语句独立执行。

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

/// plans 开始学习日期列（null = 未开始；start_learning 写入）
pub(crate) const DDL_PLANS_LEARNING_STARTED_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS learning_started_at DATE";

/// plans 学习状态三分类列：not_started | active | completed
pub(crate) const DDL_PLANS_LEARNING_STATUS_COL: &str =
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS learning_status TEXT NOT NULL DEFAULT 'not_started'";

/// 学习日程表：开始学习后按每周学习日落定的逐章安排（date=哪天学，
/// done=该行是否打卡，每章独立）；随所属计划级联清理
pub(crate) const DDL_STUDY_SCHEDULE_TABLE: &str = "CREATE TABLE IF NOT EXISTS study_schedule (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          date DATE NOT NULL,
          phase_idx INT NOT NULL,
          chapter_idx INT NOT NULL,
          chapter_title TEXT NOT NULL DEFAULT '',
          phase_name TEXT NOT NULL DEFAULT '',
          done BOOLEAN NOT NULL DEFAULT FALSE,
          created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          UNIQUE (plan_id, date, phase_idx, chapter_idx)
        )";

/// 月/年日历查询索引（按日期范围扫）
pub(crate) const DDL_STUDY_SCHEDULE_DATE_IDX: &str =
    "CREATE INDEX IF NOT EXISTS idx_study_schedule_date ON study_schedule (date)";

// ---- markdown 化演进（任务 18，幂等）----

/// 章节正文列：完整 markdown 全文（NOT NULL DEFAULT ''：行存在即含此列，
/// 空串是有意义的空正文，status 语义由该列 strip 后承载）
pub(crate) const DDL_CHAPTERS_CONTENT_MD_COL: &str =
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS content_md TEXT NOT NULL DEFAULT ''";

/// 章型列（2026-10-21 kind 引入）：concept / practice / project / review，
/// 决定正文形态要求与前端口径；空串/未知值由 plan_store 按 concept 兜底
pub(crate) const DDL_CHAPTERS_KIND_COL: &str =
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'concept'";

/// 章节核心术语列（JSONB 数组）：只用于汇总全计划术语表，不参与生成决策
pub(crate) const DDL_CHAPTERS_KEY_TERMS_COL: &str =
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS key_terms JSONB NOT NULL DEFAULT '[]'::jsonb";

/// 退役子表 DROP（IF EXISTS 幂等）：外键存在于子表指向 chapters，删子表
/// 不破坏 chapters 自身及其余级联链；历史建过子表的 DB 与全新 DB 都收敛到
/// 同一终态。统一排在序列最后执行（先前建后删与只删等价，一样幂等）。
pub(crate) const DDL_DROP_CONCEPTS_TABLE: &str = "DROP TABLE IF EXISTS concepts";
pub(crate) const DDL_DROP_KNOWLEDGE_POINTS_TABLE: &str = "DROP TABLE IF EXISTS knowledge_points";
pub(crate) const DDL_DROP_DIAGRAMS_TABLE: &str = "DROP TABLE IF EXISTS diagrams";
pub(crate) const DDL_DROP_DEEP_DIVES_TABLE: &str = "DROP TABLE IF EXISTS deep_dives";

/// 旧打卡表退役（2026-10-18 用户裁定：新 study_schedule 自带打卡字段后
/// 直接退役，旧打卡数据不迁移）
pub(crate) const DDL_DROP_STUDY_DAY_MARKS_TABLE: &str = "DROP TABLE IF EXISTS study_day_marks";

// ---- 五张子表：全部经外键 ON DELETE CASCADE 随 plans 行级联清理 ----
// （均含时间戳规范列 created_at / updated_at）

/// 阶段表
pub(crate) const DDL_PHASES_TABLE: &str = "CREATE TABLE IF NOT EXISTS phases (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, name TEXT NOT NULL, week_range TEXT NOT NULL DEFAULT '',
          objectives JSONB NOT NULL DEFAULT '[]', practice JSONB NOT NULL DEFAULT '[]',
          assignments JSONB NOT NULL DEFAULT '[]',
          created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          UNIQUE (plan_id, idx)
        )";

/// 阶段收尾项目表（每阶段至多一行）
pub(crate) const DDL_PROJECTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS projects (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL UNIQUE REFERENCES phases(id) ON DELETE CASCADE,
          name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', background TEXT NOT NULL DEFAULT '',
          steps JSONB NOT NULL DEFAULT '[]', acceptance_criteria JSONB NOT NULL DEFAULT '[]',
          estimated_hours INT NOT NULL DEFAULT 0,
          created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )";

/// 章节表（content_md 由演进段追加，避免新表 DDL 与演进语句双份漂移）
pub(crate) const DDL_CHAPTERS_TABLE: &str = "CREATE TABLE IF NOT EXISTS chapters (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL REFERENCES phases(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '',
          kind TEXT NOT NULL DEFAULT 'concept',
          key_terms JSONB NOT NULL DEFAULT '[]'::jsonb,
          created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          UNIQUE (phase_id, idx)
        )";

/// 推荐学习资源表
pub(crate) const DDL_RESOURCES_TABLE: &str = "CREATE TABLE IF NOT EXISTS resources (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, type TEXT NOT NULL DEFAULT 'docs', note TEXT NOT NULL DEFAULT '',
          created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )";

// ---------------------------------------------------------------------------
// 时间戳规范段：updated_at 自动维护（业务 UPDATE 无需手写 updated_at）
// ---------------------------------------------------------------------------

/// updated_at 自动维护函数（幂等）：任意 UPDATE 前刷新 NEW.updated_at
const DDL_SET_UPDATED_AT_FN: &str =
    "CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
         BEGIN
           NEW.updated_at := now();
           RETURN NEW;
         END;
         $$ LANGUAGE plpgsql";

// ---------------------------------------------------------------------------
// 注释规范段：全部表与列的中文 COMMENT（天然幂等，随 DDL 重放）
// ---------------------------------------------------------------------------

/// 表与列注释（结构与上一节 DDL 一一对应；新增列/表时同步维护）
const COMMENT_STATEMENTS: &[&str] = &[
    // ---- plans ----
    "COMMENT ON TABLE plans IS '学习计划主表：学习路线规划的云端唯一真源'",
    "COMMENT ON COLUMN plans.id IS '计划主键（UUID）'",
    "COMMENT ON COLUMN plans.title IS '计划标题'",
    "COMMENT ON COLUMN plans.goal IS '学习目标'",
    "COMMENT ON COLUMN plans.overview IS '计划概述'",
    "COMMENT ON COLUMN plans.level IS '当前水平'",
    "COMMENT ON COLUMN plans.weeks IS '计划周数'",
    "COMMENT ON COLUMN plans.plan_json IS '旧版整计划 JSON（只读兼容；规范化后新行不再写入）'",
    "COMMENT ON COLUMN plans.favorite IS '收藏标记'",
    "COMMENT ON COLUMN plans.created_at IS '创建时间'",
    "COMMENT ON COLUMN plans.updated_at IS '修改时间（触发器自动维护）'",
    "COMMENT ON COLUMN plans.status IS '行级生成状态：generating | complete | failed'",
    "COMMENT ON COLUMN plans.estimated_total_hours IS '预计总学时（小时）'",
    "COMMENT ON COLUMN plans.weekly_hours IS '每周可用学习小时（重生成时重建请求上下文用）'",
    "COMMENT ON COLUMN plans.interests IS '偏好方向（JSONB 字符串数组）'",
    "COMMENT ON COLUMN plans.background IS '已有基础描述'",
    "COMMENT ON COLUMN plans.study_days IS '每周学习日（JSONB 数字数组，1=周一..7=周日）'",
    "COMMENT ON COLUMN plans.learning_started_at IS '开始学习日期（null = 未开始；开始学习接口写入）'",
    "COMMENT ON COLUMN plans.learning_status IS '学习状态三分类：not_started | active | completed（学习中全章生成完成自动流转）'",
    "COMMENT ON COLUMN plans.study_total IS '学习打卡日程总数（冗余计数：应用层在开始学习/重排事务内增量维护，列表直读免统计；存量数据不回填）'",
    "COMMENT ON COLUMN plans.study_done IS '已打卡章数（冗余计数：应用层在打卡/取消打卡事务内增量维护）'",
    // ---- phases ----
    "COMMENT ON TABLE phases IS '学习阶段表：计划的分阶段骨架，随所属计划级联删除'",
    "COMMENT ON COLUMN phases.id IS '阶段主键（UUID）'",
    "COMMENT ON COLUMN phases.plan_id IS '所属计划 id（ON DELETE CASCADE）'",
    "COMMENT ON COLUMN phases.idx IS '阶段序号（计划内唯一）'",
    "COMMENT ON COLUMN phases.name IS '阶段名称'",
    "COMMENT ON COLUMN phases.week_range IS '周范围展示文本'",
    "COMMENT ON COLUMN phases.objectives IS '阶段目标（JSONB 字符串数组）'",
    "COMMENT ON COLUMN phases.practice IS '实操要点（JSONB 字符串数组）'",
    "COMMENT ON COLUMN phases.assignments IS '作业清单（JSONB 字符串数组）'",
    "COMMENT ON COLUMN phases.status IS '阶段状态：pending（骨架已入库）| complete | failed'",
    "COMMENT ON COLUMN phases.created_at IS '创建时间'",
    "COMMENT ON COLUMN phases.updated_at IS '修改时间（触发器自动维护）'",
    // ---- projects ----
    "COMMENT ON TABLE projects IS '阶段收尾项目表：每阶段至多一行，随所属阶段级联删除'",
    "COMMENT ON COLUMN projects.id IS '项目主键（UUID）'",
    "COMMENT ON COLUMN projects.phase_id IS '所属阶段 id（唯一，ON DELETE CASCADE）'",
    "COMMENT ON COLUMN projects.name IS '项目名称'",
    "COMMENT ON COLUMN projects.description IS '项目描述'",
    "COMMENT ON COLUMN projects.background IS '项目背景与动机'",
    "COMMENT ON COLUMN projects.steps IS '建议实施步骤（JSONB 字符串数组）'",
    "COMMENT ON COLUMN projects.acceptance_criteria IS '验收标准（JSONB 字符串数组）'",
    "COMMENT ON COLUMN projects.estimated_hours IS '预计投入学时（小时）'",
    "COMMENT ON COLUMN projects.created_at IS '创建时间'",
    "COMMENT ON COLUMN projects.updated_at IS '修改时间（触发器自动维护）'",
    // ---- chapters ----
    "COMMENT ON TABLE chapters IS '学习章节表：阶段下的章节化内容，随所属阶段级联删除'",
    "COMMENT ON COLUMN chapters.id IS '章节主键（UUID）'",
    "COMMENT ON COLUMN chapters.phase_id IS '所属阶段 id（ON DELETE CASCADE）'",
    "COMMENT ON COLUMN chapters.idx IS '章节序号（阶段内唯一）'",
    "COMMENT ON COLUMN chapters.title IS '章节标题'",
    "COMMENT ON COLUMN chapters.summary IS '章节摘要'",
    "COMMENT ON COLUMN chapters.status IS '章节状态：pending | complete | failed'",
    "COMMENT ON COLUMN chapters.content_md IS '章节正文 markdown 全文（空串 = 正文尚未生成）'",
    "COMMENT ON COLUMN chapters.kind IS '章型：concept | practice | project | review'",
    "COMMENT ON COLUMN chapters.key_terms IS '本章核心术语数组（JSONB），用于汇总全计划术语表'",
    "COMMENT ON COLUMN chapters.created_at IS '创建时间'",
    "COMMENT ON COLUMN chapters.updated_at IS '修改时间（触发器自动维护）'",
    // ---- resources ----
    "COMMENT ON TABLE resources IS '推荐学习资源表：计划级资源清单，随所属计划级联删除'",
    "COMMENT ON COLUMN resources.id IS '资源主键（UUID）'",
    "COMMENT ON COLUMN resources.plan_id IS '所属计划 id（ON DELETE CASCADE）'",
    "COMMENT ON COLUMN resources.idx IS '资源序号'",
    "COMMENT ON COLUMN resources.title IS '资源名称'",
    "COMMENT ON COLUMN resources.type IS '资源类型（如 docs | video | book | interactive）'",
    "COMMENT ON COLUMN resources.note IS '备注说明'",
    "COMMENT ON COLUMN resources.created_at IS '创建时间'",
    "COMMENT ON COLUMN resources.updated_at IS '修改时间（触发器自动维护）'",
    // ---- study_schedule ----
    "COMMENT ON TABLE study_schedule IS '学习日程表：开始学习后按每周学习日落定的逐章安排（date=哪天学，done=该行是否打卡），随所属计划级联删除'",
    "COMMENT ON COLUMN study_schedule.id IS '日程行主键（UUID）'",
    "COMMENT ON COLUMN study_schedule.plan_id IS '所属计划 id（ON DELETE CASCADE）'",
    "COMMENT ON COLUMN study_schedule.date IS '学习日期（哪天学）'",
    "COMMENT ON COLUMN study_schedule.phase_idx IS '阶段序号（plans 内定位阶段）'",
    "COMMENT ON COLUMN study_schedule.chapter_idx IS '章节序号（阶段内定位章节）'",
    "COMMENT ON COLUMN study_schedule.chapter_title IS '章节标题（冗余展示名，重排时刷新）'",
    "COMMENT ON COLUMN study_schedule.phase_name IS '阶段名称（冗余展示名，重排时刷新）'",
    "COMMENT ON COLUMN study_schedule.done IS '该行是否打卡（每章独立勾选；仅允许 date≤今天）'",
    "COMMENT ON COLUMN study_schedule.created_at IS '创建时间'",
    "COMMENT ON COLUMN study_schedule.updated_at IS '修改时间（触发器自动维护）'",
];

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
    // 4.5 学习日程演进：每周学习日列 + 学习状态/开始日期列 + 学习日程表
    //    （均只依赖 plans 表；旧打卡表 study_day_marks 在第 5 段退役）
    DDL_PLANS_STUDY_DAYS_COL,
    DDL_PLANS_LEARNING_STARTED_COL,
    DDL_PLANS_LEARNING_STATUS_COL,
    DDL_STUDY_SCHEDULE_TABLE,
    DDL_STUDY_SCHEDULE_DATE_IDX,
    // 5. 任务 18 演进：chapters 正文列 + 退役子表 DROP（统一收尾，
    //    历史 DB 与全新 DB 都收敛到同一终态）
    DDL_CHAPTERS_CONTENT_MD_COL,
    DDL_CHAPTERS_KIND_COL,
    DDL_CHAPTERS_KEY_TERMS_COL,
    DDL_DROP_CONCEPTS_TABLE,
    DDL_DROP_KNOWLEDGE_POINTS_TABLE,
    DDL_DROP_DIAGRAMS_TABLE,
    DDL_DROP_DEEP_DIVES_TABLE,
    DDL_DROP_STUDY_DAY_MARKS_TABLE,
    // 6. 时间戳规范补列（子表 CREATE 已含列，此处为已部署旧库幂等补齐；
    //    必须在对应建表之后；旧行取迁移时刻兑底，此后由触发器接管）
    "ALTER TABLE phases ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE phases ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE projects ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE projects ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE chapters ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE resources ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE resources ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE study_schedule ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    "ALTER TABLE study_schedule ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
    // 6.5 学习打卡计数冗余列（打卡页列表直读，免 COUNT 子查询；应用层在
    //     开始学习/重排/打卡事务内增量维护；存量数据不回填，保持 0）
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS study_total INT NOT NULL DEFAULT 0",
    "ALTER TABLE plans ADD COLUMN IF NOT EXISTS study_done INT NOT NULL DEFAULT 0",
    // 7. updated_at 自动维护触发器（必须在补列之后：updated_at 列得先存在；
    //    CREATE OR REPLACE 重放/并发安全（PG14+），显式 SET updated_at 与触发器同值共存无害）
    DDL_SET_UPDATED_AT_FN,
    "CREATE OR REPLACE TRIGGER trg_plans_set_updated_at BEFORE UPDATE ON plans FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
    "CREATE OR REPLACE TRIGGER trg_phases_set_updated_at BEFORE UPDATE ON phases FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
    "CREATE OR REPLACE TRIGGER trg_projects_set_updated_at BEFORE UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
    "CREATE OR REPLACE TRIGGER trg_chapters_set_updated_at BEFORE UPDATE ON chapters FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
    "CREATE OR REPLACE TRIGGER trg_resources_set_updated_at BEFORE UPDATE ON resources FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
    "CREATE OR REPLACE TRIGGER trg_study_schedule_set_updated_at BEFORE UPDATE ON study_schedule FOR EACH ROW EXECUTE FUNCTION set_updated_at()",
];

/// 执行全部 DDL（逐条跑，任一失败即整体失败并归一为 Db 错误）。
/// 幂等语义：全部语句可重放，因此同一批（组件升级/网络重连重建池）首连多次执行安全。
pub(crate) async fn run(pool: &PgPool) -> Result<(), AgentError> {
    for (i, sql) in DDL_STATEMENTS
        .iter()
        .chain(COMMENT_STATEMENTS.iter())
        .enumerate()
    {
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
    /// （content_md 演进列在 chapters 建表之后；四条退役子表 DROP 连续且在
    /// 演进列之后、时间戳补列段之前；markdown 化后不再保留任何兜底 UPDATE）
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
        // Task E：study_days 列与 study_schedule 表都在 plans 建表之后
        let plans_table = find("CREATE TABLE IF NOT EXISTS plans");
        let study_days_col = find("ALTER TABLE plans ADD COLUMN IF NOT EXISTS study_days");
        let marks_table = find("CREATE TABLE IF NOT EXISTS study_schedule");
        assert!(
            plans_table < study_days_col,
            "study_days 演进列必须在 plans 建表之后"
        );
        assert!(
            plans_table < marks_table,
            "study_schedule 建表必须在 plans 建表之后（外键依赖）"
        );
        // 四张退役子表 DROP 恰好各一条、连续、都在 content_md 演进列之后，
        // 且在时间戳补列段之前（先收敛结构终态，再做时间戳/触发器/注释）
        for (needle, label) in [
            ("DROP TABLE IF EXISTS concepts", "concepts"),
            ("DROP TABLE IF EXISTS knowledge_points", "knowledge_points"),
            ("DROP TABLE IF EXISTS diagrams", "diagrams"),
            ("DROP TABLE IF EXISTS deep_dives", "deep_dives"),
        ] {
            let pos = find(needle);
            assert!(pos > chapters_md, "退役子表 DROP 应在演进列之后: {label}");
        }
        let deep_dives = find("DROP TABLE IF EXISTS deep_dives");
        let first_backfill =
            find("ALTER TABLE phases ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ");
        assert!(
            deep_dives < first_backfill,
            "退役子表 DROP 应在时间戳补列段之前"
        );
        assert!(
            !DDL_STATEMENTS
                .iter()
                .any(|s| s.trim_start().starts_with("UPDATE")),
            "DDL 序列不得包含数据 UPDATE 语句（updated_at 由触发器维护）"
        );
        assert!(
            !DDL_STATEMENTS
                .iter()
                .any(|s| s.contains("CREATE TABLE IF NOT EXISTS concepts")),
            "退役子表不得再出现在建表序列"
        );
        // 时间戳规范：五张子表的补列都在对应建表之后；触发器在补列之后
        for table in [
            "phases",
            "projects",
            "chapters",
            "resources",
            "study_schedule",
        ] {
            let created = find(&format!(
                "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS created_at"
            ));
            let updated = find(&format!(
                "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS updated_at"
            ));
            let created_in_table = find(&format!("CREATE TABLE IF NOT EXISTS {table} "));
            assert!(
                created_in_table < created && created < updated,
                "{table} 时间戳补列顺序错误"
            );
        }
        let fn_pos = find("CREATE OR REPLACE FUNCTION set_updated_at()");
        let trg_plans = find("CREATE OR REPLACE TRIGGER trg_plans_set_updated_at");
        let trg_marks = find("CREATE OR REPLACE TRIGGER trg_study_schedule_set_updated_at");
        assert!(
            fn_pos < trg_plans && trg_plans < trg_marks,
            "触发器语句顺序错误（函数先于触发器）"
        );
        let backfill_updated =
            find("ALTER TABLE study_schedule ADD COLUMN IF NOT EXISTS updated_at");
        assert!(
            backfill_updated < fn_pos,
            "触发器必须在 updated_at 补列之后"
        );
        // 序列末条恰为最后一个触发器 CREATE
        assert!(
            DDL_STATEMENTS[DDL_STATEMENTS.len() - 1]
                .contains("CREATE OR REPLACE TRIGGER trg_study_schedule_set_updated_at"),
            "序列末条应为 study_schedule 触发器"
        );
    }

    /// 注释规范：全部语句合法形态（COMMENT ON），覆盖六张表与全部时间戳列
    #[test]
    fn comments_cover_tables_and_timestamps() {
        assert!(
            COMMENT_STATEMENTS
                .iter()
                .all(|s| s.starts_with("COMMENT ON ")),
            "注释数组不得混入非 COMMENT 语句"
        );
        for table in [
            "plans",
            "phases",
            "projects",
            "chapters",
            "resources",
            "study_schedule",
        ] {
            assert!(
                COMMENT_STATEMENTS
                    .iter()
                    .any(|s| s.starts_with(&format!("COMMENT ON TABLE {table} IS "))),
                "缺少表注释: {table}"
            );
            for col in ["created_at", "updated_at"] {
                assert!(
                    COMMENT_STATEMENTS
                        .iter()
                        .any(|s| s.contains(&format!("COMMENT ON COLUMN {table}.{col} "))),
                    "缺少时间戳列注释: {table}.{col}"
                );
            }
        }
    }

    /// 逐条语句的方可执行性由集成测试在真库验证；本模块只校验无空白 SQL 混入
    #[test]
    fn statements_are_non_empty() {
        assert!(
            DDL_STATEMENTS
                .iter()
                .chain(COMMENT_STATEMENTS.iter())
                .all(|s| !s.trim().is_empty()),
            "DDL 数组不得混入空语句"
        );
    }
}
