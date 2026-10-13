//! plan_store 集成测试：#[sqlx::test] 在 DATABASE_URL 指向的 PostgreSQL 实例上
//! 为每个用例创建独立临时库（用例结束自动删除），验证规范化多表存储的真实行为。
//!
//! 运行方式（需本机可连的 PostgreSQL，连接用户需有建库权限）：
//!   DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres cargo test
//! 临时库不承载业务数据，与设置页配置的云端库互不相干。

use sqlx::PgPool;
use uuid::Uuid;

use crate::db;
use crate::plan_store;
use crate::protocol::{
    ChapterDef, ConceptDef, DeepDiveDef, KnowledgeDiagramDef, KnowledgePointDef, MainKnowledgeDef,
    PhaseProject, PlanRequest, StudyPhase,
};

// ---------------------------------------------------------------------------
// fixture 构造
// ---------------------------------------------------------------------------

/// 生成请求 fixture（create_generating_plan 用）
fn sample_request() -> PlanRequest {
    PlanRequest {
        goal: "掌握 Rust 系统编程".into(),
        level: "beginner".into(),
        weekly_hours: 10,
        weeks: 8,
        interests: vec!["cli".into()],
        background: "有 C 基础".into(),
    }
}

/// 章节 fixture：概念 / 知识点 / 图示 / 深入内容各一条，标题均携带入参可追溯
fn sample_chapter(title: &str) -> ChapterDef {
    ChapterDef {
        title: title.into(),
        summary: format!("{title}·摘要"),
        main_knowledge: MainKnowledgeDef {
            concepts: vec![ConceptDef {
                name: format!("{title}·概念"),
                definition: "值在同一时刻只有唯一所有者".into(),
            }],
            knowledge_points: vec![KnowledgePointDef {
                title: format!("{title}·知识点"),
                detail: "赋值默认转移所有权".into(),
            }],
            diagrams: vec![KnowledgeDiagramDef {
                title: format!("{title}·图示"),
                mermaid: "flowchart LR\n  a --> b".into(),
                caption: "所有权转移示意".into(),
            }],
        },
        deep_dives: vec![DeepDiveDef {
            topic: format!("{title}·深入"),
            angle: "内存布局视角".into(),
            content: "Rust 借 RAII 在编译期保证资源释放。".into(),
        }],
    }
}

/// 阶段 fixture：chapter_titles 决定章节数；项目恒有值（name 随阶段名可追溯）
fn sample_phase(name: &str, chapter_titles: &[&str]) -> StudyPhase {
    StudyPhase {
        name: name.into(),
        week_range: "第 1-2 周".into(),
        objectives: vec!["目标甲".into(), "目标乙".into()],
        chapters: chapter_titles.iter().map(|t| sample_chapter(t)).collect(),
        theory: vec![],
        practice: vec!["实操甲".into()],
        assignments: vec!["作业甲".into()],
        project: PhaseProject {
            name: format!("{name}·项目"),
            description: "综合运用本阶段所学".into(),
            background: "补全项目背景".into(),
            steps: vec!["步骤甲".into(), "步骤乙".into()],
            estimated_hours: 6,
            acceptance_criteria: vec!["验收甲".into()],
        },
    }
}

/// 建表（与生产首连同一套幂等 DDL）
async fn prepare_schema(pool: &PgPool) {
    db::ensure_schema(pool).await.expect("ensure_schema 失败");
}

/// 创建一条 generating 计划行，返回其 id
async fn new_generating_plan(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    plan_store::create_generating_plan(pool, id, &sample_request())
        .await
        .expect("create_generating_plan 失败");
    id
}

/// 表行数（表名全部来自本文件字面量，无注入面）
async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("COUNT 查询失败")
}

// ---------------------------------------------------------------------------
// 用例
// ---------------------------------------------------------------------------

