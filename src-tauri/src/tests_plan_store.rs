//! plan_store 集成测试：#[sqlx::test] 在 DATABASE_URL 指向的 PostgreSQL 实例上
//! 为每个用例创建独立临时库（用例结束自动删除），验证规范化多表存储的真实行为。
//!
//! 运行方式（需本机可连的 PostgreSQL，连接用户需有建库权限；用例已标 #[ignore]，
//!   在 config.json 填好 database.testUrl 后 cargo test -- --ignored（tests_support 在测试进程启动时桥接 DATABASE_URL，不再手工 export）
//! 临时库不承载业务数据，与设置页配置的云端库互不相干。
//!
//! 任务 18 markdown 化：chapters 只剩 {title, summary, content_md, status}，
//! status 由 content_md 定价（空正文 → failed 可重生成）；四张内容子表已 DROP。

use sqlx::PgPool;
use uuid::Uuid;

use crate::commands;
use crate::db;
use crate::plan_state;
use crate::plan_store;
use crate::plan_store::{PlanOutlineRow, PlanRequestRow};
use crate::protocol::{
    ChapterDef, OutlinePhasePayload, PhaseProject, PlanRequest, StudyPhase, StudyResource,
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
        study_days: None,
    }
}

/// 章节 fixture：markdown 形态（title / summary / contentMd），status 由存储层
/// 按 content_md 定价；正文带章节标题可追溯，保证轮回断言可区分新旧数据。
fn sample_chapter(title: &str) -> ChapterDef {
    ChapterDef {
        title: title.into(),
        summary: format!("{title}·摘要"),
        kind: "concept".into(),
        key_terms: vec![format!("{title}术语")],
        content_md: format!(
            "## {title}\n\n本章围绕「{title}」展开。\n\n```mermaid\nflowchart TD\n    A --> B\n```\n"
        ),
        visualizations: Vec::new(),
        exercises: Vec::new(),
        done: None,
        status: None,
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
        status: None,
    }
}

/// 阶段级定价（审查 P2 修复）：upsert_phase 含空正文章节 → 阶段 status 保持
/// pending（重生成入口不丢失）；整阶段正文齐备 → 阶段 complete 联动翻牌
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_phase_status_derived_from_chapter_content(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();

    // 阶段含一个空正文章 → 阶段保持 pending（不得误翻牌 complete）
    let mut mixed = sample_phase("阶段甲", &["章A", "章B"]);
    mixed.chapters[1].content_md = "   ".to_string();
    plan_store::upsert_phase(&pool, plan_id, 0, &mixed)
        .await
        .expect("upsert_phase 混内容阶段失败");
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("pending".into()),
        "存在空正文章节，阶段不得置 complete"
    );
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 1).await,
        Some("failed".into())
    );

    // 阶段级重生成（回炉）后全部正文齐备 → 阶段联动翻牌 complete
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("阶段甲", &["章A", "章B"]))
        .await
        .expect("upsert_phase 回炉失败");
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into()),
        "全章节正文齐备后阶段应翻牌 complete"
    );

    // 边界：无章节的阶段不得置 complete（全(...) 对空集误真）
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段乙"], &[2]), &[])
        .await
        .unwrap();
    let mut bare = sample_phase("阶段乙", &[]);
    bare.chapters.clear();
    plan_store::upsert_phase(&pool, plan_id, 1, &bare)
        .await
        .expect("upsert_phase 无章节阶段失败");
    assert_eq!(
        phase_status(&pool, plan_id, 1).await,
        Some("pending".into()),
        "无章节阶段不得翻牌 complete"
    );
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

/// 大纲骨架 fixture：phase_names 每项一个阶段，章节数 = chapter_counts 对应项
fn sample_outline(phase_names: &[&str], chapter_counts: &[usize]) -> Vec<OutlinePhasePayload> {
    phase_names
        .iter()
        .zip(chapter_counts.iter())
        .map(|(name, n)| OutlinePhasePayload {
            name: (*name).into(),
            week_range: "第 1-2 周".into(),
            chapter_titles: (0..*n).map(|i| format!("{name}·章{i}")).collect(),
            project_brief: None,
            ..Default::default()
        })
        .collect()
}

/// 大纲骨架配套资源 fixture
fn sample_resources() -> Vec<StudyResource> {
    vec![
        StudyResource {
            title: "The Rust Book".into(),
            kind: "docs".into(),
            note: "官方教程".into(),
            url: "https://doc.rust-lang.org/book/".into(),
            verification: "reachable".into(),
        },
        StudyResource {
            title: "Rust by Example".into(),
            kind: "docs".into(),
            note: "示例驱动".into(),
            url: "https://doc.rust-lang.org/rust-by-example/".into(),
            verification: "unverified".into(),
        },
    ]
}

/// 阶段状态（按 idx 取，供断言）
/// 计划行终态（结算口径改造后，upsert_chapter 会在全章 complete 时直接翻 complete）
async fn plan_status(pool: &PgPool, plan_id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .expect("查询计划状态")
        .flatten()
}

async fn phase_status(pool: &PgPool, plan_id: Uuid, idx: i32) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM phases WHERE plan_id = $1 AND idx = $2")
        .bind(plan_id)
        .bind(idx)
        .fetch_one(pool)
        .await
        .ok()
}

/// 章节状态（按 (plan, phase_idx, chapter_idx) 取）
async fn chapter_status(
    pool: &PgPool,
    plan_id: Uuid,
    phase_idx: i32,
    chapter_idx: i32,
) -> Option<String> {
    sqlx::query_scalar(
        "SELECT c.status FROM chapters c
         JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1 AND p.idx = $2 AND c.idx = $3",
    )
    .bind(plan_id)
    .bind(phase_idx)
    .bind(chapter_idx)
    .fetch_one(pool)
    .await
    .ok()
}

/// 章节正文（按 (plan, phase_idx, chapter_idx) 取，roundtrip 断言用）
async fn chapter_content_md(
    pool: &PgPool,
    plan_id: Uuid,
    phase_idx: i32,
    chapter_idx: i32,
) -> Option<String> {
    sqlx::query_scalar(
        "SELECT c.content_md FROM chapters c
         JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1 AND p.idx = $2 AND c.idx = $3",
    )
    .bind(plan_id)
    .bind(phase_idx)
    .bind(chapter_idx)
    .fetch_one(pool)
    .await
    .ok()
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

/// 同 (plan_id, idx) 两次 upsert：phases 行不翻倍，章节整体替换，阶段名更新，
/// markdown 正文跟随最新一次写入
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
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
    assert_eq!(
        count(&pool, "resources").await,
        0,
        "upsert 不写 resources（大纲播种专属）"
    );
    let (name, plan_fk): (String, Uuid) = sqlx::query_as("SELECT name, plan_id FROM phases")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "基础阶段v2", "同 idx 二次 upsert 应更新阶段名");
    assert_eq!(plan_fk, plan_id);
}

