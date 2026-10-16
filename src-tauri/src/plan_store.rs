//! 计划数据规范化多表存储：阶段级 upsert（每阶段完成即落库，章节整体替换）与多表重组读取。
//!
//! 与 db.rs 的分工：
//! - db.rs：连接池管理、首连建表（ensure_schema -> db_ddl）
//! - 本模块：流式生成链路（任务 10）的存储层——create_generating_plan /
//!   update_outline_meta / seed_outline / upsert_phase / upsert_chapter /
//!   mark_phase_pending / mark_chapter_pending / finalize_plan / finalize_failed /
//!   delete_plan_full，以及 get_plan 新路径的多表重组读取（load_study_plan +
//!   纯函数 reassemble_plan）与重生成上下文读取（load_plan_request_meta / load_plan_outline）
//!
//! 五表 DDL（plans/phases/projects/chapters/resources）与两个演进列
//! （status、chapters.content_md）逐字定义在 db_ddl，首连幂等执行。

use sqlx::{PgPool, Row, Transaction};
use std::collections::HashMap;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::{db_err, fail};
use crate::error::AgentError;
use crate::protocol::{
    ChapterDef, OutlinePhasePayload, PhaseProject, PlanRequest, StudyPhase, StudyPlan,
    StudyResource,
};

// ---------------------------------------------------------------------------
// 写入：生成链路（任务 10 流式入库）
// ---------------------------------------------------------------------------

/// 生成开始时预置 plans 行（id 由调用方生成）：status='generating'，
/// title 暂用 goal 截断（大纲就绪后由 update_outline_meta / finalize_plan 覆写）。
/// plan_json 保持 NULL（规范化新行不再写整串 JSON），overview 走列默认空串。
/// 消费方：commands::generate_plan（流式自动入库编排）。
pub async fn create_generating_plan(
    pool: &PgPool,
    plan_id: Uuid,
    request: &PlanRequest,
) -> Result<(), AgentError> {
    let title: String = request.goal.chars().take(200).collect();
    // studyDays 请求侧 None = 全周（serde 层转发 Python 时已归一，这里兑底同一语义）
    let study_days = request.study_days.clone().unwrap_or_else(vec_full_week);
    sqlx::query(
        "INSERT INTO plans (id, title, goal, level, weeks, weekly_hours, interests, background, study_days, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'generating')",
    )
    .bind(plan_id)
    .bind(&title)
    .bind(&request.goal)
    .bind(&request.level)
    .bind(i32::try_from(request.weeks).unwrap_or(0))
    .bind(i32::try_from(request.weekly_hours).unwrap_or(0))
    .bind(sqlx::types::Json(&request.interests))
    .bind(&request.background)
    .bind(sqlx::types::Json(&study_days))
    .execute(pool)
    .await
    .map_err(|e| fail("创建生成中计划", e))?;
    info!(%plan_id, "规范化计划行已创建（status=generating）");
    Ok(())
}

/// 全周学习日（周一..周日）：None 请求侧的归一终值，与 protocol::study_days_serde
/// 的序列化兑底同语义（两侧各自展开，由 wire 单测锁定一致）
fn vec_full_week() -> Vec<u8> {
    vec![1, 2, 3, 4, 5, 6, 7]
}

/// 更新计划的每周学习日（Task E 需求 5）：校验非空、取值 1..=7，
/// 去重升序后 UPDATE；计划不存在报错（不静默成功）。
/// 消费方：commands::set_plan_study_days（前端日抽屉「编辑学习日」）。
/// 更新计划的每周学习日（Task E 需求 5）：校验非空、取值 1..=7，去重升序
/// 后 UPDATE，返回归一化结果（前端直接采用，无需重拉）；
/// 校验失败 / 计划不存在归一为 AgentError::Db 中文文案。
pub async fn set_study_days(
    pool: &PgPool,
    plan_id: Uuid,
    days: &[u8],
) -> Result<Vec<u8>, AgentError> {
    if days.is_empty() {
        return Err(AgentError::Db("每周学习日不能为空（至少选择一天）".into()));
    }
    let mut normalized = days.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    if let Some(bad) = normalized.iter().find(|d| !(1u8..=7).contains(*d)) {
        return Err(AgentError::Db(format!(
            "每周学习日取值须为 1（周一）..7（周日），实际含 {bad}"
        )));
    }
    let updated = sqlx::query("UPDATE plans SET study_days = $2, updated_at = now() WHERE id = $1")
        .bind(plan_id)
        .bind(sqlx::types::Json(&normalized))
        .execute(pool)
        .await
        .map_err(|e| fail("更新每周学习日", e))?;
    if updated.rows_affected() == 0 {
        return Err(AgentError::Db(format!("计划不存在或已被删除：{plan_id}")));
    }
    info!(%plan_id, count = normalized.len(), "每周学习日已更新");
    Ok(normalized)
}