/// 同 (plan_id, idx) 两次 upsert：phases 行不翻倍，章节整体替换，阶段名更新
#[sqlx::test]
#[ignore = "需要 DATABASE_URL 指向本机 PostgreSQL（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_phase_idempotent(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::upsert_phase(
        &pool,
        plan_id,
        0,
        &sample_phase("基础阶段", &["章节一", "章节二"]),
    )
    .await
    .expect("首次 upsert 失败");
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("基础阶段v2", &["章节一"]))
        .await
        .expect("二次 upsert 失败");

    assert_eq!(count(&pool, "plans").await, 1);
    assert_eq!(
        count(&pool, "phases").await,
        1,
        "同 (plan_id, idx) 二次 upsert 不得新增 phases 行"
    );
    assert_eq!(
        count(&pool, "chapters").await,
        1,
        "二次 upsert 后章节应整体替换为最新一批"
    );
    assert_eq!(count(&pool, "concepts").await, 1, "子表不得残留旧章节数据");
    let (name, plan_fk): (String, Uuid) = sqlx::query_as("SELECT name, plan_id FROM phases")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "基础阶段v2", "同 idx 二次 upsert 应更新阶段名");
    assert_eq!(plan_fk, plan_id);
}

/// 回炉重写：upsert 覆盖后旧章节行与子行（概念/知识点/图示/深入/项目）不残留
#[sqlx::test]
#[ignore = "需要 DATABASE_URL 指向本机 PostgreSQL（见文件头运行方式），无库环境下跳过"]
async fn test_rework_replaces_children(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::upsert_phase(
        &pool,
        plan_id,
        1,
        &sample_phase("旧阶段", &["旧章节甲", "旧章节乙"]),
    )
    .await
    .expect("首次 upsert 失败");
    plan_store::upsert_phase(&pool, plan_id, 1, &sample_phase("新阶段", &["新章节甲"]))
        .await
        .expect("回炉 upsert 失败");

    let titles: Vec<String> = sqlx::query_scalar("SELECT title FROM chapters ORDER BY idx")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        titles,
        vec!["新章节甲".to_string()],
        "旧章节行必须被删除，不得残留"
    );
    // 每章各 1 条子数据：全库只剩新章节的份额，旧章节子行已被级联清空
    assert_eq!(count(&pool, "concepts").await, 1);
    assert_eq!(count(&pool, "knowledge_points").await, 1);
    assert_eq!(count(&pool, "diagrams").await, 1);
    assert_eq!(count(&pool, "deep_dives").await, 1);
    // 项目行被替换（projects.phase_id UNIQUE，至多一行）
    let project: String = sqlx::query_scalar("SELECT name FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(project, "新阶段·项目");
}

/// 乱序扩展 idx 2,0,1：get_plan 多表重组后 phases 按 idx 升序、子数据完整；
/// update_outline_meta / finalize_plan 的元信息覆写生效
#[sqlx::test]
#[ignore = "需要 DATABASE_URL 指向本机 PostgreSQL（见文件头运行方式），无库环境下跳过"]
async fn test_get_plan_reassembles_sorted(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    // 乱序扩展：2 → 0 → 1
    plan_store::upsert_phase(&pool, plan_id, 2, &sample_phase("阶段丙", &["丙章"]))
        .await
        .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("阶段甲", &["甲章"]))
        .await
        .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 1, &sample_phase("阶段乙", &["乙章"]))
        .await
        .unwrap();
    plan_store::update_outline_meta(&pool, plan_id, "重组后的标题", 66)
        .await
        .unwrap();

    let record = db::get_plan_by_pool(&pool, &plan_id.to_string())
        .await
        .expect("get_plan 失败");
    let names: Vec<&str> = record.plan.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["阶段甲", "阶段乙", "阶段丙"],
        "phases 必须按 idx 升序重组"
    );
    assert_eq!(record.title, "重组后的标题", "大纲元信息应已覆写标题");
    assert_eq!(record.plan.estimated_total_hours, 66);
    // deep_dives / diagrams（以及概念）逐阶段完整
    for phase in &record.plan.phases {
        assert_eq!(phase.chapters.len(), 1);
        let chapter = &phase.chapters[0];
        assert_eq!(chapter.deep_dives.len(), 1, "deep_dives 缺失");
        assert_eq!(
            chapter.deep_dives[0].topic,
            format!("{}·深入", chapter.title)
        );
        assert_eq!(chapter.main_knowledge.diagrams.len(), 1, "diagrams 缺失");
        assert_eq!(chapter.main_knowledge.concepts.len(), 1);
    }
    // 项目重组回 PhaseProject
    assert_eq!(record.plan.phases[0].project.name, "阶段甲·项目");
    assert_eq!(
        record.plan.phases[0].project.steps,
        vec!["步骤甲".to_string(), "步骤乙".to_string()]
    );
    assert_eq!(record.plan.phases[0].project.estimated_hours, 6);

    // finalize：status/title/hours 覆写后读取仍一致
    plan_store::finalize_plan(&pool, plan_id, "complete", Some("完结标题"), Some(77))
        .await
        .expect("finalize_plan 失败");
    let record = db::get_plan_by_pool(&pool, &plan_id.to_string())
        .await
        .unwrap();
    assert_eq!(record.title, "完结标题");
    assert_eq!(record.plan.estimated_total_hours, 77);
    let status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "complete");
}