/// 回炉重写：upsert 覆盖后旧章节行与 project 不残留
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
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
    // 正文不残留：唯一章节的正文来自新章节
    assert!(chapter_content_md(&pool, plan_id, 1, 0)
        .await
        .unwrap()
        .contains("新章节甲"));
    // 项目行被替换（projects.phase_id UNIQUE，至多一行）
    let project: String = sqlx::query_scalar("SELECT name FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(project, "新阶段·项目");
}

/// 乱序扩展 idx 2,0,1：get_plan 多表重组后 phases 按 idx 升序、markdown 正文
/// 等值读回；update_outline_meta / finalize_plan 的元信息覆写生效
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
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
    plan_store::update_outline_meta(
        &pool,
        plan_id,
        "重组后的标题",
        Some("整体概述文本"),
        66,
        None,
    )
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
    // overview 落库修复（任务 22）：有值必写，列表页/详情页概述不再为空
    assert_eq!(
        record.overview, "整体概述文本",
        "大纲 overview 应随元信息一并落库"
    );
    assert_eq!(record.plan.estimated_total_hours, 66);
    // markdown 正文等值读回（含 mermaid 码块，逐阶段完整）
    for phase in &record.plan.phases {
        assert_eq!(phase.chapters.len(), 1);
        let chapter = &phase.chapters[0];
        assert_eq!(
            chapter.content_md,
            sample_chapter(&chapter.title).content_md,
            "read 回的 contentMd 必须等值于写入内容"
        );
        assert!(chapter.content_md.contains("```mermaid"));
        assert_eq!(chapter.status.as_deref(), Some("complete"));
    }
    // 项目重组回 PhaseProject
    assert_eq!(record.plan.phases[0].project.name, "阶段甲·项目");
    assert_eq!(
        record.plan.phases[0].project.steps,
        vec!["步骤甲".to_string(), "步骤乙".to_string()]
    );
    assert_eq!(record.plan.phases[0].project.estimated_hours, 6);

    // finalize：status/title/hours 覆写后读取仍一致
    plan_store::finalize_plan(
        &pool,
        plan_id,
        crate::plan_state::PlanStatus::Complete,
        Some("完结标题"),
        Some(77),
    )
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

/// 删 plans 行：五表（plans + 阶段/项目/章节/资源）全级联清空；
/// 四张内容子表已在 markdown 化中 DROP（表不存在即断言终态）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
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

    for table in ["plans", "phases", "projects", "chapters", "resources"] {
        assert_eq!(
            count(&pool, table).await,
            0,
            "删除 plans 后 {table} 应级联清空"
        );
    }
}

// ---------------------------------------------------------------------------
// 任务 13：两级状态机 + 大纲骨架 + 上下文读回
// ---------------------------------------------------------------------------

/// seed_outline：phases/chapters 建骨架行（status=pending，content_md=''），
/// resources 入库，project 无行
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_seed_outline_creates_skeleton(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(
        &pool,
        plan_id,
        &sample_outline(&["阶段甲", "阶段乙"], &[2, 1]),
        &sample_resources(),
    )
    .await
    .expect("seed_outline 失败");

    assert_eq!(count(&pool, "phases").await, 2, "骨架应建立 2 个阶段行");
    assert_eq!(count(&pool, "chapters").await, 3, "骨架应建立 2+1 个章节行");
    assert_eq!(count(&pool, "resources").await, 2);
    assert_eq!(count(&pool, "projects").await, 0, "大纲阶段不播种 project");
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("pending".into())
    );
    assert_eq!(
        phase_status(&pool, plan_id, 1).await,
        Some("pending".into())
    );
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("pending".into())
    );
    // 骨架章 content_md 列默认空串（DEFAULT '' 是有意义的空正文）
    assert_eq!(
        chapter_content_md(&pool, plan_id, 0, 0).await,
        Some(String::new())
    );
    // 章节标题携带大纲题目
    let titles: Vec<String> = sqlx::query_scalar(
        "SELECT c.title FROM chapters c JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1 AND p.idx = 0 ORDER BY c.idx",
    )
    .bind(plan_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        titles,
        vec!["阶段甲·章0".to_string(), "阶段甲·章1".to_string()]
    );
    // 读回：StudyPhase.status / Chapter.status 装入（骨架 pending），正文空串有效
    let record = db::get_plan_by_pool(&pool, &plan_id.to_string())
        .await
        .unwrap();
    assert_eq!(record.plan.phases[0].status.as_deref(), Some("pending"));
    assert_eq!(
        record.plan.phases[0].chapters[0].status.as_deref(),
        Some("pending")
    );
    assert_eq!(record.plan.phases[0].chapters[0].content_md, "");
}

/// seed_outline 幂等：二次调用不重复播种
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_seed_outline_idempotent(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    let outline = sample_outline(&["阶段甲"], &[2]);

    plan_store::seed_outline(&pool, plan_id, &outline, &sample_resources())
        .await
        .unwrap();
    plan_store::seed_outline(&pool, plan_id, &outline, &sample_resources())
        .await
        .expect("二次 seed 应幂等成功");

    assert_eq!(count(&pool, "phases").await, 1, "二次播种不得新增阶段");
    assert_eq!(count(&pool, "chapters").await, 2, "二次播种不得新增章节");
    assert_eq!(count(&pool, "resources").await, 2, "二次播种不得重复资源");
}

/// done 包络资源补写（任务 24，审查 Finding-1）：upsert_resources
/// DELETE+INSERT 幂等覆写（重发 done 不重复、可减可增），get_plan 回读有值
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_resources_overwrites_and_reads_back(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    // 无资源 -> 空写（幂等基线：重复调用不残留）
    plan_store::upsert_resources(&pool, plan_id, &[])
        .await
        .unwrap();
    plan_store::upsert_resources(&pool, plan_id, &sample_resources())
        .await
        .expect("upsert_resources 失败");
    assert_eq!(count(&pool, "resources").await, 2);

    // 重发 done（覆写）：数量可减不残留旧行
    plan_store::upsert_resources(&pool, plan_id, &sample_resources()[..1])
        .await
        .expect("二次覆写失败");
    assert_eq!(count(&pool, "resources").await, 1, "重发 done 不得重复资源");

    // get_plan 回读有值（结果页资源卡唯一内容源）
    let record = db::get_plan_by_pool(&pool, &plan_id.to_string())
        .await
        .expect("get_plan 失败");
    assert_eq!(record.plan.resources.len(), 1);
    assert_eq!(record.plan.resources[0].title, "The Rust Book");
    assert_eq!(record.plan.resources[0].kind, "docs");
}