/// 当日已学打卡开关（Task E）：done → INSERT ON CONFLICT DO NOTHING；
/// undone → DELETE；两路均幂等（重复调用终态一致）。
/// 计划不存在时 INSERT 触发外键约束报错（打卡不凭空建行）。
/// 消费方：commands::set_study_day_mark（日抽屉「当日已学」勾选）。
pub async fn set_day_mark(
    pool: &PgPool,
    plan_id: Uuid,
    day: chrono::NaiveDate,
    done: bool,
) -> Result<(), AgentError> {
    if done {
        sqlx::query(
            "INSERT INTO study_day_marks (plan_id, day) VALUES ($1, $2)
             ON CONFLICT (plan_id, day) DO NOTHING",
        )
        .bind(plan_id)
        .bind(day)
        .execute(pool)
        .await
        .map_err(|e| fail("写入当日已学打卡", e))?;
    } else {
        sqlx::query("DELETE FROM study_day_marks WHERE plan_id = $1 AND day = $2")
            .bind(plan_id)
            .bind(day)
            .execute(pool)
            .await
            .map_err(|e| fail("撤销当日已学打卡", e))?;
    }
    info!(%plan_id, %day, done, "当日已学打卡已更新");
    Ok(())
}

/// 读取计划的全部已打卡日（Task E）：get_plan 拼装 dayMarks wire 用；
/// 无打卡行返回空 map。
pub async fn load_plan_marks(
    pool: &PgPool,
    plan_id: Uuid,
) -> Result<HashMap<chrono::NaiveDate, ()>, AgentError> {
    let rows = sqlx::query("SELECT day FROM study_day_marks WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("查询当日已学打卡", e))?;
    let mut marks = HashMap::with_capacity(rows.len());
    for row in rows {
        marks.insert(row.try_get("day").map_err(db_err)?, ());
    }
    Ok(marks)
}

/// 大纲就绪：覆写计划标题、概述与预计总学时（title 暂为 goal 截断的占位时由
/// 本函数替换）。overview 修复（任务 22）：此前漏写 plans.overview 列，前端
/// 历史条目/详情概述为空——有值必写；Python 端缺省产出 Some("")，空串视为
/// 缺失照写空串（列 NOT NULL DEFAULT ''，NULL 不可写）。
/// 消费方：commands generate_plan 后台消费（outline 事件到达时）。
pub async fn update_outline_meta(
    pool: &PgPool,
    plan_id: Uuid,
    title: &str,
    overview: Option<&str>,
    estimated_total_hours: i64,
) -> Result<(), AgentError> {
    sqlx::query(
        "UPDATE plans SET title = $2, overview = $3, estimated_total_hours = $4, updated_at = now()
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(title)
    .bind(overview.unwrap_or(""))
    .bind(i32::try_from(estimated_total_hours).unwrap_or(0))
    .execute(pool)
    .await
    .map_err(|e| fail("更新计划大纲元信息", e))?;
    info!(%plan_id, "计划大纲元信息已更新");
    Ok(())
}

/// 阶段级 upsert（单事务）：写入/更新 (plan_id, idx) 的 phases 行后，整体替换其
/// chapters（DELETE 后重插，旧章节行不残留）与 project。
/// 同一阶段回炉重写时旧数据不残留；事务保证半途失败不留脏数据。
/// 阶段行 status 与章节定价联动（与 upsert_chapter 的 NOT EXISTS 翻牌语义对称）：
/// 仅当存在章节且每章正文非空时置 'complete'，否则保持/回落 'pending'
/// （正文空的阶段保持可重生成入口）；章节行 status 由内容定价（见
/// chapter_row_status）：正文空仍落 failed，前端可重生成。
/// 消费方：commands::generate_plan（phase 事件到达时）/ regenerate_phase（任务 15）。
pub async fn upsert_phase(
    pool: &PgPool,
    plan_id: Uuid,
    idx: i32,
    phase: &StudyPhase,
) -> Result<(), AgentError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启阶段写入事务", e))?;

    // 0. 阶段状态由本章正文定价派生：有章节且每章正文非空才 complete
    let phase_status = if !phase.chapters.is_empty()
        && phase
            .chapters
            .iter()
            .all(|c| !c.content_md.trim().is_empty())
    {
        "complete"
    } else {
        "pending"
    };

    // 1. phases 行：冲突即更新（UNIQUE (plan_id, idx)），RETURNING 拿物理行 id；
    //    status 用绑定参数透传（INSERT 与 DO UPDATE 双路同值，骨架 pending 行在此翻牌）
    let phase_id: Uuid = sqlx::query_scalar(
        "INSERT INTO phases (plan_id, idx, name, week_range, objectives, practice, assignments, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (plan_id, idx) DO UPDATE SET
             name = EXCLUDED.name,
             week_range = EXCLUDED.week_range,
             objectives = EXCLUDED.objectives,
             practice = EXCLUDED.practice,
             assignments = EXCLUDED.assignments,
             status = EXCLUDED.status
         RETURNING id",
    )
    .bind(plan_id)
    .bind(idx)
    .bind(&phase.name)
    .bind(&phase.week_range)
    .bind(sqlx::types::Json(&phase.objectives))
    .bind(sqlx::types::Json(&phase.practice))
    .bind(sqlx::types::Json(&phase.assignments))
    .bind(phase_status)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| fail("写入阶段", e))?;

    // 2. 整体替换子数据：先清 project 与 chapters（旧章节行一并不残留）
    sqlx::query("DELETE FROM projects WHERE phase_id = $1")
        .bind(phase_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("清理旧项目", e))?;
    sqlx::query("DELETE FROM chapters WHERE phase_id = $1")
        .bind(phase_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("清理旧章节", e))?;

    // 3. 重插 chapters（idx 即数组下标，读取按 idx 升序还原顺序；纯单行写入，
    //    status 由 content_md 定价——正文为空仍可入库翻 failed，前端可重生成）
    for (i, chapter) in phase.chapters.iter().enumerate() {
        insert_chapter_row(&mut tx, phase_id, i as i64, chapter).await?;
    }

    // 项目（若存在）：StudyPhase.project 非 Option（协议兼容旧 JSON），
    // 以 name 非空视为「有项目」；空项目不落行
    if !phase.project.name.trim().is_empty() {
        sqlx::query(
            "INSERT INTO projects (phase_id, name, description, background, steps, acceptance_criteria, estimated_hours)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(phase_id)
        .bind(&phase.project.name)
        .bind(&phase.project.description)
        .bind(&phase.project.background)
        .bind(sqlx::types::Json(&phase.project.steps))
        .bind(sqlx::types::Json(&phase.project.acceptance_criteria))
        .bind(i32::try_from(phase.project.estimated_hours).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("写入阶段项目", e))?;
    }

    tx.commit().await.map_err(|e| fail("提交阶段事务", e))?;
    info!(%plan_id, idx, "阶段已 upsert 到规范化多表");
    Ok(())
}

/// 大纲就绪后播种骨架（单事务）：phases(status='pending') + chapters（标题骨架，
/// status='pending'）+ resources；不播种 project（大纲阶段只有 project_brief，
/// 无法填充完整结构，project 留待 upsert_phase 整体写入）。
/// 幂等守卫：仅当该计划 phases 表为空时才播种（防 regenerate 与竞态覆盖已有完成
/// 数据）；regenerate 路径绝不触发 seed（守卫亦兜底）。
/// 消费方：commands generate_plan outline 事件接线。
pub async fn seed_outline(
    pool: &PgPool,
    plan_id: Uuid,
    phases: &[OutlinePhasePayload],
    resources: &[StudyResource],
) -> Result<(), AgentError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启大纲播种事务", e))?;

    // 守卫：该计划已有阶段行则跳过（regenerate / 竞态下不覆盖已有数据）
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM phases WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| fail("检查已有阶段", e))?;
    if existing > 0 {
        info!(%plan_id, "计划已有阶段数据，跳过大纲播种（幂等守卫）");
        return Ok(());
    }

    for (i, outline) in phases.iter().enumerate() {
        let phase_id: Uuid = sqlx::query_scalar(
            "INSERT INTO phases (plan_id, idx, name, week_range, status)
             VALUES ($1, $2, $3, $4, 'pending') RETURNING id",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&outline.name)
        .bind(&outline.week_range)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| fail("播种阶段骨架", e))?;

        for (j, title) in outline.chapter_titles.iter().enumerate() {
            sqlx::query(
                "INSERT INTO chapters (phase_id, idx, title, summary, status)
                 VALUES ($1, $2, $3, '', 'pending')",
            )
            .bind(phase_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(title)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("播种章节骨架", e))?;
        }
    }

    for (i, resource) in resources.iter().enumerate() {
        sqlx::query(
            "INSERT INTO resources (plan_id, idx, title, type, note) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&resource.title)
        .bind(&resource.kind)
        .bind(&resource.note)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("播种学习资源", e))?;
    }

    tx.commit().await.map_err(|e| fail("提交大纲播种事务", e))?;
    info!(%plan_id, phase_count = phases.len(), "大纲骨架已播种（status=pending）");
    Ok(())
}

