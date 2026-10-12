//! 云端 PostgreSQL 持久化：学习计划历史（云端唯一真源，无本地库无同步）。
//!
//! - 惰性连接池：首次使用才建；配置指纹（host/port/database/user）变更时自动重建，
//!   设置页改完配置立即生效，无需重启应用；
//! - 首连自动建表（CREATE TABLE IF NOT EXISTS），零手工 DDL 步骤；
//! - 每次连接/查询带超时，网络故障快速失败并归一为 AgentError::Db。

use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::DbConfig;
use crate::error::AgentError;
use crate::protocol::{PlanRecord, PlanSummary, SavePlanRequest};

/// 单次连接/查询超时
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// 连接池上限（个人低频读写场景，小池足够）
const POOL_MAX: u32 = 4;
/// 列表单次上限（防御性：个人历史量级远达不到）
const LIST_LIMIT: i64 = 500;

struct PoolEntry {
    /// 配置指纹：host:port:database:user（password 不参与——变更密码时重建也无妨，
    /// 但指纹里不放避免日志/比较泄露）
    key: String,
    pool: PgPool,
}

/// 全局连接池：配置变更时整体替换（旧池随 Arc 释放）
static POOL: Mutex<Option<PoolEntry>> = Mutex::const_new(None);

/// 获取（或按配置重建）连接池；未配置时返回友好错误。
pub async fn pool(config: &DbConfig) -> Result<PgPool, AgentError> {
    if config.host.trim().is_empty() {
        return Err(AgentError::Db(
            "云端数据库未配置：请先在设置页填写并保存连接信息".into(),
        ));
    }
    let key = format!(
        "{}:{}:{}:{}",
        config.host, config.port, config.database, config.user
    );

    // 快路径：指纹匹配则复用（PgPool 内部是 Arc，clone 廉价）
    {
        let guard = POOL.lock().await;
        if let Some(entry) = guard.as_ref() {
            if entry.key == key {
                return Ok(entry.pool.clone());
            }
        }
    }

    // 慢路径：建池与建表不持锁，避免阻塞其他 command 的快路径检查
    let options = PgConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .database(&config.database)
        .username(&config.user)
        .password(&config.password)
        // v1 不启用 TLS（自签证书配置繁琐）；公网安全靠强密码 + 专用最小权限账号。
        // statement_timeout：服务端掊断超时查询，防止黑洞查询长期占用池内连接
        .ssl_mode(PgSslMode::Disable)
        .options([("statement_timeout", "10s")]);
    let started = std::time::Instant::now();
    let pool = PgPoolOptions::new()
        .max_connections(POOL_MAX)
        .acquire_timeout(QUERY_TIMEOUT)
        // 空闲连接定期回收，降低 NAT/防火墙静默断连后复用死连接的概率
        .idle_timeout(Duration::from_secs(300))
        .connect_with(options)
        .await;
    if let Err(e) = &pool {
        warn!(
            host = %config.host,
            error = %e,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "连接云端数据库失败"
        );
    }
    let pool = pool.map_err(|e| AgentError::Db(format!("连接云端数据库失败：{e}")))?;
    ensure_schema(&pool).await?;
    info!(
        host = %config.host,
        database = %config.database,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "云端数据库已连接"
    );

    let mut guard = POOL.lock().await;
    // 并发重建时后到者让先到者（同 key 池等价），避免无谓抖动
    if guard.as_ref().is_none_or(|e| e.key != key) {
        *guard = Some(PoolEntry {
            key,
            pool: pool.clone(),
        });
    }
    Ok(pool)
}

