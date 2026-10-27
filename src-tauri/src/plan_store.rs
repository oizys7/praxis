//! 计划数据规范化多表存储：阶段级 upsert（每阶段完成即落库，章节整体替换）与多表重组读取。
//!
//! 与 db.rs 的分工：
//! - db.rs：连接池管理、首连建表（ensure_schema -> db_ddl）
//! - 本模块：流式生成链路（任务 10）的存储层——create_generating_plan /
//!   update_outline_meta / seed_outline / upsert_phase / upsert_chapter /
//!   mark_phase_generating / mark_chapter_generating / set_chapter_generating /
//!   finalize_plan / finalize_failed /
//!   delete_plan_full，以及 get_plan 新路径的多表重组读取（load_study_plan +
//!   纯函数 reassemble_plan）与重生成上下文读取（load_plan_request_meta / load_plan_outline）
//!
//! 五表 DDL（plans/phases/projects/chapters/resources）与两个演进列
//! （status、chapters.content_md）逐字定义在 db_ddl，首连幂等执行。

use std::collections::HashMap;

use sqlx::{PgPool, Row, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::db::{db_err, fail};
use crate::error::AgentError;
use crate::plan_state;
use crate::protocol::{
    ChapterDef, ExerciseDef, GlossaryEntryDef, OutlinePhasePayload, PhaseProject, PlanRequest,
    StudyPhase, StudyPlan, StudyResource, VisualizationDef,
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
    outline_json: Option<&serde_json::Value>,
) -> Result<(), AgentError> {
    // outline_json 只在有新载荷时覆写（COALESCE 保留旧值）：二次元信息更新
    // 不应把已有的完整大纲抹成 NULL。
    sqlx::query(
        "UPDATE plans SET title = $2, overview = $3, estimated_total_hours = $4,
                outline_json = COALESCE($5, outline_json), updated_at = now()
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(title)
    .bind(overview.unwrap_or(""))
    .bind(i32::try_from(estimated_total_hours).unwrap_or(0))
    .bind(outline_json)
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

    // 0. 阶段状态由子章定价汇总——唯一算法见 plan_state::phase_of_chapters。
    //    整体写入口径：空正文是「尚未生成」→ Pending（等待章级填充），全非空
    //    → Complete。这与单章写入的 chapter_of_content（空 → Failed）刻意不同，
    //    两者的区别此前只存在于两段内联代码里，现已收敛到 plan_state 模块。
    let chapter_statuses: Vec<plan_state::ChapterStatus> = phase
        .chapters
        .iter()
        .map(|c| plan_state::chapter_after_bulk_write(&c.content_md))
        .collect();
    let phase_status = plan_state::phase_of_chapters(&chapter_statuses).as_str();

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
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&outline.name)
        .bind(&outline.week_range)
        .bind(plan_state::PhaseStatus::Pending.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| fail("播种阶段骨架", e))?;

        for (j, title) in outline.chapter_titles.iter().enumerate() {
            sqlx::query(
                "INSERT INTO chapters (phase_id, idx, title, summary, status)
                 VALUES ($1, $2, $3, '', $4)",
            )
            .bind(phase_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(title)
            .bind(plan_state::ChapterStatus::Pending.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("播种章节骨架", e))?;
        }
    }

    for (i, resource) in resources.iter().enumerate() {
        sqlx::query(
            "INSERT INTO resources (plan_id, idx, title, type, note, url, verification)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&resource.title)
        .bind(&resource.kind)
        .bind(&resource.note)
        .bind(&resource.url)
        .bind(&resource.verification)
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
            "INSERT INTO resources (plan_id, idx, title, type, note, url, verification)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(plan_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&resource.title)
        .bind(&resource.kind)
        .bind(&resource.note)
        .bind(&resource.url)
        .bind(&resource.verification)
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

    // 阶段联动（同一事务原子）：状态 = 子章汇总纯函数——有生成中章 →
    // generating（逐章推进动画）；全 complete → complete；有 failed → failed；
    // 否则 pending。单章成功但阶段未全完成时从 generating 正确回落，
    // 不再悬挂（2026-10-16 反馈①回归：退出重进徽章消失）
    // SQL 文本唯一来源在 plan_state::SQL_PHASE_STATUS_CASE（与纯函数
    // phase_of_chapters 同语义，配对测试锁死），此处只做拼接。
    let phase_status_sql = format!(
        "UPDATE phases SET status = {} WHERE id = $1",
        plan_state::SQL_PHASE_STATUS_CASE
    );
    sqlx::query(&phase_status_sql)
        .bind(phase_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("推进阶段状态", e))?;

    // 终态与学习状态联动（同事务原子、零额外往返）：
    // - status：generating 计划全章 complete → complete。计划终态按章节口径判定，
    //   不等 done 事件——done 要等 review 的 LLM 调用返回，而 review 只是质量
    //   检查，它的卡顿不该让内容齐备的计划永远停在「生成中」（实测：27/27 章
    //   已完成、review 挂起，前端因章覆盖率显示完成而后端仍是 generating）。
    // - learning_status：active 计划全章 complete → completed；不全 complete
    //   时条件不匹配，保持原值（not_started 同样不自动流转）。
    // 两个 CASE 各自守住当前值，因此同一条语句可服务两类计划。
    // 状态字面量全部走 plan_state（$2/$3 计划态、$4/$5 学习态），
    // 「全章 complete」判定复用 SQL_PLAN_ALL_CHAPTERS_COMPLETE（与
    // plan_state::plan_of_chapters 同语义）。
    let plan_status_sql = format!(
        "UPDATE plans SET
             status = CASE WHEN status = $2 THEN $3 ELSE status END,
             learning_status = CASE WHEN learning_status = $4 THEN $5 ELSE learning_status END
         WHERE id = $1
           AND (status = $2 OR learning_status = $4)
           AND {}",
        plan_state::SQL_PLAN_ALL_CHAPTERS_COMPLETE
    );
    sqlx::query(&plan_status_sql)
        .bind(plan_id)
        .bind(plan_state::PlanStatus::Generating.as_str())
        .bind(plan_state::PlanStatus::Complete.as_str())
        .bind(plan_state::LearningStatus::Active.as_str())
        .bind(plan_state::LearningStatus::Completed.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("推进学习状态", e))?;

    tx.commit().await.map_err(|e| fail("提交单章事务", e))?;
    info!(%plan_id, phase_index, chapter_index, "单章已 upsert（status=complete）");
    Ok(())
}

/// 阶段置 generating（regenerate_phase 开始时调用，前端即时转「生成中」；
/// 落库持久——退出再进入仍显示生成中，2026-10-16 反馈①）；
/// 阶段内容（chapters）不动，各章由 chapterStart 事件逐章点亮；
/// 重生成成功后由 upsert_phase 整体替换。消费方：regenerate_phase。
pub async fn mark_phase_generating(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
) -> Result<(), AgentError> {
    sqlx::query("UPDATE phases SET status = $3 WHERE plan_id = $1 AND idx = $2")
        .bind(plan_id)
        .bind(i32::try_from(phase_index).unwrap_or(0))
        .bind(plan_state::PhaseStatus::Generating.as_str())
        .execute(pool)
        .await
        .map_err(|e| fail("置阶段生成中", e))?;
    info!(%plan_id, phase_index, "阶段已置 generating");
    Ok(())
}

/// 章节置 generating（regenerate_chapter 开始时调用）；注意同时把所属阶段置
/// generating——单章重生成会让阶段显示「生成中」，完成后由 upsert_chapter
/// 阶段联动翻牌。状态落库持久（2026-10-16 反馈①）。消费方：regenerate_chapter。
pub async fn mark_chapter_generating(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<(), AgentError> {
    let phase_i32 = i32::try_from(phase_index).unwrap_or(0);
    let chapter_i32 = i32::try_from(chapter_index).unwrap_or(0);
    sqlx::query(
        "UPDATE chapters c SET status = $4
         FROM phases p WHERE c.phase_id = p.id AND p.plan_id = $1 AND p.idx = $2 AND c.idx = $3",
    )
    .bind(plan_id)
    .bind(phase_i32)
    .bind(chapter_i32)
    .bind(plan_state::ChapterStatus::Generating.as_str())
    .execute(pool)
    .await
    .map_err(|e| fail("置章节生成中", e))?;
    sqlx::query("UPDATE phases SET status = $3 WHERE plan_id = $1 AND idx = $2")
        .bind(plan_id)
        .bind(phase_i32)
        .bind(plan_state::PhaseStatus::Generating.as_str())
        .execute(pool)
        .await
        .map_err(|e| fail("置所属阶段生成中", e))?;
    info!(%plan_id, phase_index, chapter_index, "章节及所属阶段已置 generating");
    Ok(())
}

/// 单章置 generating（chapterStart 事件到达时逐章点亮，2026-10-16 反馈④）：
/// 全阶段重生成中当前章开始生成即落库转圈；所属阶段已在入口由
/// mark_phase_generating 置过，不再重复写。消费方：RegenSink ChapterStart 分支。
pub async fn set_chapter_generating(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<(), AgentError> {
    sqlx::query(
        "UPDATE chapters c SET status = $4
         FROM phases p WHERE c.phase_id = p.id AND p.plan_id = $1 AND p.idx = $2 AND c.idx = $3",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .bind(plan_state::ChapterStatus::Generating.as_str())
    .execute(pool)
    .await
    .map_err(|e| fail("置章节生成中", e))?;
    info!(%plan_id, phase_index, chapter_index, "章节已置 generating");
    Ok(())
}

/// 计划状态的章级三值判定（`Complete` / `Partial` / `Failed`）。
///
/// 收尾结算的唯一入口。`Partial` 存在的理由：自愈链跑完之后，一份 7 章可用、
/// 1 章空缺的计划既不该报成功（用户在别处会读到空章），也不该报失败
/// （7 章真内容被一句话抹掉）。二值口径只能在这两种谎之间二选一。
///
/// 仍是「读全部章状态 + 纯函数」的低频路径（收尾/清扫各一次），与热路径上
/// `upsert_chapter` 的 SQL 常量不冲突：热路径只判「能否翻 complete」，
/// 生成进行中翻 partial 没有意义（其余章只是还没轮到）。
pub async fn plan_status_of_chapters(
    pool: &PgPool,
    plan_id: Uuid,
) -> Result<plan_state::PlanStatus, AgentError> {
    // 判定口径唯一来源：plan_state::plan_of_chapters（纯函数）。
    // 这是低频路径——终态收尾与启动清扫各调一次，不在「每章写入」的热路径上，
    // 因此用「读全部章状态 + 纯函数」换取可测的单一语义；热路径上的状态推进
    // 仍走 SQL 常量（SQL_PHASE_STATUS_CASE），两者由配对测试锁死。
    // 未知状态兜底 Pending：plan_of_chapters 视其为「非 complete」→ 不放宽判定。
    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT c.status FROM chapters c JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询章节状态", e))?;
    let parsed: Vec<plan_state::ChapterStatus> = statuses
        .iter()
        .map(|raw| {
            plan_state::ChapterStatus::parse(raw).unwrap_or(plan_state::ChapterStatus::Pending)
        })
        .collect();
    Ok(plan_state::plan_of_chapters(&parsed))
}

/// 启动自愈报告（幂等；每次启动执行一次）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// 扫过的可疑计划数（`status` 为 failed 或 generating）
    pub scanned_plans: u64,
    /// 反推后由非 complete 变为 complete 的计划数
    pub healed_plans: u64,
    /// 反推修正的章节 / 阶段行数
    pub healed_children: u64,
}

impl SweepReport {
    /// 没有任何修复动作时调用方无需打日志。
    pub fn is_clean(&self) -> bool {
        self.healed_plans == 0 && self.healed_children == 0
    }
}
// ---------------------------------------------------------------------------
// 计划分组（用户自定义）
//
// 分组只是「组织视图的方式」：一个计划最多属于一个组，删组不删计划（外键
// ON DELETE SET NULL 让组内计划回到未分组）。空组必须能保留，所以是独立表
// 而不是 plans 上的一个字符串标签——否则最后一个计划移走后组名就丢了。
// ---------------------------------------------------------------------------

/// 分组列表（含组内计划数），按用户排序位、再按创建时间
pub(crate) async fn list_plan_groups(
    pool: &PgPool,
) -> Result<Vec<crate::protocol::PlanGroup>, AgentError> {
    let rows = sqlx::query(
        "SELECT g.id, g.name, COUNT(p.id) AS plan_count          FROM plan_groups g LEFT JOIN plans p ON p.group_id = g.id          GROUP BY g.id, g.name, g.sort_order, g.created_at          ORDER BY g.sort_order, g.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询计划分组", e))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: Uuid = row.try_get("id").map_err(db_err)?;
        let name: String = row.try_get("name").map_err(db_err)?;
        let plan_count: i64 = row.try_get("plan_count").map_err(db_err)?;
        out.push(crate::protocol::PlanGroup {
            id: id.to_string(),
            name,
            plan_count,
        });
    }
    Ok(out)
}

/// 新建分组：排序位追加到末尾（「新建的在最后」符合预期）
pub(crate) async fn create_plan_group(
    pool: &PgPool,
    name: &str,
) -> Result<crate::protocol::PlanGroup, AgentError> {
    let row = sqlx::query(
        "INSERT INTO plan_groups (id, name, sort_order)          VALUES ($1, $2, COALESCE((SELECT MAX(sort_order) + 1 FROM plan_groups), 0))          RETURNING id, name",
    )
    .bind(Uuid::new_v4())
    .bind(name.trim())
    .fetch_one(pool)
    .await
    .map_err(|e| fail("新建计划分组", e))?;

    let id: Uuid = row.try_get("id").map_err(db_err)?;
    let name: String = row.try_get("name").map_err(db_err)?;
    Ok(crate::protocol::PlanGroup {
        id: id.to_string(),
        name,
        plan_count: 0,
    })
}

/// 重命名分组
pub(crate) async fn rename_plan_group(
    pool: &PgPool,
    id: Uuid,
    name: &str,
) -> Result<(), AgentError> {
    sqlx::query("UPDATE plan_groups SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(name.trim())
        .execute(pool)
        .await
        .map_err(|e| fail("重命名计划分组", e))?;
    Ok(())
}

/// 删除分组（组内计划因外键 ON DELETE SET NULL 自动回到未分组，不删计划）
pub(crate) async fn delete_plan_group(pool: &PgPool, id: Uuid) -> Result<(), AgentError> {
    sqlx::query("DELETE FROM plan_groups WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| fail("删除计划分组", e))?;
    Ok(())
}

/// 把计划移入分组（None = 移出到未分组）
pub(crate) async fn set_plan_group(
    pool: &PgPool,
    plan_id: Uuid,
    group_id: Option<Uuid>,
) -> Result<(), AgentError> {
    // 刻意不动 updated_at：调整分组是视图操作，不该让计划看起来「刚更新过」
    sqlx::query("UPDATE plans SET group_id = $2 WHERE id = $1")
        .bind(plan_id)
        .bind(group_id)
        .execute(pool)
        .await
        .map_err(|e| fail("调整计划分组", e))?;
    Ok(())
}

/// 启动自愈（任务 22 + 2026-10-22 改为「自下而上反推」）。
///
/// 崩溃 / 被强杀 / 重启后，后台生成任务随进程消失，不会再有终态事件来收尾。
///
/// **口径**：状态是派生数据，唯一真相在最下层——章节的 `content_md`。所以自愈
/// 不做「找出哪些行还是 generating 再去修」的症状式修补（那样每种残留形态都要
/// 单独写一条分支，且漏掉 pending 残留、状态与正文矛盾等情形），而是：
///
/// 1. 取出状态可疑的计划（`failed` / `generating`；`complete` 视为已收口不扫）；
/// 2. 对每个计划，从最下层往上反推整棵状态树：
///    - chapters ← `content_md`（有正文 → complete，无正文 → failed）；
///    - phases ← 子章聚合（`SQL_PHASE_STATUS_CASE`，与在线写入同一条 SQL 常量）；
///    - plans ← 全部章聚合（`SQL_PLAN_ALL_CHAPTERS_COMPLETE`）。
///
/// 这样一次覆盖全部不一致形态：崩溃留下的 generating、骨架期的 pending、正文已
/// 写但状态没跟上、状态与正文矛盾（complete 却无正文）都收敛到同一个正确答案。
///
/// 只更新「状态确实需要变」的行（`IS DISTINCT FROM`），因此计数即真实修复量，
/// 重复执行恒为 0（幂等）。
///
/// 桌面单实例语义假定：若双实例并开，自愈会按「当下的正文」给存活实例进行中的
/// 行判态——该实例后续写入会重新推进状态，最终一致。
pub async fn sweep_stale_generating(pool: &PgPool) -> Result<SweepReport, AgentError> {
    let mut report = SweepReport::default();

    let candidates: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM plans WHERE status = $1 OR status = $2 ORDER BY created_at",
    )
    .bind(plan_state::PlanStatus::Failed.as_str())
    .bind(plan_state::PlanStatus::Generating.as_str())
    .fetch_all(pool)
    .await
    .map_err(|e| fail("扫描可疑计划", e))?;
    report.scanned_plans = candidates.len() as u64;

    for plan_id in candidates {
        // ① 叶子层：章节状态的真相只有正文
        let fixed_chapters: Vec<Uuid> = sqlx::query_scalar(&format!(
            "UPDATE chapters c SET status = {from_content}, updated_at = now()
             WHERE c.phase_id IN (SELECT id FROM phases WHERE plan_id = $1)
               AND c.status IS DISTINCT FROM {from_content}
             RETURNING c.id",
            from_content = plan_state::SQL_CHAPTER_STATUS_FROM_CONTENT
        ))
        .bind(plan_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("按正文反推章节状态", e))?;
        report.healed_children += fixed_chapters.len() as u64;

        // ② 阶段层：由子章聚合（与 upsert_chapter 共用同一条 SQL 常量）
        let fixed_phases: Vec<Uuid> = sqlx::query_scalar(&format!(
            "UPDATE phases SET status = {case}, updated_at = now()
             WHERE plan_id = $1 AND status IS DISTINCT FROM {case}
             RETURNING id",
            case = plan_state::SQL_PHASE_STATUS_CASE
        ))
        .bind(plan_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("按子章反推阶段状态", e))?;
        report.healed_children += fixed_phases.len() as u64;

        // ③ 计划层：由全部章聚合（$2 = complete、$3 = failed）
        let healed: Option<String> = sqlx::query_scalar(&format!(
            "UPDATE plans
             SET status = CASE WHEN {all_done} THEN $2 ELSE $3 END, updated_at = now()
             WHERE id = $1 AND status IS DISTINCT FROM (CASE WHEN {all_done} THEN $2 ELSE $3 END)
             RETURNING status",
            all_done = plan_state::SQL_PLAN_ALL_CHAPTERS_COMPLETE
        ))
        .bind(plan_id)
        .bind(plan_state::PlanStatus::Complete.as_str())
        .bind(plan_state::PlanStatus::Failed.as_str())
        .fetch_optional(pool)
        .await
        .map_err(|e| fail("按章节反推计划状态", e))?;
        if healed.as_deref() == Some(plan_state::PlanStatus::Complete.as_str()) {
            report.healed_plans += 1;
        }
    }

    if !report.is_clean() {
        info!(
            scanned_plans = report.scanned_plans,
            healed_plans = report.healed_plans,
            healed_children = report.healed_children,
            "启动自愈完成：按正文 / 子章 / 全章自下而上反推状态树"
        );
    }
    Ok(report)
}

/// 失败收尾（children 位）：phases/chapters 中 status='pending' 的行置 failed；
/// complete 永不被 failed 覆盖（状态机不变量：失败收尾只动 pending）。
/// 消费方：finalize_plan(status=failed) / regenerate 失败路径（任务 15）。
pub async fn finalize_failed(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    sqlx::query("UPDATE phases SET status = $2 WHERE plan_id = $1 AND status IN ($3, $4)")
        .bind(plan_id)
        .bind(plan_state::PhaseStatus::Failed.as_str())
        .bind(plan_state::PhaseStatus::Pending.as_str())
        .bind(plan_state::PhaseStatus::Generating.as_str())
        .execute(pool)
        .await
        .map_err(|e| fail("收尾失败阶段状态", e))?;
    sqlx::query(
        "UPDATE chapters c SET status = $2 FROM phases p
         WHERE c.phase_id = p.id AND p.plan_id = $1 AND c.status IN ($3, $4)",
    )
    .bind(plan_id)
    .bind(plan_state::ChapterStatus::Failed.as_str())
    .bind(plan_state::ChapterStatus::Pending.as_str())
    .bind(plan_state::ChapterStatus::Generating.as_str())
    .execute(pool)
    .await
    .map_err(|e| fail("收尾失败章节状态", e))?;
    info!(%plan_id, "pending/generating 阶段与章节已置 failed（complete 不受影响）");
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
    status: plan_state::PlanStatus,
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
    .bind(status.as_str())
    .bind(title)
    .bind(estimated_total_hours.map(|h| i32::try_from(h).unwrap_or(0)))
    .execute(pool)
    .await
    .map_err(|e| fail("收尾计划状态", e))?;
    // 状态类型化后只剩两种收尾：failed 需要把滞留 pending/generating 子行一并
    // 收口；complete 不需要额外动作。历史上这里还有一个「未知状态」warn 分支，
    // 枚举消除了它（调用方传错字符串这类缺陷从此不可能出现）。
    if status == plan_state::PlanStatus::Failed {
        finalize_failed(pool, plan_id).await?;
    }
    info!(%plan_id, status = status.as_str(), "计划状态已收尾");
    Ok(())
}

/// regenerate 收尾（章级三值口径）：done(plan) 后先收掉本次没产出的子行
/// （图内静默失败的章不会发事件，不收口就永远停在 generating、前端一直转圈），
/// 再按章内容算出 complete / partial / failed 并无条件写回。
///
/// 历史两版都是二值思维：更早的 `WHERE status <> 'failed'` 守卫让补齐失败的
/// 计划行永久卡 failed（2026-10-16 计划 68c1aee0 实锤）；随后改成「只在全章
/// complete 时写回」，部分成功时又停在原状态。三值口径消除了这两个死角。
/// 消费方：commands regenerate 双命令的 done(plan) 路径。
/// 整计划重生成前的重置：清空旧的派生内容与生成上下文，把计划行拉回
/// generating。请求参数（目标/水平/时长/周数/偏好/学习日）沿用库中原值。
///
/// 为什么必须清空：整计划重生成会产出**全新大纲**，旧的阶段/章节索引与标题
/// 都不再对应，留着只会与新内容混在一起；generation_artifacts 按
/// (阶段, 章) 索引，旧素材会让新生成读到错章的材料，必须一并丢弃；学习日程
/// 指向的是旧章节，同样失效（重新开始学习由用户再点）。
pub async fn reset_plan_for_regeneration(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启整计划重生成事务", e))?;
    // 顺序敏感：先清引用了 phase/chapter 索引的表，再删 phases（子表级联）
    for (label, sql) in [
        (
            "清理旧生成留档",
            "DELETE FROM generation_artifacts WHERE plan_id = $1",
        ),
        (
            "清理旧学习日程",
            "DELETE FROM study_schedule WHERE plan_id = $1",
        ),
        ("清理旧推荐资源", "DELETE FROM resources WHERE plan_id = $1"),
        ("清理旧阶段与章节", "DELETE FROM phases WHERE plan_id = $1"),
    ] {
        sqlx::query(sql)
            .bind(plan_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail(label, e))?;
    }
    sqlx::query(
        // estimated_total_hours 是 NOT NULL 列：归零而非置空（新大纲的 outline
        // 事件会重新写入真实学时）
        "UPDATE plans SET status = $2, learning_status = $3, outline_json = NULL,
         estimated_total_hours = 0, updated_at = now() WHERE id = $1",
    )
    .bind(plan_id)
    .bind(plan_state::PlanStatus::Generating.as_str())
    .bind(plan_state::LearningStatus::NotStarted.as_str())
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("重置计划行状态", e))?;
    tx.commit()
        .await
        .map_err(|e| fail("提交整计划重生成事务", e))?;
    info!(
        %plan_id,
        "整计划重生成：旧阶段/章节/素材/日程已清空，计划行回到 generating"
    );
    Ok(())
}

pub async fn finalize_regen_complete(pool: &PgPool, plan_id: Uuid) -> Result<(), AgentError> {
    // 先把本次没产出的子行收口：单章生成失败在图内是静默的（不炸整条流，
    // 也不发 chapter 事件），若不清理，那一章会永远停在 generating、前端一直
    // 转圈。finalize_failed 只动 pending|generating，complete 章不受影响，
    // 因此对「部分成功的重生成」也只是把没产出的部分标成失败。
    finalize_failed(pool, plan_id).await?;
    // 三值落地：原来只有「全 complete」才写回，部分成功时会停在旧状态
    // （典型是停在 failed 永不翻身）。现在按章级口径无条件重算。
    let status = plan_status_of_chapters(pool, plan_id).await?;
    sqlx::query("UPDATE plans SET status = $2, updated_at = now() WHERE id = $1")
        .bind(plan_id)
        .bind(status.as_str())
        .execute(pool)
        .await
        .map_err(|e| fail("收尾重生成计划状态", e))?;
    info!(%plan_id, status = status.as_str(), "重生成收口：计划行状态已按章级口径更新");
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
    let row = sqlx::query(
        "SELECT title, overview, estimated_total_hours, outline_json FROM plans WHERE id = $1",
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询计划大纲元信息", e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let hours: i32 = row.try_get("estimated_total_hours").map_err(db_err)?;

    // 优先：生成时落下的完整大纲。重生成要靠它还原提示词上下文——章型、
    // 知识点、阶段目标、版本事实。缺了这些，`_expand_chapter_draft` 会把章型
    // 兜底成 concept、只用章标题当检索词、并告诉模型「未探测到版本信息」。
    // 旧计划该列为 NULL，回退到下面按 phases/chapters 行重建的骨架形态。
    let stored: Option<serde_json::Value> = row.try_get("outline_json").map_err(db_err)?;
    let mut outline = match stored {
        Some(value) => {
            let parsed: crate::protocol::RegenOutline =
                serde_json::from_value(value).map_err(|e| {
                    AgentError::Db(format!("解析已存计划大纲（outline_json）失败：{e}"))
                })?;
            PlanOutlineRow {
                title: parsed.title,
                overview: parsed.overview,
                estimated_total_hours: parsed.estimated_total_hours,
                phases: parsed.phases,
            }
        }
        None => PlanOutlineRow {
            title: row.try_get("title").map_err(db_err)?,
            overview: row.try_get("overview").map_err(db_err)?,
            estimated_total_hours: hours.max(0) as u32,
            phases: rebuild_outline_phases(pool, plan_id).await?,
        },
    };

    // 检索素材留档：填成与 chapter_titles 等长的并行数组。Python 侧命中即跳过
    // 联网检索——重生成与首次生成因此用同一份素材，且省掉重复的检索计费。
    apply_research_artifacts(pool, plan_id, &mut outline.phases).await?;
    Ok(Some(outline))
}

/// 骨架重建：outline_json 缺失（本轮改造前生成的老计划）时，按
/// phases/chapters 行还原最小可用大纲。这条路径拿不到重生成上下文
/// （章型 / 知识点 / 阶段目标 / 版本事实）。
async fn rebuild_outline_phases(
    pool: &PgPool,
    plan_id: Uuid,
) -> Result<Vec<OutlinePhasePayload>, AgentError> {
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
            ..Default::default()
        });
    }
    Ok(phases)
}

/// 把 generation_artifacts 里的检索素材填进大纲阶段。
/// 无留档 = 空数组 = 该章照常联网检索（不是错误，老计划与首次生成都是这样）。
async fn apply_research_artifacts(
    pool: &PgPool,
    plan_id: Uuid,
    phases: &mut [OutlinePhasePayload],
) -> Result<(), AgentError> {
    let rows = sqlx::query(
        "SELECT phase_idx, chapter_idx, payload FROM generation_artifacts
         WHERE plan_id = $1 AND kind = 'research'",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询检索素材留档", e))?;

    let mut by_key: HashMap<(i32, i32), serde_json::Value> = HashMap::new();
    for row in rows {
        let phase_idx: i32 = row.try_get("phase_idx").map_err(db_err)?;
        let chapter_idx: i32 = row.try_get("chapter_idx").map_err(db_err)?;
        let payload: serde_json::Value = row.try_get("payload").map_err(db_err)?;
        by_key.insert((phase_idx, chapter_idx), payload);
    }

    for (index, phase) in phases.iter_mut().enumerate() {
        let phase_idx = i32::try_from(index).unwrap_or(-1);
        phase.chapter_research_snippets = (0..phase.chapter_titles.len())
            .map(|chapter_index| {
                let key = (phase_idx, i32::try_from(chapter_index).unwrap_or(-1));
                by_key
                    .get(&key)
                    .and_then(|payload| payload.as_array())
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
    }
    Ok(())
}

/// 写入章节生成输入留档（research / prompt 两类）。
///
/// 复合主键 upsert：重生成同一章只覆盖自己那一行，不会追加膨胀——
/// 这也是留档能长期开着的前提。写失败由调用方只 warn，绝不影响生成主链路：
/// 留档是「加速与追溯」的优化，不是正确性依赖。
pub async fn upsert_chapter_artifacts(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
    research_snippets: &[serde_json::Value],
    prompt_trace: Option<&serde_json::Value>,
) -> Result<(), AgentError> {
    let phase_idx = i32::try_from(phase_index).unwrap_or(-1);
    let chapter_idx = i32::try_from(chapter_index).unwrap_or(-1);
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启生成留档事务", e))?;

    let research = serde_json::Value::Array(research_snippets.to_vec());
    sqlx::query(
        "INSERT INTO generation_artifacts (plan_id, phase_idx, chapter_idx, kind, payload)
         VALUES ($1, $2, $3, 'research', $4)
         ON CONFLICT (plan_id, phase_idx, chapter_idx, kind)
         DO UPDATE SET payload = EXCLUDED.payload, created_at = now()",
    )
    .bind(plan_id)
    .bind(phase_idx)
    .bind(chapter_idx)
    .bind(&research)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("写入检索素材留档", e))?;

    if let Some(trace) = prompt_trace {
        sqlx::query(
            "INSERT INTO generation_artifacts (plan_id, phase_idx, chapter_idx, kind, payload)
             VALUES ($1, $2, $3, 'prompt', $4)
             ON CONFLICT (plan_id, phase_idx, chapter_idx, kind)
             DO UPDATE SET payload = EXCLUDED.payload, created_at = now()",
        )
        .bind(plan_id)
        .bind(phase_idx)
        .bind(chapter_idx)
        .bind(trace)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("写入提示词留档", e))?;
    }

    tx.commit().await.map_err(|e| fail("提交生成留档事务", e))?;
    Ok(())
}