/// done 终态补写推荐资源（单事务）：DELETE 后批量 INSERT（DELETE+INSERT 幂等
/// 覆写，重发 done 不重复；同一事务保证半途失败不留脏数据）。
/// 消费方：GenerationConsumer done 分支（任务 23 审查 Finding-1：结果页唯一
/// 内容源是回读 DB，done 包络携带的 resources 不落库则资源卡永久缺失）。
/// 空数组不调用（调用方以 plan.resources 非空为前提，避免覆写清空既有序源）。
/// plan_store::tests 的真库集成测试直接调取验证幂等/回读。
pub async fn upsert_resources(
    pool: &PgPool,
    plan_id: Uuid,
    resources: &[StudyResource],
) -> Result<(), AgentError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启资源写入事务", e))?;

    // 整体替换：旧资源行不残留（与 upsert_phase 的子数据替换语义同源）
    sqlx::query("DELETE FROM resources WHERE plan_id = $1")
        .bind(plan_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("清理旧资源", e))?;

    for (i, resource) in resources.iter().enumerate() {
        sqlx::query(
            "INSERT INTO resources (plan_id, idx, title, type, note) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&resource.title)
        .bind(&resource.kind)
        .bind(&resource.note)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("写入学习资源", e))?;
    }

    tx.commit().await.map_err(|e| fail("提交资源事务", e))?;
    info!(%plan_id, count = resources.len(), "done 包络推荐资源已落库");
    Ok(())
}