/// 状态机主链：seed → upsert idx=0 → complete；失败收尾后 idx=1（含章节）变 failed，
/// idx=0（complete）不受影响——失败收尾只动 pending
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_marks_complete_and_failure_marks_pending_failed(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(
        &pool,
        plan_id,
        &sample_outline(&["阶段甲", "阶段乙"], &[2, 1]),
        &[],
    )
    .await
    .unwrap();
    plan_store::upsert_phase(
        &pool,
        plan_id,
        0,
        &sample_phase("阶段甲", &["甲章", "乙章"]),
    )
    .await
    .unwrap();

    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into())
    );
    // upsert 整体替换：idx=0 的 2 个骨架章被替换为内容章且 status=complete
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("complete".into())
    );
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 1).await,
        Some("complete".into())
    );
    assert_eq!(
        phase_status(&pool, plan_id, 1).await,
        Some("pending".into())
    );

    plan_store::finalize_plan(
        &pool,
        plan_id,
        crate::plan_state::PlanStatus::Failed,
        None,
        None,
    )
    .await
    .expect("finalize_plan 失败");

    assert_eq!(
        phase_status(&pool, plan_id, 1).await,
        Some("failed".into()),
        "pending 阶段应收尾为 failed"
    );
    assert_eq!(
        chapter_status(&pool, plan_id, 1, 0).await,
        Some("failed".into())
    );
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into()),
        "complete 不得被 failed 覆盖"
    );
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("complete".into())
    );
    let plans_status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(plans_status, "failed");
}

/// 空正文定价（任务 18 裁定）：upsert_chapter 写入空 contentMd → 章节行 failed、
/// 阶段不置 complete；补写正文后 complete + 阶段联动翻牌
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_chapter_empty_content_failed_then_complete(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();

    // 章 A：空正文入库 → failed（可重生成），阶段不推进
    let mut empty = sample_chapter("章A");
    empty.content_md = "   ".to_string();
    plan_store::upsert_chapter(&pool, plan_id, 0, 0, &empty)
        .await
        .expect("upsert_chapter 空正文失败");
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("failed".into()),
        "strip 后空正文必须翻 failed"
    );
    // 阶段状态是子章纯函数：有 failed 章 → failed。这里只锁「不得 complete」，
    // 不锁死 failed/pending 的具体取值（否则每次调整 CASE 分支都要改断言）。
    assert_ne!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into()),
        "存在 failed 章节，阶段不得置 complete"
    );

    // 章 B：正文完整 → complete，但章 A 仍 failed，阶段保持 pending
    plan_store::upsert_chapter(&pool, plan_id, 0, 1, &sample_chapter("章B"))
        .await
        .expect("upsert_chapter 章 B 失败");
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 1).await,
        Some("complete".into())
    );
    assert_ne!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into()),
        "failed 章节未收口前阶段不得翻牌"
    );
    assert_eq!(
        plan_status(&pool, plan_id).await,
        Some("generating".into()),
        "存在 failed 章节时计划不得提前结算为 complete"
    );

    // 重新生成章 A：正文补齐 → complete，两章全 complete，阶段联动翻牌
    plan_store::upsert_chapter(&pool, plan_id, 0, 0, &sample_chapter("章A"))
        .await
        .expect("upsert_chapter 章 A 重写失败");
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("complete".into())
    );
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into()),
        "全部章节 complete 后阶段应联动翻牌"
    );
    assert_eq!(
        plan_status(&pool, plan_id).await,
        Some("complete".into()),
        "全部章节 complete 后计划应直接结算为 complete（不等 done 事件）"
    );
}

/// 单章回炉：覆盖写后旧章内容不残留；complete 章不被失败收尾覆盖
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_chapter_replaces_content_without_residue(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    plan_store::upsert_chapter(&pool, plan_id, 0, 0, &sample_chapter("章A"))
        .await
        .expect("upsert_chapter 失败");

    // 同位回炉：旧正文整体被新正文替换（唯一章节，contentMd 等值）
    plan_store::upsert_chapter(&pool, plan_id, 0, 0, &sample_chapter("章A-v2"))
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "chapters").await,
        1,
        "同 (phase_id, idx) 回炉不得新增章节行"
    );
    let content = chapter_content_md(&pool, plan_id, 0, 0).await.unwrap();
    assert!(content.contains("章A-v2"), "正文应更新为回炉后的版本");
    assert!(!content.contains("## 章A\n"), "旧章开头条目不残留");
    assert_eq!(
        chapter_status(&pool, plan_id, 0, 0).await,
        Some("complete".into())
    );

    // status 机不变量：覆盖回炉后再 seed 不会重复播种（幂等守卫）
    let outline = sample_outline(&["阶段甲"], &[1]);
    plan_store::seed_outline(&pool, plan_id, &outline, &[])
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "chapters").await,
        1,
        "幂等守卫下骨架 seed 不得覆盖已回炉的章节数据"
    );
}

/// 重生成上下文读回：create_generating_plan 写入的七字段经 load_plan_request_meta 一致读回
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_plan_request_meta_roundtrip(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    let row = plan_store::load_plan_request_meta(&pool, plan_id)
        .await
        .expect("查询失败")
        .expect("generating 行应存在");
    assert_eq!(
        row,
        PlanRequestRow {
            goal: "掌握 Rust 系统编程".into(),
            level: "beginner".into(),
            weeks: 8,
            weekly_hours: 10,
            interests: vec!["cli".into()],
            background: "有 C 基础".into(),
            // study_days 请求侧 None = 全周，落库为列默认 '[1..7]'，读回恒为数组
            study_days: Some(vec![1, 2, 3, 4, 5, 6, 7]),
        },
        "七字段 + studyDays 应与原请求逐字段一致（None 归一全周）"
    );

    // 不存在的 id：None
    let missing = plan_store::load_plan_request_meta(&pool, Uuid::new_v4())
        .await
        .unwrap();
    assert!(missing.is_none());
}

// ---------------------------------------------------------------------------
// Task E：每周学习日列 + 当日已学打卡表
// ---------------------------------------------------------------------------