/// 首连建表（幂等）：表结构与索引定义见 plan/2026-10-04_plan-persistence-pg.md。
/// 章节化存储演化 DDL 见任务 9 简报（与 plan_store.rs 九表读写逐字对齐）。
/// pub(crate)：plan_store 集成测试（tests_plan_store）复用同一套 DDL。
pub(crate) async fn ensure_schema(pool: &PgPool) -> Result<(), AgentError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS plans (
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
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_plans_created_at ON plans (created_at DESC)")
        .execute(pool)
        .await
        .map_err(|e| fail("初始化索引", e))?;

    // ---- 章节化存储演化（幂等）----
    // 旧列 plan_json 保留只读兼容：规范化新行不再写整串 JSON（新写入一律 NULL），
    // 对已可空的列执行 DROP NOT NULL 幂等成功
    sqlx::query("ALTER TABLE plans ALTER COLUMN plan_json DROP NOT NULL")
        .execute(pool)
        .await
        .map_err(|e| fail("演化 plans 表", e))?;
    // 生成状态：generating | complete | failed（默认 'complete' 保持旧行语义）
    sqlx::query(
        "ALTER TABLE plans ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'complete'",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("演化 plans 表", e))?;
    sqlx::query(
        "ALTER TABLE plans ADD COLUMN IF NOT EXISTS estimated_total_hours INT NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("演化 plans 表", e))?;
    // 以下八张子表全部经外键 ON DELETE CASCADE 随 plans 行级联清理
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS phases (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, name TEXT NOT NULL, week_range TEXT NOT NULL DEFAULT '',
          objectives JSONB NOT NULL DEFAULT '[]', practice JSONB NOT NULL DEFAULT '[]',
          assignments JSONB NOT NULL DEFAULT '[]',
          UNIQUE (plan_id, idx)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projects (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL UNIQUE REFERENCES phases(id) ON DELETE CASCADE,
          name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', background TEXT NOT NULL DEFAULT '',
          steps JSONB NOT NULL DEFAULT '[]', acceptance_criteria JSONB NOT NULL DEFAULT '[]',
          estimated_hours INT NOT NULL DEFAULT 0
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS chapters (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          phase_id UUID NOT NULL REFERENCES phases(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '',
          UNIQUE (phase_id, idx)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS concepts (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, name TEXT NOT NULL, definition TEXT NOT NULL DEFAULT ''
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS knowledge_points (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, detail TEXT NOT NULL DEFAULT ''
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS diagrams (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL DEFAULT '', mermaid TEXT NOT NULL DEFAULT '', caption TEXT NOT NULL DEFAULT ''
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS deep_dives (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
          idx INT NOT NULL, topic TEXT NOT NULL, angle TEXT NOT NULL DEFAULT '', content TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS resources (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
          idx INT NOT NULL, title TEXT NOT NULL, type TEXT NOT NULL DEFAULT 'docs', note TEXT NOT NULL DEFAULT ''
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| fail("初始化表结构", e))?;
    Ok(())
}

/// 操作失败统一记录日志并转 Db 错误（op 为中文动作名，直接拼进用户可见消息；
/// pub(crate)：plan_store 复用同一套错误归一与日志风格）
pub(crate) fn fail(op: &str, e: sqlx::Error) -> AgentError {
    warn!(op, error = %e, "云端数据库操作失败");
    AgentError::Db(format!("{op}失败：{e}"))
}

/// 行字段读取失败统一转 Db 错误
pub(crate) fn db_err(e: sqlx::Error) -> AgentError {
    fail("读取数据", e)
}

/// 字符串 id 解析为 UUID（防御非法输入直落 SQL 参数）
fn parse_id(id: &str) -> Result<uuid::Uuid, AgentError> {
    uuid::Uuid::parse_str(id).map_err(|_| AgentError::Db(format!("无效的计划 id：{id}")))
}

/// 保存计划（新记录），返回生成的 id
pub async fn save_plan(config: &DbConfig, request: &SavePlanRequest) -> Result<String, AgentError> {
    let pool = pool(config).await?;
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO plans (id, title, goal, overview, level, weeks, plan_json)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(&request.title)
    .bind(&request.goal)
    .bind(&request.overview)
    .bind(&request.level)
    .bind(i32::try_from(request.weeks).unwrap_or(0))
    .bind(sqlx::types::Json(&request.plan))
    .execute(&pool)
    .await
    .map_err(|e| fail("保存计划", e))?;
    info!(plan_id = %id, title = %request.title, "学习计划已保存到云端");
    Ok(id.to_string())
}

/// 计划列表（不含整份计划）：收藏置顶，其余按创建时间倒序；
/// query 非空时对标题/目标做大小写不敏感的模糊过滤（空串匹配全部）。
pub async fn list_plans(config: &DbConfig, query: &str) -> Result<Vec<PlanSummary>, AgentError> {
    let pool = pool(config).await?;
    let pattern = format!("%{}%", query.trim());
    let rows = sqlx::query(
        "SELECT id, title, goal, overview, level, weeks, favorite, created_at
         FROM plans
         WHERE title ILIKE $1 OR goal ILIKE $1
         ORDER BY favorite DESC, created_at DESC
         LIMIT $2",
    )
    .bind(&pattern)
    .bind(LIST_LIMIT)
    .fetch_all(&pool)
    .await
    .map_err(|e| fail("查询计划列表", e))?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id: uuid::Uuid = row.try_get("id").map_err(db_err)?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").map_err(db_err)?;
        let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
        result.push(PlanSummary {
            id: id.to_string(),
            title: row.try_get("title").map_err(db_err)?,
            goal: row.try_get("goal").map_err(db_err)?,
            overview: row.try_get("overview").map_err(db_err)?,
            level: row.try_get("level").map_err(db_err)?,
            weeks: weeks.max(0) as u32,
            favorite: row.try_get("favorite").map_err(db_err)?,
            created_at: created.to_rfc3339(),
        });
    }
    Ok(result)
}

/// 读取完整计划记录（含整份计划）：plan_json 旧行走旧反序列化路径（只读兼容），
/// 规范化新行走多表重组路径（见 plan_store::load_study_plan）
pub async fn get_plan(config: &DbConfig, id: &str) -> Result<PlanRecord, AgentError> {
    let pool = pool(config).await?;
    get_plan_by_pool(&pool, id).await
}

/// get_plan 的 pool 版核心（pub(crate) 供集成测试直连临时库复用）
pub(crate) async fn get_plan_by_pool(pool: &PgPool, id: &str) -> Result<PlanRecord, AgentError> {
    let row = sqlx::query(
        "SELECT id, title, goal, overview, level, weeks, favorite, created_at, plan_json, estimated_total_hours
         FROM plans WHERE id = $1",
    )
    .bind(parse_id(id)?)
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询计划", e))?
    .ok_or_else(|| AgentError::Db(format!("计划不存在或已被删除：{id}")))?;

    let plan_json: Option<sqlx::types::Json<StudyPlanAlias>> =
        row.try_get("plan_json").map_err(db_err)?;
    let record_id: uuid::Uuid = row.try_get("id").map_err(db_err)?;
    let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").map_err(db_err)?;
    let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
    let estimated_total_hours: i32 = row.try_get("estimated_total_hours").map_err(db_err)?;
    let title: String = row.try_get("title").map_err(db_err)?;
    let overview: String = row.try_get("overview").map_err(db_err)?;
    let plan = match plan_json {
        // 旧行：整串 plan_json 反序列化（含 estimatedTotalHours，行为与改造前一致）
        Some(json) => json.0,
        // 规范化新行：多表按 idx 重组
        None => {
            crate::plan_store::load_study_plan(
                pool,
                record_id,
                &title,
                &overview,
                estimated_total_hours,
            )
            .await?
        }
    };
    Ok(PlanRecord {
        id: record_id.to_string(),
        title,
        goal: row.try_get("goal").map_err(db_err)?,
        overview,
        level: row.try_get("level").map_err(db_err)?,
        weeks: weeks.max(0) as u32,
        favorite: row.try_get("favorite").map_err(db_err)?,
        created_at: created.to_rfc3339(),
        plan,
    })
}

/// StudyPlan 的类型别名（sqlx::types::Json 泛型参数需要具名类型）
type StudyPlanAlias = crate::protocol::StudyPlan;

/// 删除计划（硬删除；id 不存在视为已删除，幂等成功）
pub async fn delete_plan(config: &DbConfig, id: &str) -> Result<(), AgentError> {
    let pool = pool(config).await?;
    sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(parse_id(id)?)
        .execute(&pool)
        .await
        .map_err(|e| fail("删除计划", e))?;
    info!(plan_id = id, "学习计划已删除");
    Ok(())
}

/// 切换收藏状态，返回切换后的值（id 不存在时返回 false）
pub async fn toggle_favorite(config: &DbConfig, id: &str) -> Result<bool, AgentError> {
    let pool = pool(config).await?;
    let row = sqlx::query(
        "UPDATE plans SET favorite = NOT favorite, updated_at = now()
         WHERE id = $1 RETURNING favorite",
    )
    .bind(parse_id(id)?)
    .fetch_optional(&pool)
    .await
    .map_err(|e| fail("更新收藏状态", e))?;
    match row {
        Some(row) => Ok(row.try_get::<bool, _>(0).map_err(db_err)?),
        None => {
            warn!(plan_id = id, "收藏目标不存在");
            Ok(false)
        }
    }
}