/// 单章 upsert（单事务）：骨架行的内容填充 / 章节重生成入库。
/// - phases 行必须已存在（seed_outline 或既有数据），否则报错（单章不凭空建阶段）；
/// - chapters ON CONFLICT (phase_id, idx) 更新 title/summary/content_md，status
///   由 content_md 定价（正文空 → failed，不推进阶段）；
/// - 尾部阶段联动：该阶段不再存在 status<>'complete' 的章节时，同事务内把阶段
///   置 complete（状态机推进与数据写入原子）——正文为空的章节翻 failed 行，
///   阶段因此保持不置 complete，留给重新生成收口。
///
/// 消费方：regenerate_chapter（任务 15）。
pub async fn upsert_chapter(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
    chapter: &ChapterDef,
) -> Result<(), AgentError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启单章写入事务", e))?;

    let phase_id: Uuid =
        sqlx::query_scalar("SELECT id FROM phases WHERE plan_id = $1 AND idx = $2")
            .bind(plan_id)
            .bind(i32::try_from(phase_index).unwrap_or(0))
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| fail("定位阶段行", e))?
            .ok_or_else(|| {
                AgentError::Db(format!(
                    "阶段不存在，无法写入章节：plan={plan_id} phaseIndex={phase_index}"
                ))
            })?;

    // 单章写入（章节行冲突即更新，status 由 content_md 定价）
    insert_chapter_row(&mut tx, phase_id, chapter_index as i64, chapter).await?;

    // 阶段联动（同一事务原子）：全部章节 complete 时把阶段推到 complete
    sqlx::query(
        "UPDATE phases SET status = 'complete'
         WHERE id = $1 AND NOT EXISTS (
             SELECT 1 FROM chapters c WHERE c.phase_id = phases.id AND c.status <> 'complete'
         )",
    )
    .bind(phase_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("推进阶段状态", e))?;

    tx.commit().await.map_err(|e| fail("提交单章事务", e))?;
    info!(%plan_id, phase_index, chapter_index, "单章已 upsert（status=complete）");
    Ok(())
}

/// 阶段重置为 pending（regenerate_phase 开始时调用，前端即时转「生成中」）；
/// 阶段内容（chapters）不动，重生成成功后由 upsert_phase 整体替换。
/// 消费方：regenerate_phase（任务 15）。
pub async fn mark_phase_pending(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
) -> Result<(), AgentError> {
    sqlx::query("UPDATE phases SET status = 'pending' WHERE plan_id = $1 AND idx = $2")
        .bind(plan_id)
        .bind(i32::try_from(phase_index).unwrap_or(0))
        .execute(pool)
        .await
        .map_err(|e| fail("重置阶段状态", e))?;
    info!(%plan_id, phase_index, "阶段已置回 pending");
    Ok(())
}

/// 章节重置为 pending（regenerate_chapter 开始时调用）；注意同时把所属阶段回置
/// pending——单章重生成会让阶段显示「生成中」，完成后由 upsert_chapter 阶段联动翻牌。
/// 消费方：regenerate_chapter（任务 15）。
pub async fn mark_chapter_pending(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<(), AgentError> {
    let phase_i32 = i32::try_from(phase_index).unwrap_or(0);
    let chapter_i32 = i32::try_from(chapter_index).unwrap_or(0);
    sqlx::query(
        "UPDATE chapters c SET status = 'pending'
         FROM phases p WHERE c.phase_id = p.id AND p.plan_id = $1 AND p.idx = $2 AND c.idx = $3",
    )
    .bind(plan_id)
    .bind(phase_i32)
    .bind(chapter_i32)
    .execute(pool)
    .await
    .map_err(|e| fail("重置章节状态", e))?;
    sqlx::query("UPDATE phases SET status = 'pending' WHERE plan_id = $1 AND idx = $2")
        .bind(plan_id)
        .bind(phase_i32)
        .execute(pool)
        .await
        .map_err(|e| fail("重置所属阶段状态", e))?;
    info!(%plan_id, phase_index, chapter_index, "章节及所属阶段已置回 pending");
    Ok(())
}

/// 全章脈完成判定（2026-10-16 用户定案，终态收尾唯一口径）：总计划在完成
/// 时校验所有章脈状态——存在至少一章 且 全部章脈 complete → true（总状态
/// complete）；存在任一失败章/未完成的滞留章（骨架 pending）或零章行
/// → false（总状态 failed，finalize_failed 将 pending 子行翻 failed）。
/// 骨架行均为 pending，不充当内容证据；章三态 pending（生成中）/
/// complete（生成成功）/ failed（生成失败）。消费方：commands 终态收尾
/// （done 终态与断线耗尽收尾）。
pub async fn all_chapters_complete(pool: &PgPool, plan_id: Uuid) -> Result<bool, AgentError> {
    let all_done: bool = sqlx::query_scalar(
        "SELECT EXISTS( SELECT 1 FROM chapters c JOIN phases p ON c.phase_id = p.id
                        WHERE p.plan_id = $1 )
             AND NOT EXISTS(
                 SELECT 1 FROM chapters c2 JOIN phases p2 ON c2.phase_id = p2.id
                 WHERE p2.plan_id = $1 AND c2.status <> 'complete' )",
    )
    .bind(plan_id)
    .fetch_one(pool)
    .await
    .map_err(|e| fail("查询全章节完成状态", e))?;
    Ok(all_done)
}

/// 启动清扫（任务 22）：sidecar / 应用重启后，后台生成任务随 Python 进程消失，
/// status='generating' 的存量行永远等不到终态事件 → 全部清为 failed（仅 DB
/// 模式；重复执行恒为空集，幂等）；其 pending 骨架子行一并置 failed，
/// complete 子行不动（finalize_failed 的状态机不变量）。
/// 桌面单实例语义假定：若双实例并开，清扫可能把存活实例进行中的 generating
/// 行暂翻 failed——该实例 done 收尾会终态覆写自愈，仅列表页短暂显示 failed。
/// 返回清扫的行数（供日志）。消费方：main.rs setup 钩子。
pub async fn sweep_stale_generating(pool: &PgPool) -> Result<u64, AgentError> {
    let stale: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE plans SET status = 'failed', updated_at = now()
         WHERE status = 'generating' RETURNING id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| fail("清扫存量生成中计划", e))?;
    for plan_id in &stale {
        if let Err(e) = finalize_failed(pool, *plan_id).await {
            warn!(%plan_id, error = %e, "清扫存量计划的子行收尾失败（主行已置 failed）");
        }
    }
    let count = stale.len() as u64;
    if count > 0 {
        info!(count, "启动清扫：存量 generating 计划行已置 failed");
    }
    Ok(count)
}