/// studyDays 落库读回（需求 5）：请求侧 None → 列默认全周；set_study_days
/// 归一（去重排序）后 UPDATE，读回一致；重生成上下文同步可见
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_study_days_roundtrip(pool: PgPool) {
    prepare_schema(&pool).await;
    // 默认路径：sample_request.study_days = None → create_generating_plan 写全周
    let plan_id = new_generating_plan(&pool).await;
    let meta = plan_store::load_plan_request_meta(&pool, plan_id)
        .await
        .expect("查询失败")
        .expect("generating 行应存在");
    assert_eq!(
        meta.study_days,
        Some(vec![1, 2, 3, 4, 5, 6, 7]),
        "None 归一全周落库"
    );

    // 指定路径：直接 UPDATE 学习日（set_study_days 已由 start_learning/
    // update_learning_days 取代；归一校验由 schedule::normalize_days 单测覆盖）
    sqlx::query("UPDATE plans SET study_days = $2 WHERE id = $1")
        .bind(plan_id)
        .bind(sqlx::types::Json(&vec![1u8, 3, 5]))
        .execute(&pool)
        .await
        .expect("更新学习日失败");
    let meta = plan_store::load_plan_request_meta(&pool, plan_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.study_days, Some(vec![1, 3, 5]));
}

/// get_plan / list_plans wire roundtrip：studyDays 进入 PlanRecord 与
/// PlanSummary 列表行；学习状态/开始日期/章节计数随列表行携带（打卡日历真数据）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_plan_wire_carries_study_days_and_marks(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    sqlx::query("UPDATE plans SET study_days = $2 WHERE id = $1")
        .bind(plan_id)
        .bind(sqlx::types::Json(&vec![2u8, 4, 6]))
        .execute(&pool)
        .await
        .unwrap();

    let record = db::get_plan_by_pool(&pool, &plan_id.to_string())
        .await
        .unwrap();
    assert_eq!(record.study_days, vec![2, 4, 6], "studyDays wire roundtrip");

    let summaries = db::list_plans_by_pool(&pool, "").await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].study_days,
        vec![2, 4, 6],
        "列表行携带 studyDays"
    );
    assert_eq!(
        summaries[0].learning_status, "not_started",
        "学习状态默认未开始"
    );
    assert_eq!(summaries[0].learning_started_at, None, "未开始无开始日期");
    assert_eq!(summaries[0].chapter_total, 0, "无骨架计划章节计数为 0");
    assert_eq!(summaries[0].chapter_done, 0);
}

/// PlanOutline 骨架读回：phases 按 idx 升序 + chapterTitles 顺序与播种一致（任务 15 依赖）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_plan_outline_roundtrip(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(
        &pool,
        plan_id,
        &sample_outline(&["阶段甲", "阶段乙"], &[2, 1]),
        &[],
    )
    .await
    .unwrap();

    let outline = plan_store::load_plan_outline(&pool, plan_id)
        .await
        .expect("查询失败")
        .expect("generating 行应存在");
    assert_eq!(
        outline,
        PlanOutlineRow {
            title: "掌握 Rust 系统编程".into(), // title 暂为 goal 截断占位
            overview: String::new(),
            estimated_total_hours: 0,
            phases: sample_outline(&["阶段甲", "阶段乙"], &[2, 1]),
        }
    );
    // 不存在的 id：None
    assert!(plan_store::load_plan_outline(&pool, Uuid::new_v4())
        .await
        .unwrap()
        .is_none());
}

/// 协议安全（审查重点 2）：ensure_schema 在「历史建过四子表的 DB」与「全新 DB」
/// 两形态下都收敛到同一终态——四张退役子表不存在，chapters 保留且新增 content_md 列
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_ensure_schema_converges_child_tables_dropped(pool: PgPool) {
    prepare_schema(&pool).await;

    // 造历史形态：手动重建四张退役子表（含外键，模拟升级前遗留库）
    for ddl in [
        "CREATE TABLE concepts (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE, idx INT NOT NULL, name TEXT NOT NULL, definition TEXT NOT NULL DEFAULT '')",
        "CREATE TABLE knowledge_points (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE, idx INT NOT NULL, title TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '')",
        "CREATE TABLE diagrams (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE, idx INT NOT NULL, title TEXT NOT NULL DEFAULT '', mermaid TEXT NOT NULL DEFAULT '', caption TEXT NOT NULL DEFAULT '')",
        "CREATE TABLE deep_dives (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE, idx INT NOT NULL, topic TEXT NOT NULL, angle TEXT NOT NULL DEFAULT '', content TEXT NOT NULL)",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
    }
    assert_eq!(count(&pool, "concepts").await, 0);

    // 重跑首连 DDL：与全新库一样收敛——四张子表 DROP，chapters 附上了 content_md
    prepare_schema(&pool).await;
    for table in ["concepts", "knowledge_points", "diagrams", "deep_dives"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!exists, "退役子表 {table} 必须被 DROP 收敛");
    }
    // chapters.content_md 列在位且 NOT NULL DEFAULT ''（全新/历史两形态共用）
    let md_col: (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='chapters' AND column_name='content_md'), \
         (SELECT is_nullable = 'NO' FROM information_schema.columns WHERE table_name='chapters' AND column_name='content_md'), \
         EXISTS(SELECT 1 FROM chapters)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(md_col.0, "chapters.content_md 列必须在位");
    assert!(md_col.1, "content_md 必须 NOT NULL");
    assert!(!md_col.2, "空库无章节行");
    // chapters 的 UNIQUE(phase_id, idx) / FK(cascade) 不变：写入仍受约束
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    assert_eq!(count(&pool, "chapters").await, 1);
}

// -----------------------------------------------------------------------
// 任务 22：收尾语义修订（2026-10-16 章级定案：全章生成成功 → complete；
// 任一失败章/滞留骨架/零章行 → failed）与启动清扫
// -----------------------------------------------------------------------

/// 收尾语义修订：done(error) 但全章生成成功且至少一章 → plans 置 complete
/// （观测丢失不吞已入库内容）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_settle_terminal_error_with_phase_rows_completes(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("阶段甲", &["甲章"]))
        .await
        .unwrap();
    assert!(plan_store::all_chapters_complete(&pool, plan_id)
        .await
        .unwrap());

    commands::settle_terminal_with_hours(&pool, plan_id, "error", None)
        .await
        .expect("settle_terminal 失败");
    let status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "complete",
        "有阶段内容的 error 终态应翻 complete（可重生成/重看）"
    );
    assert!(
        matches!(
            plan_store::all_chapters_complete(&pool, plan_id).await,
            Ok(true)
        ),
        "全章成功应保留成立（存在性为真，非仅查询成功）"
    );
}

