//! 计划数据规范化多表存储：阶段级 upsert（每阶段完成即落库，子表级联替换）与多表重组读取。
//!
//! 与 db.rs 的分工：
//! - db.rs：连接池管理、首连建表（ensure_schema）、旧整串 plan_json 链路（save_plan 等保留）
//! - 本模块：流式生成链路（任务 10）的存储层——create_generating_plan /
//!   update_outline_meta / upsert_phase / finalize_plan / delete_plan_full，
//!   以及 get_plan 新路径的多表重组读取（load_study_plan + 纯函数 reassemble_plan）
//!
//! 九表 DDL（plans + phases + projects + chapters + concepts + knowledge_points +
//! diagrams + deep_dives + resources）逐字定义在 db::ensure_schema，首连幂等执行。

use sqlx::{PgPool, Row};
use tracing::info;
use uuid::Uuid;

use crate::db::{db_err, fail};
use crate::error::AgentError;
use crate::protocol::{
    ChapterDef, ConceptDef, DeepDiveDef, KnowledgeDiagramDef, KnowledgePointDef, MainKnowledgeDef,
    PhaseProject, PlanRequest, StudyPhase, StudyPlan, StudyResource,
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
    sqlx::query(
        "INSERT INTO plans (id, title, goal, level, weeks, status)
         VALUES ($1, $2, $3, $4, $5, 'generating')",
    )
    .bind(plan_id)
    .bind(&title)
    .bind(&request.goal)
    .bind(&request.level)
    .bind(i32::try_from(request.weeks).unwrap_or(0))
    .execute(pool)
    .await
    .map_err(|e| fail("创建生成中计划", e))?;
    info!(%plan_id, "规范化计划行已创建（status=generating）");
    Ok(())
}

/// 大纲就绪：覆写计划标题与预计总学时（title 暂为 goal 截断的占位时由本函数替换）。
/// 消费方：commands::generate_plan（outline 事件到达时）。
pub async fn update_outline_meta(
    pool: &PgPool,
    plan_id: Uuid,
    title: &str,
    estimated_total_hours: i64,
) -> Result<(), AgentError> {
    sqlx::query(
        "UPDATE plans SET title = $2, estimated_total_hours = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(title)
    .bind(i32::try_from(estimated_total_hours).unwrap_or(0))
    .execute(pool)
    .await
    .map_err(|e| fail("更新计划大纲元信息", e))?;
    info!(%plan_id, "计划大纲元信息已更新");
    Ok(())
}

/// 阶段级 upsert（单事务）：写入/更新 (plan_id, idx) 的 phases 行后，整体替换其
/// chapters（concepts/knowledge_points/diagrams/deep_dives 随外键级联清空）与 project。
/// 同一阶段回炉重写时旧数据不残留；事务保证半途失败不留脏数据。
/// 消费方：commands::generate_plan（phase 事件到达时）。
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

    // 1. phases 行：冲突即更新（UNIQUE (plan_id, idx)），RETURNING 拿物理行 id
    let phase_id: Uuid = sqlx::query_scalar(
        "INSERT INTO phases (plan_id, idx, name, week_range, objectives, practice, assignments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (plan_id, idx) DO UPDATE SET
             name = EXCLUDED.name,
             week_range = EXCLUDED.week_range,
             objectives = EXCLUDED.objectives,
             practice = EXCLUDED.practice,
             assignments = EXCLUDED.assignments
         RETURNING id",
    )
    .bind(plan_id)
    .bind(idx)
    .bind(&phase.name)
    .bind(&phase.week_range)
    .bind(sqlx::types::Json(&phase.objectives))
    .bind(sqlx::types::Json(&phase.practice))
    .bind(sqlx::types::Json(&phase.assignments))
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| fail("写入阶段", e))?;

    // 2. 整体替换子数据：先清 project 与 chapters（chapters 的四张子表靠外键级联清空）
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

    // 3. 重插 chapters 与子表（idx 即数组下标，读取按 idx 升序还原顺序）
    for (i, chapter) in phase.chapters.iter().enumerate() {
        let chapter_id: Uuid = sqlx::query_scalar(
            "INSERT INTO chapters (phase_id, idx, title, summary)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(phase_id)
        .bind(i32::try_from(i).unwrap_or(0))
        .bind(&chapter.title)
        .bind(&chapter.summary)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| fail("写入章节", e))?;

        for (j, concept) in chapter.main_knowledge.concepts.iter().enumerate() {
            sqlx::query(
                "INSERT INTO concepts (chapter_id, idx, name, definition) VALUES ($1, $2, $3, $4)",
            )
            .bind(chapter_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(&concept.name)
            .bind(&concept.definition)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("写入概念", e))?;
        }
        for (j, point) in chapter.main_knowledge.knowledge_points.iter().enumerate() {
            sqlx::query(
                "INSERT INTO knowledge_points (chapter_id, idx, title, detail) VALUES ($1, $2, $3, $4)",
            )
            .bind(chapter_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(&point.title)
            .bind(&point.detail)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("写入知识点", e))?;
        }
        for (j, diagram) in chapter.main_knowledge.diagrams.iter().enumerate() {
            sqlx::query(
                "INSERT INTO diagrams (chapter_id, idx, title, mermaid, caption)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(chapter_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(&diagram.title)
            .bind(&diagram.mermaid)
            .bind(&diagram.caption)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("写入图示", e))?;
        }
        for (j, dive) in chapter.deep_dives.iter().enumerate() {
            sqlx::query(
                "INSERT INTO deep_dives (chapter_id, idx, topic, angle, content)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(chapter_id)
            .bind(i32::try_from(j).unwrap_or(0))
            .bind(&dive.topic)
            .bind(&dive.angle)
            .bind(&dive.content)
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("写入深入内容", e))?;
        }
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

/// 生成收尾：覆写 status（complete | failed）；title / estimated_total_hours
/// 为 Some 时一并更新，None 保持原值。
/// 消费方：commands::generate_plan（done 终态与异常路径收尾）。
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
    info!(%plan_id, status, "计划状态已收尾");
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
    objectives: Vec<String>,
    practice: Vec<String>,
    assignments: Vec<String>,
    project: Option<ProjectRow>,
    chapters: Vec<ChapterRows>,
}