/// 失败收尾（children 位）：phases/chapters 中 status='pending' 的行置 failed；
/// complete 永不被 failed 覆盖（状态机不变量：失败收尾只动 pending）。
/// 消费方：finalize_plan(status=failed) / regenerate 失败路径（任务 15）。
pub async fn finalize_failed(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    sqlx::query("UPDATE phases SET status = 'failed' WHERE plan_id = $1 AND status = 'pending'")
        .bind(plan_id)
        .execute(pool)
        .await
        .map_err(|e| fail("收尾失败阶段状态", e))?;
    sqlx::query(
        "UPDATE chapters c SET status = 'failed' FROM phases p
         WHERE c.phase_id = p.id AND p.plan_id = $1 AND c.status = 'pending'",
    )
    .bind(plan_id)
    .execute(pool)
    .await
    .map_err(|e| fail("收尾失败章节状态", e))?;
    info!(%plan_id, "pending 阶段/章节已置 failed（complete 不受影响）");
    Ok(())
}

/// 生成收尾：覆写 status（complete | failed）；title / estimated_total_hours
/// 为 Some 时一并更新，None 保持原值。
/// status='failed' 时同调 finalize_failed：pending 阶段/章节一并置 failed
/// （complete 不动），保证状态机不留滞留 pending 的行。
/// 消费方：commands::generate_plan（done 终态与异常路径收尾）/ regenerate（任务 15）。
pub async fn finalize_plan(
    pool: &PgPool,
    plan_id: Uuid,
    status: &str,
    title: Option<&str>,
    estimated_total_hours: Option<i64>,
) -> Result<(), AgentError> {
    sqlx::query(
        "UPDATE plans SET
             status = $2,
             title = COALESCE($3, title),
             estimated_total_hours = COALESCE($4, estimated_total_hours),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(status)
    .bind(title)
    .bind(estimated_total_hours.map(|h| i32::try_from(h).unwrap_or(0)))
    .execute(pool)
    .await
    .map_err(|e| fail("收尾计划状态", e))?;
    if status == "failed" {
        finalize_failed(pool, plan_id).await?;
    } else if status != "complete" {
        warn!(%plan_id, status, "未知计划收尾状态，children 位不流转");
    }
    info!(%plan_id, status, "计划状态已收尾");
    Ok(())
}

/// regenerate 成功收尾（任务 15 裁定的守卫语义）：done(plan) 后把计划行翻
/// complete，但 failed 行不强推——单阶段/单章成功不改变计划级 complete 语义，
/// 计划级收尾由后续全阶段/全章节重生成推进（已知简化，前端不依赖计划行状态展示）。
/// generating 行（重生成期不应出现，防御性）与 complete 行（幂等重写）同样翻 complete。
/// 消费方：commands regenerate 双命令的 done(plan) 路径（任务 15）。
pub async fn finalize_regen_complete(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    sqlx::query(
        "UPDATE plans SET status = 'complete', updated_at = now()
         WHERE id = $1 AND status <> 'failed'",
    )
    .bind(plan_id)
    .execute(pool)
    .await
    .map_err(|e| fail("收尾重生成计划状态", e))?;
    info!(%plan_id, "重生成计划行已按守卫语义收尾（failed 行保持）");
    Ok(())
}

/// 删除整份计划（硬删除）：子表全部经外键 ON DELETE CASCADE 级联清空；
/// id 不存在视为已删除，幂等成功。
/// 消费方：commands::generate_plan（clarification 终态删 generating 行）。
pub async fn delete_plan_full(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(plan_id)
        .execute(pool)
        .await
        .map_err(|e| fail("删除计划", e))?;
    info!(%plan_id, "学习计划及子数据已删除");
    Ok(())
}

// ---------------------------------------------------------------------------
// 重生成上下文读取（任务 13 新增；任务 15 的 regenerate 编排消费）
// ---------------------------------------------------------------------------

/// 重建 PlanRequest 所需的轻量行结构：与 protocol::PlanRequest 逐字段对应，
/// 但由本模块自持（字段拼接回 PlanRequest 由任务 15 做，本任务只提供元数据读取）；
/// study_days（Task E）：None = 全周（旧行缺省语义）
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRequestRow {
    pub goal: String,
    pub level: String,
    pub weeks: u32,
    pub weekly_hours: u32,
    pub interests: Vec<String>,
    pub background: String,
    pub study_days: Option<Vec<u8>>,
}

/// 重建 PlanOutline 所需的轻量行结构：plans 元信息 + 阶段骨架
/// （phases 复用 protocol::OutlinePhasePayload：name/weekRange/chapterTitles）
#[derive(Debug, Clone, PartialEq)]
pub struct PlanOutlineRow {
    pub title: String,
    pub overview: String,
    pub estimated_total_hours: u32,
    pub phases: Vec<OutlinePhasePayload>,
}

/// 读取重建 PlanRequest 所需的七字段（goal/level/weeks/weeklyHours/interests/
/// background 六字段本体，title 由 Plans 行同表可读不入本结构）；
/// 计划不存在时返回 None。消费方：regenerate 编排（任务 15）。
pub async fn load_plan_request_meta(
    pool: &PgPool,
    plan_id: Uuid,
) -> Result<Option<PlanRequestRow>, AgentError> {
    let row = sqlx::query(
        "SELECT goal, level, weeks, weekly_hours, interests, background, study_days FROM plans WHERE id = $1",
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询计划上下文", e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
    let weekly_hours: i32 = row.try_get("weekly_hours").map_err(db_err)?;
    let interests: sqlx::types::Json<Vec<String>> = row.try_get("interests").map_err(db_err)?;
    // 旧行列默认全周 JSONB 数组（DDL DEFAULT '[1..7]'），读回恒为合法数组；
    // 空数组不兑底（校验层已拒绝空学习日）
    let study_days: sqlx::types::Json<Vec<u8>> = row.try_get("study_days").map_err(db_err)?;
    Ok(Some(PlanRequestRow {
        goal: row.try_get("goal").map_err(db_err)?,
        level: row.try_get("level").map_err(db_err)?,
        weeks: weeks.max(0) as u32,
        weekly_hours: weekly_hours.max(0) as u32,
        interests: interests.0,
        background: row.try_get("background").map_err(db_err)?,
        study_days: Some(study_days.0),
    }))
}

/// 读取计划大纲骨架（PlanOutline 形）：plans 元信息与 phases（name/weekRange）
/// 及各阶段 chapters 标题列表（按 idx 升序）。计划不存在时返回 None。
/// objectives / project_brief 不在骨架：大纲事件也不携带——重生成裁定，
/// 全阶段重生成自产 objectives、projectBrief 传空；单章重生成的简介另读 project 行。
///
/// 消费方：regenerate 编排（任务 15）。
pub async fn load_plan_outline(
    pool: &PgPool,
    plan_id: Uuid,
) -> Result<Option<PlanOutlineRow>, AgentError> {
    let row = sqlx::query("SELECT title, overview, estimated_total_hours FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| fail("查询计划大纲元信息", e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let hours: i32 = row.try_get("estimated_total_hours").map_err(db_err)?;

    let phase_rows =
        sqlx::query("SELECT id, name, week_range FROM phases WHERE plan_id = $1 ORDER BY idx ASC")
            .bind(plan_id)
            .fetch_all(pool)
            .await
            .map_err(|e| fail("查询大纲阶段列表", e))?;

    let mut phases = Vec::with_capacity(phase_rows.len());
    for prow in phase_rows {
        let phase_id: Uuid = prow.try_get("id").map_err(db_err)?;
        let titles: Vec<String> =
            sqlx::query("SELECT title FROM chapters WHERE phase_id = $1 ORDER BY idx ASC")
                .bind(phase_id)
                .fetch_all(pool)
                .await
                .map_err(|e| fail("查询大纲章节标题", e))?
                .into_iter()
                .map(|c| c.try_get::<String, _>("title").map_err(db_err))
                .collect::<Result<_, _>>()?;
        phases.push(OutlinePhasePayload {
            name: prow.try_get("name").map_err(db_err)?,
            week_range: prow.try_get("week_range").map_err(db_err)?,
            chapter_titles: titles,
            project_brief: None,
        });
    }

    Ok(Some(PlanOutlineRow {
        title: row.try_get("title").map_err(db_err)?,
        overview: row.try_get("overview").map_err(db_err)?,
        estimated_total_hours: hours.max(0) as u32,
        phases,
    }))
}

/// 读取目标阶段的既有项目名（单章重生成的 projectBrief 上下文：任务 15 裁定
/// ——读 project 行 name 充当简介，若有；无项目行 / 阶段不存在返回 None）。
/// 消费方：regenerate_chapter（任务 15）。
pub async fn load_phase_project_brief(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
) -> Result<Option<String>, AgentError> {
    sqlx::query_scalar(
        "SELECT pr.name FROM projects pr
         JOIN phases p ON pr.phase_id = p.id
         WHERE p.plan_id = $1 AND p.idx = $2",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询阶段项目简介", e))
}

// ---------------------------------------------------------------------------
// 共享写入：章节单行（upsert_phase / upsert_chapter 复用，防双份漂移）
// ---------------------------------------------------------------------------

/// 章节行 status 由内容定价：正文 strip 后非空 → complete，空 → failed。
/// 空内容翻 failed（而非 complete）是重生成入口的判定链发出点：前端按
/// failed/pending 态露出「重新生成此章」按钮。INSERT / DO UPDATE 双路共用。
fn chapter_row_status(content_md: &str) -> &'static str {
    if content_md.trim().is_empty() {
        "failed"
    } else {
        "complete"
    }
}

/// 写入单个章节行（冲突即更新 title/summary/content_md/status）。
/// 调用方须在事务内执行；返传物理章节行 id。upsert_phase（整阶段替换重插）与
/// upsert_chapter（单章骨架填充/重生成）共用同一套写入语义，避免两份实现漂移。
async fn insert_chapter_row(
    tx: &mut Transaction<'static, sqlx::Postgres>,
    phase_id: Uuid,
    idx: i64,
    chapter: &ChapterDef,
) -> Result<Uuid, AgentError> {
    let chapter_id: Uuid = sqlx::query_scalar(
        "INSERT INTO chapters (phase_id, idx, title, summary, content_md, status)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (phase_id, idx) DO UPDATE SET
             title = EXCLUDED.title,
             summary = EXCLUDED.summary,
             content_md = EXCLUDED.content_md,
             status = EXCLUDED.status
         RETURNING id",
    )
    .bind(phase_id)
    .bind(i32::try_from(idx).unwrap_or(0))
    .bind(&chapter.title)
    .bind(&chapter.summary)
    .bind(&chapter.content_md)
    .bind(chapter_row_status(&chapter.content_md))
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| fail("写入章节", e))?;
    Ok(chapter_id)
}