/// 纯骨架收尾：只播种 outline 骨架（章行全部 pending、零内容）→
/// done(error) 不得被骨架行短路成 complete，应翻 failed
/// （2026-10-16 章级定案的回归钉：老语义 any-rows → complete 的历史缺陷）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_settle_terminal_skeleton_only_fails(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();

    commands::settle_terminal_with_hours(&pool, plan_id, "error", None)
        .await
        .expect("settle_terminal 失败");
    let status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "failed",
        "纯骨架 pending 章不得成立为 complete，应翻 failed"
    );
}

/// 含失败章收尾：任一章生成失败 → 总状态 failed（部分成功不算成功，
/// 失败部件保留内容可从详情页重生成）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_settle_terminal_failed_chapter_fails_total(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("阶段甲", &["甲章"]))
        .await
        .unwrap();
    // 模拟一醒章生成失败（其余章仍 complete）
    sqlx::query("UPDATE chapters SET status = 'failed' WHERE idx = 0")
        .execute(&pool)
        .await
        .unwrap();

    commands::settle_terminal_with_hours(&pool, plan_id, "plan", None)
        .await
        .expect("settle_terminal 失败");
    let status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "failed",
        "存在失败章时总状态必须为 failed（部分成功不算成功）"
    );
}

/// 零内容收尾：未播种也未扩展任何阶段 → plans 置 failed（plan/error 终态同口径；
/// clarification 仍删 generating 行，既有三态语义防回归）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_settle_terminal_zero_content_fails(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    // 终态为 plan 但零内容：同样按失败收尾（宁可失败态可删，不留假 complete）
    commands::settle_terminal_with_hours(&pool, plan_id, "plan", None)
        .await
        .expect("settle_terminal 失败");
    let status: String = sqlx::query_scalar("SELECT status FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "failed", "零内容的 plan 终态应翻 failed");
}

/// clarification 终态：删 generating 行（既有语义防回归）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_settle_terminal_clarification_deletes_row(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    commands::settle_terminal_with_hours(&pool, plan_id, "clarification", None)
        .await
        .expect("settle_terminal 失败");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "clarification 终态应删除 generating 行");
}