/// 按保留期清理留档：只清「已完成且超过保留期未变动」的计划。
///
/// 生成中 / 失败的计划一概不动——失败计划随时可能被用户点重生成，
/// 那时正需要留档里的素材与提示词。留档不承担长期归档职责，
/// 计划一旦稳定完成，过程输入就没有保留价值了。
pub async fn purge_stale_artifacts(pool: &PgPool, retention_days: i64) -> Result<u64, AgentError> {
    let result = sqlx::query(
        "DELETE FROM generation_artifacts
         WHERE plan_id IN (
             SELECT id FROM plans
             WHERE status = $1 AND updated_at < now() - make_interval(days => $2)
         )",
    )
    .bind(plan_state::PlanStatus::Complete.as_str())
    .bind(i32::try_from(retention_days).unwrap_or(7))
    .execute(pool)
    .await
    .map_err(|e| fail("清理过期生成留档", e))?;
    Ok(result.rows_affected())
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
/// 章型兜底：空串与未知值一律按 concept（DB DEFAULT 只覆盖未显式插入的情形）。
fn chapter_kind_or_default(kind: &str) -> &str {
    match kind.trim() {
        known @ ("concept" | "practice" | "project" | "review") => known,
        _ => "concept",
    }
}

/// 章节行 status 定价（保留为薄封装，便于函数体内联阅读与单元测试引用）。
/// 唯一算法在 `plan_state::chapter_of_content`——本模块不再自带一份。
fn chapter_row_status(content_md: &str) -> &'static str {
    plan_state::chapter_of_content(content_md).as_str()
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
        "INSERT INTO chapters (phase_id, idx, title, summary, content_md, status, kind,
                               key_terms, visualizations, exercises)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (phase_id, idx) DO UPDATE SET
             title = EXCLUDED.title,
             summary = EXCLUDED.summary,
             content_md = EXCLUDED.content_md,
             status = EXCLUDED.status,
             kind = EXCLUDED.kind,
             key_terms = EXCLUDED.key_terms,
             visualizations = EXCLUDED.visualizations,
             exercises = EXCLUDED.exercises
         RETURNING id",
    )
    .bind(phase_id)
    .bind(i32::try_from(idx).unwrap_or(0))
    .bind(&chapter.title)
    .bind(&chapter.summary)
    .bind(&chapter.content_md)
    .bind(chapter_row_status(&chapter.content_md))
    .bind(chapter_kind_or_default(&chapter.kind))
    .bind(sqlx::types::Json(&chapter.key_terms))
    .bind(sqlx::types::Json(&chapter.visualizations))
    .bind(sqlx::types::Json(&chapter.exercises))
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
    /// 章型（NOT NULL DEFAULT 'concept'）
    kind: String,
    /// 本章核心术语（JSONB 数组，用于汇总术语表）
    key_terms: Vec<String>,
    /// 过程动画（JSONB 数组）
    visualizations: Vec<VisualizationDef>,
    /// 结构化自测题（JSONB 数组）
    exercises: Vec<ExerciseDef>,
    /// 只读派生：该章是否有已完成的打卡记录（由 study_schedule 聚合）
    done: bool,
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

    // 章节与项目各用一次查询取回全计划数据，再按 phase_id 内存分组：
    // 原先「每阶段两次查询」是 N+1（6 阶段 = 13 次往返），对远端 PG 意味着
    // 十余次网络 RTT，配合前端轮询会占满连接池并拖慢 upsert（实测单条章节
    // 查询达 143s，并出现 statement timeout 与连接获取超阈值告警）。
    let mut chapters_by_phase: HashMap<Uuid, Vec<ChapterRows>> = HashMap::new();
    for crow in sqlx::query(
        "SELECT c.phase_id, c.title, c.summary, c.status, c.content_md, c.kind, c.key_terms,
                c.visualizations, c.exercises,
                EXISTS (
                    SELECT 1 FROM study_schedule s
                    WHERE s.plan_id = p.plan_id
                      AND s.phase_idx = p.idx
                      AND s.chapter_idx = c.idx
                      AND s.done
                ) AS done
         FROM chapters c JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1
         ORDER BY p.idx ASC, c.idx ASC",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询章节列表", e))?
    {
        chapters_by_phase
            .entry(crow.try_get("phase_id").map_err(db_err)?)
            .or_default()
            .push(ChapterRows {
                title: crow.try_get("title").map_err(db_err)?,
                summary: crow.try_get("summary").map_err(db_err)?,
                status: crow.try_get("status").map_err(db_err)?,
                content_md: crow.try_get("content_md").map_err(db_err)?,
                kind: crow.try_get("kind").map_err(db_err)?,
                key_terms: crow
                    .try_get::<sqlx::types::Json<Vec<String>>, _>("key_terms")
                    .map_err(db_err)?
                    .0,
                visualizations: crow
                    .try_get::<sqlx::types::Json<Vec<VisualizationDef>>, _>("visualizations")
                    .map_err(db_err)?
                    .0,
                exercises: crow
                    .try_get::<sqlx::types::Json<Vec<ExerciseDef>>, _>("exercises")
                    .map_err(db_err)?
                    .0,
                done: crow.try_get("done").map_err(db_err)?,
            });
    }

    // 项目（projects.phase_id UNIQUE，每个阶段至多一行）
    let mut project_by_phase: HashMap<Uuid, ProjectRow> = HashMap::new();
    for p in sqlx::query(
        "SELECT pr.phase_id, pr.name, pr.description, pr.background, pr.steps,
                pr.acceptance_criteria, pr.estimated_hours
         FROM projects pr JOIN phases p ON pr.phase_id = p.id
         WHERE p.plan_id = $1",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询阶段项目", e))?
    {
        project_by_phase.insert(
            p.try_get("phase_id").map_err(db_err)?,
            ProjectRow {
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
            },
        );
    }

    let mut phases = Vec::with_capacity(phase_rows.len());
    for row in phase_rows {
        let phase_id: Uuid = row.try_get("id").map_err(db_err)?;
        let objectives: sqlx::types::Json<Vec<String>> =
            row.try_get("objectives").map_err(db_err)?;
        let practice: sqlx::types::Json<Vec<String>> = row.try_get("practice").map_err(db_err)?;
        let assignments: sqlx::types::Json<Vec<String>> =
            row.try_get("assignments").map_err(db_err)?;

        phases.push(PhaseRows {
            name: row.try_get("name").map_err(db_err)?,
            week_range: row.try_get("week_range").map_err(db_err)?,
            status: row.try_get("status").map_err(db_err)?,
            objectives: objectives.0,
            practice: practice.0,
            assignments: assignments.0,
            // remove 而非 get：每阶段只消费一次，顺手释放已取走的行
            project: project_by_phase.remove(&phase_id),
            chapters: chapters_by_phase.remove(&phase_id).unwrap_or_default(),
        });
    }

    let mut resources = Vec::new();
    for r in sqlx::query(
        "SELECT title, type, note, url, verification
             FROM resources WHERE plan_id = $1 ORDER BY idx ASC",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询资源列表", e))?
    {
        resources.push(StudyResource {
            title: r.try_get("title").map_err(db_err)?,
            kind: r.try_get("type").map_err(db_err)?,
            note: r.try_get("note").map_err(db_err)?,
            url: r.try_get("url").map_err(db_err)?,
            verification: r.try_get("verification").map_err(db_err)?,
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
    let phases: Vec<StudyPhase> = rows.phases.into_iter().map(reassemble_phase).collect();
    // 术语表是计算属性：由各章 keyTerms 去重汇总（与 Python 侧 assemble 同一口径）。
    // 只读产物，随计划重新生成而变，因此不落独立表、不提供编辑入口。
    let glossary = collect_glossary(&phases);
    StudyPlan {
        title: rows.title,
        overview: rows.overview,
        estimated_total_hours: rows.estimated_total_hours.max(0) as u32,
        phases,
        resources: rows.resources,
        review_notes: String::new(),
        glossary,
    }
}

/// 从各章 keyTerms 去重汇总术语表（保留首次出现的章节标题，上限 60 条）。
fn collect_glossary(phases: &[StudyPhase]) -> Vec<GlossaryEntryDef> {
    const LIMIT: usize = 60;
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut entries: Vec<GlossaryEntryDef> = Vec::new();
    for phase in phases {
        for chapter in &phase.chapters {
            for term in &chapter.key_terms {
                let name = term.trim();
                if name.is_empty() || !seen.insert(name) {
                    continue;
                }
                entries.push(GlossaryEntryDef {
                    term: name.to_string(),
                    chapter_title: chapter.title.clone(),
                });
                if entries.len() >= LIMIT {
                    return entries;
                }
            }
        }
    }
    entries
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
        kind: row.kind,
        key_terms: row.key_terms,
        visualizations: row.visualizations,
        exercises: row.exercises,
        done: Some(row.done),
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
            kind: "concept".into(),
            key_terms: vec![title.into()],
            visualizations: Vec::new(),
            exercises: Vec::new(),
            done: false,
        };
        let rows = PlanRows {
            title: "测试计划".into(),
            overview: "概览".into(),
            estimated_total_hours: 88,
            resources: vec![StudyResource {
                title: "The Rust Book".into(),
                kind: "docs".into(),
                note: "官方教程".into(),
                url: "https://doc.rust-lang.org/book/".into(),
                verification: "reachable".into(),
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
                kind: "concept".into(),
                key_terms: Vec::new(),
                visualizations: Vec::new(),
                exercises: Vec::new(),
                done: false,
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