// ---------------------------------------------------------------------------
// 读取：get_plan 新路径（多表重组）
// ---------------------------------------------------------------------------

/// 多表行数据的内存形态（供 reassemble_plan 纯函数组装；
/// 顺序由各查询 ORDER BY idx 保证，结构体不再携带 idx）
struct PlanRows {
    title: String,
    overview: String,
    estimated_total_hours: i32,
    resources: Vec<StudyResource>,
    phases: Vec<PhaseRows>,
}

struct PhaseRows {
    name: String,
    week_range: String,
    status: Option<String>,
    objectives: Vec<String>,
    practice: Vec<String>,
    assignments: Vec<String>,
    project: Option<ProjectRow>,
    chapters: Vec<ChapterRows>,
}

struct ChapterRows {
    title: String,
    summary: String,
    status: Option<String>,
    /// markdown 正文全文：NOT NULL DEFAULT ''，空串也是有效返回值（恒 Some）
    content_md: String,
}

struct ProjectRow {
    name: String,
    description: String,
    background: String,
    steps: Vec<String>,
    acceptance_criteria: Vec<String>,
    estimated_hours: i32,
}

/// get_plan 新路径：多表查询按 idx 升序组装 StudyPlan。
/// plans 行本身（title/overview/estimated_total_hours）由调用方读取后传入，
/// 避免重复查询；status=generating/failed 的部分数据同样返回（phases 可能为空列表）。
pub async fn load_study_plan(
    pool: &PgPool,
    plan_id: Uuid,
    title: &str,
    overview: &str,
    estimated_total_hours: i32,
) -> Result<StudyPlan, AgentError> {
    let phase_rows = sqlx::query(
        "SELECT id, name, week_range, status, objectives, practice, assignments
         FROM phases WHERE plan_id = $1 ORDER BY idx ASC",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询阶段列表", e))?;

    let mut phases = Vec::with_capacity(phase_rows.len());
    for row in phase_rows {
        let phase_id: Uuid = row.try_get("id").map_err(db_err)?;
        let objectives: sqlx::types::Json<Vec<String>> =
            row.try_get("objectives").map_err(db_err)?;
        let practice: sqlx::types::Json<Vec<String>> = row.try_get("practice").map_err(db_err)?;
        let assignments: sqlx::types::Json<Vec<String>> =
            row.try_get("assignments").map_err(db_err)?;

        let chapter_rows = sqlx::query(
            "SELECT title, summary, status, content_md FROM chapters WHERE phase_id = $1 ORDER BY idx ASC",
        )
        .bind(phase_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("查询章节列表", e))?;

        let mut chapters = Vec::with_capacity(chapter_rows.len());
        for crow in chapter_rows {
            chapters.push(ChapterRows {
                title: crow.try_get("title").map_err(db_err)?,
                summary: crow.try_get("summary").map_err(db_err)?,
                status: crow.try_get("status").map_err(db_err)?,
                content_md: crow.try_get("content_md").map_err(db_err)?,
            });
        }

        // 项目（projects.phase_id UNIQUE，至多一行）
        let project = sqlx::query(
            "SELECT name, description, background, steps, acceptance_criteria, estimated_hours
             FROM projects WHERE phase_id = $1",
        )
        .bind(phase_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| fail("查询阶段项目", e))?
        .map(|p| -> Result<ProjectRow, AgentError> {
            Ok(ProjectRow {
                name: p.try_get("name").map_err(db_err)?,
                description: p.try_get("description").map_err(db_err)?,
                background: p.try_get("background").map_err(db_err)?,
                steps: p
                    .try_get::<sqlx::types::Json<Vec<String>>, _>("steps")
                    .map_err(db_err)?
                    .0,
                acceptance_criteria: p
                    .try_get::<sqlx::types::Json<Vec<String>>, _>("acceptance_criteria")
                    .map_err(db_err)?
                    .0,
                estimated_hours: p.try_get("estimated_hours").map_err(db_err)?,
            })
        })
        .transpose()?;

        phases.push(PhaseRows {
            name: row.try_get("name").map_err(db_err)?,
            week_range: row.try_get("week_range").map_err(db_err)?,
            status: row.try_get("status").map_err(db_err)?,
            objectives: objectives.0,
            practice: practice.0,
            assignments: assignments.0,
            project,
            chapters,
        });
    }

    let mut resources = Vec::new();
    for r in
        sqlx::query("SELECT title, type, note FROM resources WHERE plan_id = $1 ORDER BY idx ASC")
            .bind(plan_id)
            .fetch_all(pool)
            .await
            .map_err(|e| fail("查询资源列表", e))?
    {
        resources.push(StudyResource {
            title: r.try_get("title").map_err(db_err)?,
            kind: r.try_get("type").map_err(db_err)?,
            note: r.try_get("note").map_err(db_err)?,
        });
    }

    Ok(reassemble_plan(PlanRows {
        title: title.to_string(),
        overview: overview.to_string(),
        estimated_total_hours,
        resources,
        phases,
    }))
}