/// 启动清扫：status='generating' 存量行翻 failed + pending 子行翻 failed，
/// complete 行不受影响；二次清扫恒为空集（幂等）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_sweep_stale_generating_only_touches_generating(pool: PgPool) {
    prepare_schema(&pool).await;
    let stale_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, stale_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();

    // 对照行：complete 状态不受清扫影响
    let done_id = new_generating_plan(&pool).await;
    sqlx::query("UPDATE plans SET status = 'complete' WHERE id = $1")
        .bind(done_id)
        .execute(&pool)
        .await
        .unwrap();

    let report = plan_store::sweep_stale_generating(&pool).await.unwrap();
    assert_eq!(report.scanned_plans, 1, "只有 generating 计划可疑");
    assert_eq!(
        report.healed_plans, 0,
        "本例无全章 complete 的 failed 行，不应自愈"
    );

    let stale_status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(stale_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stale_status, "failed");
    // pending 骨架子行一并置 failed（complete 子行不动）
    let stale_phase: String = sqlx::query_scalar("SELECT status FROM phases LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stale_phase, "failed");

    let done_status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(done_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(done_status, "complete", "complete 行不应被清扫触碰");

    // 幂等：二次清扫无存量可清
    let again = plan_store::sweep_stale_generating(&pool).await.unwrap();
    assert!(
        again.is_clean(),
        "自愈应幂等（二次调用无修复量），实际 {again:?}"
    );
}

/// regen 收口（章级口径，2026-10-16 修复回归）：failed 行 + 全章节 complete
/// → finalize_regen_complete 翻回 complete（旧守卫语义会永久卡 failed）
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_finalize_regen_complete_heals_failed_row(pool: PgPool) {
    prepare_schema(&pool).await;
    let id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    // 模拟历史失败收口后的重生成补齐：行 failed、全章节 complete
    sqlx::query("UPDATE plans SET status = 'failed' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE chapters SET status = 'complete'
         WHERE phase_id IN (SELECT id FROM phases WHERE plan_id = $1)",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();

    plan_store::finalize_regen_complete(&pool, id)
        .await
        .unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "complete",
        "failed 行在全章 complete 后应被收口翻正"
    );
}

/// regen 收口（章级口径）：尚有非 complete 章（pending）时保持 failed 不动
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_finalize_regen_complete_keeps_failed_with_pending(pool: PgPool) {
    prepare_schema(&pool).await;
    let id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();
    sqlx::query("UPDATE plans SET status = 'failed' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    // 只补齐第一章，第二章仍 pending
    sqlx::query(
        "UPDATE chapters SET status = 'complete' WHERE id IN (
             SELECT c.id FROM chapters c JOIN phases p ON c.phase_id = p.id
             WHERE p.plan_id = $1 ORDER BY c.idx LIMIT 1)",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();

    plan_store::finalize_regen_complete(&pool, id)
        .await
        .unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "failed", "尚有 pending 章时收口不应翻转");
}

/// 启动自愈（2026-10-16 裁定）：failed 且全章节 complete 的行翻 complete；
/// 真 failed 行（章未齐）保持 failed；complete/generating 对照不受影响
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_sweep_heals_failed_with_all_chapters_complete(pool: PgPool) {
    prepare_schema(&pool).await;

    // 应自愈行：failed + 全章 complete
    let heal_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, heal_id, &sample_outline(&["阶段甲"], &[1]), &[])
        .await
        .unwrap();
    sqlx::query("UPDATE plans SET status = 'failed' WHERE id = $1")
        .bind(heal_id)
        .execute(&pool)
        .await
        .unwrap();
    // complete 与正文必须一致：自愈按 content_md 反推状态，只改 status 而不写
    // 正文会造出「complete 却无正文」的矛盾行，被自愈正确地改回 failed。
    sqlx::query(
        "UPDATE chapters SET status = 'complete', content_md = '已完成的正文'
         WHERE phase_id IN (SELECT id FROM phases WHERE plan_id = $1)",
    )
    .bind(heal_id)
    .execute(&pool)
    .await
    .unwrap();

    // 真失败对照行：failed + 章 pending（保持 failed）
    let keep_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, keep_id, &sample_outline(&["阶段乙"], &[1]), &[])
        .await
        .unwrap();
    sqlx::query("UPDATE plans SET status = 'failed' WHERE id = $1")
        .bind(keep_id)
        .execute(&pool)
        .await
        .unwrap();

    let report = plan_store::sweep_stale_generating(&pool).await.unwrap();
    assert_eq!(report.scanned_plans, 2, "两个 failed 计划都在候选集内");
    assert_eq!(report.healed_plans, 1, "只应自愈全章 complete 的 failed 行");

    let heal_status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(heal_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(heal_status, "complete", "自愈行应翻 complete");

    let keep_status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(keep_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(keep_status, "failed", "真失败行应保持 failed");
}

/// 崩溃自愈（2026-10-22）：重生成开始时 `mark_phase_generating` /
/// `mark_chapter_generating` 会把阶段与章节置成 generating，但**不改
/// `plans.status`**（计划整体仍是 failed / complete）。进程崩溃后这些子行再也
/// 无人收尾，前端按 `chapter.status='generating'` 一直显示「生成中」。
///
/// 自愈口径是「候选计划 + 自下而上反推」：该计划 status=failed 进入候选集，
/// 然后 chapters 按正文反推（有正文 → complete，空 → failed），phases 由子章
/// 聚合，plans 由全章聚合。二次执行应完全幂等。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_sweep_heals_generating_children_when_plan_not_generating(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();

    let phase_id: Uuid = sqlx::query_scalar("SELECT id FROM phases WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    // 造崩溃现场：章 0 已有正文但状态停在 generating（写完未落状态就被杀），
    // 章 1 空正文停在 generating，阶段停在 generating，而计划是 failed。
    for (idx, md) in [(0i32, "已有正文"), (1, "")] {
        sqlx::query(
            "UPDATE chapters SET status = 'generating', content_md = $3
             WHERE phase_id = $1 AND idx = $2",
        )
        .bind(phase_id)
        .bind(idx)
        .bind(md)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE phases SET status = 'generating' WHERE id = $1")
        .bind(phase_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE plans SET status = 'failed' WHERE id = $1")
        .bind(plan_id)
        .execute(&pool)
        .await
        .unwrap();

    let report = plan_store::sweep_stale_generating(&pool).await.unwrap();
    assert_eq!(report.scanned_plans, 1, "该 failed 计划进入候选集");
    assert!(
        report.healed_children >= 2,
        "章与阶段的残留都应被反推修正，实际 {}",
        report.healed_children
    );

    let c0: String =
        sqlx::query_scalar("SELECT status FROM chapters WHERE phase_id = $1 AND idx = 0")
            .bind(phase_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(c0, "complete", "有正文的残留章应判 complete");
    let c1: String =
        sqlx::query_scalar("SELECT status FROM chapters WHERE phase_id = $1 AND idx = 1")
            .bind(phase_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(c1, "failed", "空正文的残留章应判 failed");

    let phase_status: String = sqlx::query_scalar("SELECT status FROM phases WHERE id = $1")
        .bind(phase_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(phase_status, "failed", "阶段应随子章回落 failed");

    let plan_status: String = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(plan_status, "failed", "仍有空章，计划保持 failed");

    let again = plan_store::sweep_stale_generating(&pool).await.unwrap();
    assert_eq!(again.healed_children, 0, "二次自愈不应再有修复量");
}

/// 叶子层配对测试（2026-10-22）：章节「正文 → 状态」的 SQL 翻译
/// （`SQL_CHAPTER_STATUS_FROM_CONTENT`）必须与纯函数 `chapter_of_content` 同语义。
/// 自愈的整棵状态树都建立在这一层之上，这里一旦漂移，上面的 phases / plans
/// 反推会全部跟着错。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn sql_chapter_status_matches_pure_function(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[4]), &[])
        .await
        .unwrap();
    let phase_id: Uuid = sqlx::query_scalar("SELECT id FROM phases WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let cases = ["", "   ", "正文", "\n\t 带空白的内容 "];
    for (idx, md) in cases.iter().enumerate() {
        sqlx::query("UPDATE chapters SET content_md = $3, status = 'pending' WHERE phase_id = $1 AND idx = $2")
            .bind(phase_id)
            .bind(idx as i32)
            .bind(md)
            .execute(&pool)
            .await
            .unwrap();
        let sql_status: String = sqlx::query_scalar(&format!(
            "UPDATE chapters c SET status = {from_content} WHERE c.phase_id = $1 AND c.idx = $2 RETURNING c.status",
            from_content = crate::plan_state::SQL_CHAPTER_STATUS_FROM_CONTENT
        ))
        .bind(phase_id)
        .bind(idx as i32)
        .fetch_one(&pool)
        .await
        .unwrap();
        let pure = crate::plan_state::chapter_of_content(md);
        assert_eq!(
            sql_status,
            pure.as_str(),
            "content_md={md:?} 时 SQL 与纯函数不一致",
        );
    }
}

/// 重生成上下文（2026-10-22）：outline 事件携带的完整大纲要能原样存回、读回。
/// 这是 regen 与首次生成上下文一致的前提——丢了它，重生成会把章型兜底成
/// concept、只拿章标题当检索词、并告诉模型「未探测到版本信息」。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_outline_json_roundtrip_and_preferred_on_load(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();

    // 未写 outline_json 时：回退按行重建（骨架形态，无知识点/章型）
    let fallback = plan_store::load_plan_outline(&pool, plan_id)
        .await
        .unwrap()
        .expect("generating 行应存在");
    assert!(
        fallback.phases[0].chapter_knowledge_points.is_empty(),
        "骨架重建没有知识点"
    );

    // 写入完整大纲后：优先读它，章型/知识点/阶段目标/版本事实齐备
    let outline_json = serde_json::json!({
        "title": "重组后的标题",
        "overview": "整体概述文本",
        "estimatedTotalHours": 66,
        "phases": [{
            "name": "阶段甲",
            "weekRange": "第 1-2 周",
            "chapterTitles": ["阶段甲·章0", "阶段甲·章1"],
            "chapterKinds": ["concept", "practice"],
            "chapterKnowledgePoints": [["知识点A"], ["知识点B", "知识点C"]],
            "objectives": ["目标一"],
            "projectBrief": "做一个项目",
            "techFacts": [{"name": "tokio", "ecosystem": "crate", "latestVersion": "1.0.0"}]
        }]
    });
    plan_store::update_outline_meta(
        &pool,
        plan_id,
        "重组后的标题",
        Some("整体概述文本"),
        66,
        Some(&outline_json),
    )
    .await
    .unwrap();

    let loaded = plan_store::load_plan_outline(&pool, plan_id)
        .await
        .unwrap()
        .expect("应返回大纲");
    assert_eq!(loaded.estimated_total_hours, 66);
    let phase = &loaded.phases[0];
    assert_eq!(phase.chapter_kinds, vec!["concept", "practice"]);
    assert_eq!(
        phase.chapter_knowledge_points[1],
        vec!["知识点B", "知识点C"]
    );
    assert_eq!(phase.objectives, vec!["目标一"]);
    assert_eq!(phase.tech_facts.len(), 1);
    assert_eq!(phase.tech_facts[0]["latestVersion"], "1.0.0");

    // 防抹除：再次不带 outline_json 的元信息更新不应清掉已存大纲
    plan_store::update_outline_meta(&pool, plan_id, "标题二", None, 70, None)
        .await
        .unwrap();
    let again = plan_store::load_plan_outline(&pool, plan_id)
        .await
        .unwrap()
        .expect("应返回大纲");
    assert_eq!(
        again.phases[0].chapter_kinds.len(),
        2,
        "旧大纲不应被抹成 NULL"
    );
}

/// 生成留档（2026-10-22）：素材与提示词快照按 (计划, 阶段, 章, 类别) 覆盖写，
/// 重生成时经 load_plan_outline 读回为与章标题等长的并行数组；保留期清理
/// **只清已完成计划**——失败计划随时会被重新生成，那时正需要留档。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_generation_artifacts_roundtrip_and_purge(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(&pool, plan_id, &sample_outline(&["阶段甲"], &[2]), &[])
        .await
        .unwrap();

    let research = vec![serde_json::json!({
        "title": "装饰器 - Python 文档",
        "url": "https://docs.python.org/3/glossary.html",
        "snippet": "装饰器是接受函数并返回函数的可调用对象。"
    })];
    let trace = serde_json::json!({
        "promptKey": "chapter",
        "variables": {"chapter_title": "阶段甲·章0"}
    });
    plan_store::upsert_chapter_artifacts(&pool, plan_id, 0, 0, &research, Some(&trace))
        .await
        .unwrap();
    // 覆盖写：同键再写一次只更新，不追加（留档不会随重生成膨胀）
    plan_store::upsert_chapter_artifacts(&pool, plan_id, 0, 0, &research, None)
        .await
        .unwrap();

    let outline = plan_store::load_plan_outline(&pool, plan_id)
        .await
        .unwrap()
        .expect("应返回大纲");
    let groups = &outline.phases[0].chapter_research_snippets;
    assert_eq!(groups.len(), 2, "并行数组与章标题等长");
    assert_eq!(groups[0].len(), 1, "首章素材被读回");
    assert!(groups[1].is_empty(), "未生成过的章没有素材（照常联网检索）");

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM generation_artifacts WHERE plan_id = $1")
            .bind(plan_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 2, "research + prompt 各一行，覆盖写不新增");

    // 生成中的计划不清理：它随时可能被重新生成，正需要留档
    assert_eq!(
        plan_store::purge_stale_artifacts(&pool, -1).await.unwrap(),
        0,
        "generating 计划不参与清理"
    );

    // 只改状态：title / 学时传 None 表示沿用列上现值
    plan_store::finalize_plan(
        &pool,
        plan_id,
        crate::plan_state::PlanStatus::Complete,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        plan_store::purge_stale_artifacts(&pool, -1).await.unwrap(),
        2,
        "已完成且超保留期的计划整体清掉留档"
    );
    assert_eq!(
        plan_store::purge_stale_artifacts(&pool, -1).await.unwrap(),
        0,
        "重复清理幂等"
    );
}

/// 结算口径回归（2026-10-22）：计划终态按「全章 complete」判定，与 done 事件解耦。
/// 关键防线——单个阶段写完不等于计划完成：另一阶段还停着 pending 骨架章时，
/// upsert_chapter 不得把计划翻成 complete（否则会出现「内容不全却显示已完成」）。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn test_upsert_chapter_keeps_plan_generating_until_every_phase_complete(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;

    plan_store::seed_outline(
        &pool,
        plan_id,
        &sample_outline(&["阶段甲", "阶段乙"], &[1, 1]),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        plan_status(&pool, plan_id).await,
        Some("generating".into()),
        "仅有骨架章时计划保持 generating"
    );

    // 阶段甲写完：甲全 complete，但乙仍是 pending 骨架 → 计划不得翻牌
    plan_store::upsert_chapter(&pool, plan_id, 0, 0, &sample_chapter("甲章"))
        .await
        .expect("upsert_chapter 甲章失败");
    assert_eq!(
        phase_status(&pool, plan_id, 0).await,
        Some("complete".into())
    );
    assert_eq!(
        plan_status(&pool, plan_id).await,
        Some("generating".into()),
        "另一阶段仍为 pending 骨架时，计划不得被判为完成"
    );

    // 阶段乙写完 → 全计划章 complete → 计划直接结算
    plan_store::upsert_chapter(&pool, plan_id, 1, 0, &sample_chapter("乙章"))
        .await
        .expect("upsert_chapter 乙章失败");
    assert_eq!(
        plan_status(&pool, plan_id).await,
        Some("complete".into()),
        "最后一个待写章节落地后计划应结算为 complete"
    );
}

/// 状态机配对测试（2026-10-22 架构收敛）。
///
/// 状态的语义定义在 `plan_state`（枚举 + 纯函数），但热路径上的推进是 SQL
/// 常量（`SQL_PHASE_STATUS_CASE`）。两套实现必须同语义，否则又会回到
/// 「同一状态两处判定」的老路。本用例用真实的 phases/chapters 行构造各种
/// 章节状态组合，分别跑 SQL 与纯函数并断言结果一致。
#[sqlx::test]
#[ignore = "需要 config.json database.testUrl（见文件头运行方式），无库环境下跳过"]
async fn sql_phase_status_matches_pure_function(pool: PgPool) {
    use crate::plan_state::{self, ChapterStatus};

    // 每组是被预设到章节行上的状态集合；空集合代表「阶段下没有章节」
    let cases: Vec<Vec<ChapterStatus>> = vec![
        vec![],
        vec![ChapterStatus::Pending],
        vec![ChapterStatus::Complete],
        vec![ChapterStatus::Complete, ChapterStatus::Complete],
        vec![ChapterStatus::Complete, ChapterStatus::Failed],
        vec![ChapterStatus::Failed, ChapterStatus::Pending],
        vec![ChapterStatus::Complete, ChapterStatus::Pending],
        vec![ChapterStatus::Generating, ChapterStatus::Complete],
        vec![ChapterStatus::Generating, ChapterStatus::Failed],
    ];

    for (case_index, statuses) in cases.iter().enumerate() {
        prepare_schema(&pool).await;
        let plan_id = new_generating_plan(&pool).await;
        let chapter_count = statuses.len().max(1);
        plan_store::seed_outline(
            &pool,
            plan_id,
            &sample_outline(&["配对阶段"], &[chapter_count]),
            &[],
        )
        .await
        .unwrap();

        // 清空骨架章节后按用例重造（空集合用例保持零章节）
        sqlx::query(
            "DELETE FROM chapters c USING phases p WHERE c.phase_id = p.id AND p.plan_id = $1",
        )
        .bind(plan_id)
        .execute(&pool)
        .await
        .unwrap();
        for (idx, status) in statuses.iter().enumerate() {
            sqlx::query(
                "INSERT INTO chapters (phase_id, idx, title, summary, content_md, status)
                 SELECT p.id, $2, '章', '', '正文', $3 FROM phases p
                 WHERE p.plan_id = $1 AND p.idx = 0",
            )
            .bind(plan_id)
            .bind(idx as i32)
            .bind(status.as_str())
            .execute(&pool)
            .await
            .unwrap();
        }

        // SQL 版：跑与 upsert_chapter 相同的 UPDATE 文本
        let sql = format!(
            "UPDATE phases SET status = {} WHERE plan_id = $1 AND idx = 0",
            plan_state::SQL_PHASE_STATUS_CASE
        );
        sqlx::query(&sql)
            .bind(plan_id)
            .execute(&pool)
            .await
            .unwrap();
        let from_sql = phase_status(&pool, plan_id, 0).await;

        // 纯函数版
        let expected = plan_state::phase_of_chapters(statuses).as_str().to_string();

        assert_eq!(
            from_sql,
            Some(expected.clone()),
            "第 {case_index} 组（{statuses:?}）：SQL 与 phase_of_chapters 不一致"
        );

        // 计划级配对：all_chapters_complete 已走纯函数，这里锁住它与
        // plan_of_chapters 的等价关系（防止将来有人改回 SQL 而口径漂移）
        let all_complete = plan_store::all_chapters_complete(&pool, plan_id)
            .await
            .unwrap();
        let expected_plan_complete =
            plan_state::plan_of_chapters(statuses) == plan_state::PlanStatus::Complete;
        assert_eq!(
            all_complete, expected_plan_complete,
            "第 {case_index} 组（{statuses:?}）：all_chapters_complete 与 plan_of_chapters 不一致"
        );
    }
}

/// 整计划重生成的重置：旧阶段/章节/素材/日程/资源全部清空，计划行回到
/// generating 且大纲快照与学时归零，学习状态退回未开始。
///
/// 这条口径的存在理由：整计划重生成产出的是**全新大纲**，任何按旧索引
/// 保存的东西（章节行、留档素材、学习日程）都会污染新一次生成。
#[sqlx::test]
#[ignore = "需要真实 PostgreSQL（cargo test -- --ignored）"]
async fn test_reset_plan_for_regeneration_clears_derived_rows(pool: PgPool) {
    prepare_schema(&pool).await;
    let plan_id = new_generating_plan(&pool).await;
    plan_store::seed_outline(
        &pool,
        plan_id,
        &sample_outline(&["阶段甲"], &[2]),
        &sample_resources(),
    )
    .await
    .unwrap();
    plan_store::upsert_phase(&pool, plan_id, 0, &sample_phase("阶段甲", &["章A", "章B"]))
        .await
        .unwrap();
    plan_store::upsert_chapter_artifacts(
        &pool,
        plan_id,
        0,
        0,
        &[],
        Some(&serde_json::json!({"k": 1})),
    )
    .await
    .unwrap();
    // 日程用原生 SQL 插入：本用例只关心「重置会清掉它」，不需要 schedule
    // 模块的分配语义（那属于别的用例）
    sqlx::query(
        "INSERT INTO study_schedule (plan_id, date, phase_idx, chapter_idx, chapter_title)
         VALUES ($1, DATE '2026-10-22', 0, 0, '章A')",
    )
    .bind(plan_id)
    .execute(&pool)
    .await
    .unwrap();

    // 前置确认：重置前确实有内容
    let count = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(plan_id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert!(count("SELECT count(*) FROM phases WHERE plan_id = $1").await > 0);
    assert!(count("SELECT count(*) FROM chapters c JOIN phases p ON c.phase_id = p.id WHERE p.plan_id = $1").await > 0);
    assert!(count("SELECT count(*) FROM generation_artifacts WHERE plan_id = $1").await > 0);
    assert!(count("SELECT count(*) FROM resources WHERE plan_id = $1").await > 0);
    assert!(count("SELECT count(*) FROM study_schedule WHERE plan_id = $1").await > 0);

    plan_store::reset_plan_for_regeneration(&pool, plan_id)
        .await
        .unwrap();

    for sql in [
        "SELECT count(*) FROM phases WHERE plan_id = $1",
        "SELECT count(*) FROM chapters c JOIN phases p ON c.phase_id = p.id WHERE p.plan_id = $1",
        "SELECT count(*) FROM generation_artifacts WHERE plan_id = $1",
        "SELECT count(*) FROM resources WHERE plan_id = $1",
        "SELECT count(*) FROM study_schedule WHERE plan_id = $1",
    ] {
        assert_eq!(count(sql).await, 0, "重置后仍有残留：{sql}");
    }

    let (status, learning, outline, hours) =
        // estimated_total_hours 在库里是 INT4，按 i32 解码
        sqlx::query_as::<_, (String, String, Option<serde_json::Value>, i32)>(
            "SELECT status, learning_status, outline_json, estimated_total_hours
             FROM plans WHERE id = $1",
        )
        .bind(plan_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, plan_state::PlanStatus::Generating.as_str());
    assert_eq!(learning, plan_state::LearningStatus::NotStarted.as_str());
    assert!(
        outline.is_none(),
        "旧大纲快照必须清空（否则重生成会读回旧结构）"
    );
    assert_eq!(hours, 0, "旧学时归零（新大纲事件会重新写入）");

    // 幂等：再重置一次不报错、仍是零内容
    plan_store::reset_plan_for_regeneration(&pool, plan_id)
        .await
        .unwrap();
    assert_eq!(
        count("SELECT count(*) FROM phases WHERE plan_id = $1").await,
        0
    );
}