struct ChapterRows {
    title: String,
    summary: String,
    concepts: Vec<ConceptDef>,
    knowledge_points: Vec<KnowledgePointDef>,
    diagrams: Vec<KnowledgeDiagramDef>,
    deep_dives: Vec<DeepDiveDef>,
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
        "SELECT id, name, week_range, objectives, practice, assignments
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
            "SELECT id, title, summary FROM chapters WHERE phase_id = $1 ORDER BY idx ASC",
        )
        .bind(phase_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("查询章节列表", e))?;

        let mut chapters = Vec::with_capacity(chapter_rows.len());
        for crow in chapter_rows {
            let chapter_id: Uuid = crow.try_get("id").map_err(db_err)?;
            chapters.push(ChapterRows {
                title: crow.try_get("title").map_err(db_err)?,
                summary: crow.try_get("summary").map_err(db_err)?,
                concepts: fetch_concepts(pool, chapter_id).await?,
                knowledge_points: fetch_knowledge_points(pool, chapter_id).await?,
                diagrams: fetch_diagrams(pool, chapter_id).await?,
                deep_dives: fetch_deep_dives(pool, chapter_id).await?,
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

async fn fetch_concepts(pool: &PgPool, chapter_id: Uuid) -> Result<Vec<ConceptDef>, AgentError> {
    let mut out = Vec::new();
    for r in
        sqlx::query("SELECT name, definition FROM concepts WHERE chapter_id = $1 ORDER BY idx ASC")
            .bind(chapter_id)
            .fetch_all(pool)
            .await
            .map_err(|e| fail("查询概念列表", e))?
    {
        out.push(ConceptDef {
            name: r.try_get("name").map_err(db_err)?,
            definition: r.try_get("definition").map_err(db_err)?,
        });
    }
    Ok(out)
}

async fn fetch_knowledge_points(
    pool: &PgPool,
    chapter_id: Uuid,
) -> Result<Vec<KnowledgePointDef>, AgentError> {
    let mut out = Vec::new();
    for r in sqlx::query(
        "SELECT title, detail FROM knowledge_points WHERE chapter_id = $1 ORDER BY idx ASC",
    )
    .bind(chapter_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询知识点列表", e))?
    {
        out.push(KnowledgePointDef {
            title: r.try_get("title").map_err(db_err)?,
            detail: r.try_get("detail").map_err(db_err)?,
        });
    }
    Ok(out)
}

async fn fetch_diagrams(
    pool: &PgPool,
    chapter_id: Uuid,
) -> Result<Vec<KnowledgeDiagramDef>, AgentError> {
    let mut out = Vec::new();
    for r in sqlx::query(
        "SELECT title, mermaid, caption FROM diagrams WHERE chapter_id = $1 ORDER BY idx ASC",
    )
    .bind(chapter_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询图示列表", e))?
    {
        out.push(KnowledgeDiagramDef {
            title: r.try_get("title").map_err(db_err)?,
            mermaid: r.try_get("mermaid").map_err(db_err)?,
            caption: r.try_get("caption").map_err(db_err)?,
        });
    }
    Ok(out)
}

async fn fetch_deep_dives(pool: &PgPool, chapter_id: Uuid) -> Result<Vec<DeepDiveDef>, AgentError> {
    let mut out = Vec::new();
    for r in sqlx::query(
        "SELECT topic, angle, content FROM deep_dives WHERE chapter_id = $1 ORDER BY idx ASC",
    )
    .bind(chapter_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询深入内容列表", e))?
    {
        out.push(DeepDiveDef {
            topic: r.try_get("topic").map_err(db_err)?,
            angle: r.try_get("angle").map_err(db_err)?,
            content: r.try_get("content").map_err(db_err)?,
        });
    }
    Ok(out)
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
    }
}

fn reassemble_chapter(row: ChapterRows) -> ChapterDef {
    ChapterDef {
        title: row.title,
        summary: row.summary,
        main_knowledge: MainKnowledgeDef {
            concepts: row.concepts,
            knowledge_points: row.knowledge_points,
            diagrams: row.diagrams,
        },
        deep_dives: row.deep_dives,
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
            concepts: vec![ConceptDef {
                name: "所有权".into(),
                definition: "唯一所有者".into(),
            }],
            knowledge_points: vec![KnowledgePointDef {
                title: "移动语义".into(),
                detail: "赋值转移所有权".into(),
            }],
            diagrams: vec![KnowledgeDiagramDef {
                title: "转移图".into(),
                mermaid: "flowchart LR".into(),
                caption: "示意".into(),
            }],
            deep_dives: vec![DeepDiveDef {
                topic: "Drop".into(),
                angle: "RAII".into(),
                content: "长文".into(),
            }],
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
        assert_eq!(first.chapters[0].main_knowledge.concepts[0].name, "所有权");
        assert_eq!(first.chapters[0].deep_dives[0].topic, "Drop");
        assert_eq!(first.project.name, "项目甲");
        assert_eq!(first.project.estimated_hours, 6);
        // 无项目行 -> 协议默认值（name 空串）
        assert_eq!(plan.phases[1].project, PhaseProject::default());
        assert_eq!(plan.review_notes, "");
    }
}