// ---------------------------------------------------------------------------
// 纯函数组装（无 IO，便于单元测试）
// ---------------------------------------------------------------------------

/// 行数据 -> StudyPlan：按传入顺序组装（DB 侧已按 idx 升序取出）。
/// theory / review_notes 为旧协议兼容字段，规范化数据恒为空。
fn reassemble_plan(rows: PlanRows) -> StudyPlan {
    StudyPlan {
        title: rows.title,
        overview: rows.overview,
        estimated_total_hours: rows.estimated_total_hours.max(0) as u32,
        phases: rows.phases.into_iter().map(reassemble_phase).collect(),
        resources: rows.resources,
        review_notes: String::new(),
    }
}

fn reassemble_phase(row: PhaseRows) -> StudyPhase {
    StudyPhase {
        name: row.name,
        week_range: row.week_range,
        objectives: row.objectives,
        chapters: row.chapters.into_iter().map(reassemble_chapter).collect(),
        theory: Vec::new(),
        practice: row.practice,
        assignments: row.assignments,
        project: row.project.map(reassemble_project).unwrap_or_default(),
        status: row.status,
    }
}

fn reassemble_chapter(row: ChapterRows) -> ChapterDef {
    ChapterDef {
        title: row.title,
        summary: row.summary,
        content_md: row.content_md,
        status: row.status,
    }
}