/// 旧行带 plan_json：get_plan 走旧反序列化路径，行为与规范化改造前一致
#[sqlx::test]
#[ignore = "需要 DATABASE_URL 指向本机 PostgreSQL（见文件头运行方式），无库环境下跳过"]
async fn test_legacy_plan_json_still_reads(pool: PgPool) {
    prepare_schema(&pool).await;
    let id = Uuid::new_v4();
    // 旧版形状：theory 数组、无 chapters、estimatedTotalHours 在 JSON 内
    let legacy_json = r#"{
        "title": "旧版计划",
        "overview": "旧概览",
        "estimatedTotalHours": 42,
        "phases": [
            {
                "name": "旧阶段",
                "weekRange": "第 1-4 周",
                "objectives": ["掌握基础语法"],
                "theory": ["所有权", "借用检查"],
                "practice": ["写 CLI 工具"],
                "assignments": ["完成练习 1"],
                "project": { "name": "命令行待办", "description": "实现待办管理工具" }
            }
        ],
        "resources": []
    }"#;
    sqlx::query(
        "INSERT INTO plans (id, title, goal, overview, level, weeks, plan_json, status, estimated_total_hours)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'complete', 0)",
    )
    .bind(id)
    .bind("旧版计划")
    .bind("学 Rust")
    .bind("旧概览")
    .bind("beginner")
    .bind(4i32)
    .bind(sqlx::types::Json(serde_json::from_str::<serde_json::Value>(legacy_json).unwrap()))
    .execute(&pool)
    .await
    .unwrap();

    let record = db::get_plan_by_pool(&pool, &id.to_string())
        .await
        .expect("旧路径读取失败");
    assert_eq!(record.id, id.to_string());
    assert_eq!(record.title, "旧版计划");
    // 计划体来自 plan_json 反序列化（estimatedTotalHours=42 而非列默认 0）
    assert_eq!(
        record.plan.estimated_total_hours, 42,
        "旧路径必须读 plan_json 内的值"
    );
    assert_eq!(record.plan.phases.len(), 1);
    assert_eq!(
        record.plan.phases[0].theory,
        vec!["所有权".to_string(), "借用检查".to_string()]
    );
    assert_eq!(record.plan.phases[0].project.name, "命令行待办");
}

/// 删 plans 行：九表（plans + 阶段/项目/章节/概念/知识点/图示/深入/资源）全级联清空
#[sqlx::test]
#[ignore = "需要 DATABASE_URL 指向本机 PostgreSQL（见文件头运行方式），无库环境下跳过"]
async fn test_delete_plan_cascades(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::upsert_phase(
        &pool,
        plan_id,
        0,
        &sample_phase("阶段甲", &["甲章一", "甲章二"]),
    )
    .await
    .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 1, &sample_phase("阶段乙", &["乙章"]))
        .await
        .unwrap();
    // resources 尚无业务写入入口（任务 10+），直插一行验证级联
    sqlx::query(
        "INSERT INTO resources (plan_id, idx, title, type, note)
         VALUES ($1, 0, 'The Rust Book', 'docs', '官方教程')",
    )
    .bind(plan_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(count(&pool, "phases").await, 2);
    assert_eq!(count(&pool, "resources").await, 1);

    plan_store::delete_plan_full(&pool, plan_id)
        .await
        .expect("删除失败");

    for table in [
        "plans",
        "phases",
        "projects",
        "chapters",
        "concepts",
        "knowledge_points",
        "diagrams",
        "deep_dives",
        "resources",
    ] {
        assert_eq!(
            count(&pool, table).await,
            0,
            "删除 plans 后 {table} 应级联清空"
        );
    }
}