fn reassemble_project(row: ProjectRow) -> PhaseProject {
    PhaseProject {
        name: row.name,
        description: row.description,
        background: row.background,
        steps: row.steps,
        estimated_hours: row.estimated_hours.max(0) as u32,
        acceptance_criteria: row.acceptance_criteria,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 纯函数组装：保持传入顺序（排序责任在 SQL）、
    /// theory/review_notes 恒空、无项目行时 project 为 Default
    #[test]
    fn reassemble_keeps_row_order_and_defaults() {
        let chapter = |title: &str| ChapterRows {
            title: title.into(),
            summary: format!("{title}摘要"),
            status: Some("complete".into()),
            content_md: format!("## {title}\n\n```mermaid\nflowchart LR\n```"),
        };
        let rows = PlanRows {
            title: "测试计划".into(),
            overview: "概览".into(),
            estimated_total_hours: 88,
            resources: vec![StudyResource {
                title: "The Rust Book".into(),
                kind: "docs".into(),
                note: "官方教程".into(),
            }],
            phases: vec![
                PhaseRows {
                    name: "阶段甲".into(),
                    week_range: "第 1-2 周".into(),
                    status: Some("complete".into()),
                    objectives: vec!["目标甲".into()],
                    practice: vec!["实操甲".into()],
                    assignments: vec!["作业甲".into()],
                    project: Some(ProjectRow {
                        name: "项目甲".into(),
                        description: "描述".into(),
                        background: "背景".into(),
                        steps: vec!["步骤甲".into()],
                        acceptance_criteria: vec!["验收甲".into()],
                        estimated_hours: 6,
                    }),
                    chapters: vec![chapter("甲章")],
                },
                PhaseRows {
                    name: "阶段乙".into(),
                    week_range: "第 3-4 周".into(),
                    status: Some("complete".into()),
                    objectives: vec![],
                    practice: vec![],
                    assignments: vec![],
                    project: None,
                    chapters: vec![],
                },
            ],
        };

        let plan = reassemble_plan(rows);
        assert_eq!(plan.title, "测试计划");
        assert_eq!(plan.estimated_total_hours, 88);
        assert_eq!(plan.resources.len(), 1);
        assert_eq!(plan.resources[0].kind, "docs");
        assert_eq!(
            plan.phases
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["阶段甲", "阶段乙"],
            "组装保持行数据顺序（idx 排序在 SQL 完成）"
        );
        let first = &plan.phases[0];
        assert_eq!(first.theory, Vec::<String>::new(), "规范化数据 theory 恒空");
        assert_eq!(
            first.status.as_deref(),
            Some("complete"),
            "status 装入协议位"
        );
        assert_eq!(
            first.chapters[0].content_md,
            format!("## 甲章\n\n```mermaid\nflowchart LR\n```"),
            "content_md 等值读回"
        );
        // 空 content_md 也是有效组装（DEFAULT '' 行恒回正文）
        let empty_phases = [PhaseRows {
            name: "阶段丙".into(),
            week_range: "".into(),
            status: Some("pending".into()),
            objectives: vec![],
            practice: vec![],
            assignments: vec![],
            project: None,
            chapters: vec![ChapterRows {
                title: "骨架章".into(),
                summary: String::new(),
                status: Some("pending".into()),
                content_md: String::new(),
            }],
        }];
        let empty_plan = reassemble_plan(PlanRows {
            title: "空".into(),
            overview: String::new(),
            estimated_total_hours: 0,
            resources: vec![],
            phases: empty_phases.into_iter().collect(),
        });
        assert_eq!(
            empty_plan.phases[0].chapters[0].content_md, "",
            "DEFAULT '' 空正文恒回 Some 语义"
        );
        assert_eq!(first.project.name, "项目甲");
        assert_eq!(first.project.estimated_hours, 6);
        // 无项目行 -> 协议默认值（name 空串）
        assert_eq!(plan.phases[1].project, PhaseProject::default());
        assert_eq!(plan.review_notes, "");
    }
}
