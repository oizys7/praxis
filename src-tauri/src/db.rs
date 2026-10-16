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
use crate::protocol::{PlanRecord, PlanSummary};

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

/// 首连建表/演进（幂等）：全部 DDL 统一管理在 db_ddl 模块（含任务 13 的
/// 两级状态机与生成上下文列），本函数仅委托执行。
/// pub(crate)：plan_store 集成测试（tests_plan_store）复用同一套 DDL。
pub(crate) async fn ensure_schema(pool: &PgPool) -> Result<(), AgentError> {
    crate::db_ddl::run(pool).await
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

/// 字符串 id 解析为 UUID（防御非法输入直落 SQL 参数；pub(crate) 供
/// commands 的 Task E 新命令复用，避免双份解析漂移）
pub(crate) fn parse_id(id: &str) -> Result<uuid::Uuid, AgentError> {
    uuid::Uuid::parse_str(id).map_err(|_| AgentError::Db(format!("无效的计划 id：{id}")))
}

/// 计划列表（不含整份计划）：收藏置顶，其余按创建时间倒序；
/// query 非空时对标题/目标做大小写不敏感的模糊过滤（空串匹配全部）。
pub async fn list_plans(config: &DbConfig, query: &str) -> Result<Vec<PlanSummary>, AgentError> {
    let pool = pool(config).await?;
    list_plans_by_pool(&pool, query).await
}

/// list_plans 的 pool 版核心（pub(crate) 供集成测试直连临时库复用，
/// 与 get_plan_by_pool 同款 motif）
pub(crate) async fn list_plans_by_pool(
    pool: &PgPool,
    query: &str,
) -> Result<Vec<PlanSummary>, AgentError> {
    let pattern = format!("%{}%", query.trim());
    let rows = sqlx::query(
        "SELECT id, title, goal, overview, level, weeks, favorite, created_at, status, study_days
         FROM plans
         WHERE title ILIKE $1 OR goal ILIKE $1
         ORDER BY favorite DESC, created_at DESC
         LIMIT $2",
    )
    .bind(&pattern)
    .bind(LIST_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询计划列表", e))?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id: uuid::Uuid = row.try_get("id").map_err(db_err)?;
        let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").map_err(db_err)?;
        let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
        // 每周学习日（Task E）：列默认全周，读回恒为数组
        let study_days: sqlx::types::Json<Vec<u8>> = row.try_get("study_days").map_err(db_err)?;
        result.push(PlanSummary {
            id: id.to_string(),
            title: row.try_get("title").map_err(db_err)?,
            goal: row.try_get("goal").map_err(db_err)?,
            // 行级生成状态（generating | complete | failed）随列表回传，
            // 历史徽章跨会话恢复（任务 24 补全）
            status: row.try_get("status").map_err(db_err)?,
            overview: row.try_get("overview").map_err(db_err)?,
            level: row.try_get("level").map_err(db_err)?,
            weeks: weeks.max(0) as u32,
            study_days: study_days.0,
            favorite: row.try_get("favorite").map_err(db_err)?,
            created_at: created.to_rfc3339(),
        });
    }
    Ok(result)
}

/// 读取完整计划记录（含整份计划）：规范化新行走多表重组路径（见
/// plan_store::load_study_plan）
pub async fn get_plan(config: &DbConfig, id: &str) -> Result<PlanRecord, AgentError> {
    let pool = pool(config).await?;
    get_plan_by_pool(&pool, id).await
}

/// get_plan 的 pool 版核心（pub(crate) 供集成测试直连临时库复用）
pub(crate) async fn get_plan_by_pool(pool: &PgPool, id: &str) -> Result<PlanRecord, AgentError> {
    let row = sqlx::query(
        "SELECT id, title, goal, overview, level, weeks, favorite, created_at, status, estimated_total_hours, study_days
         FROM plans WHERE id = $1",
    )
    .bind(parse_id(id)?)
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询计划", e))?
    .ok_or_else(|| AgentError::Db(format!("计划不存在或已被删除：{id}")))?;

    let record_id: uuid::Uuid = row.try_get("id").map_err(db_err)?;
    let created: chrono::DateTime<chrono::Utc> = row.try_get("created_at").map_err(db_err)?;
    let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
    let estimated_total_hours: i32 = row.try_get("estimated_total_hours").map_err(db_err)?;
    // 行级生成状态（generating | complete | failed）：如实回传（默认 'complete'）
    let status: String = row.try_get("status").map_err(db_err)?;
    let title: String = row.try_get("title").map_err(db_err)?;
    let overview: String = row.try_get("overview").map_err(db_err)?;
    // 每周学习日（Task E）：列默认全周，读回恒为数组
    let study_days: sqlx::types::Json<Vec<u8>> = row.try_get("study_days").map_err(db_err)?;
    // 已打卡日（Task E）：'YYYY-MM-DD' 升序序列（前端 Set 化查询）
    let marks = crate::plan_store::load_plan_marks(pool, record_id).await?;
    let mut day_marks: Vec<String> = marks
        .keys()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .collect();
    day_marks.sort();
    let plan = crate::plan_store::load_study_plan(
        pool,
        record_id,
        &title,
        &overview,
        estimated_total_hours,
    )
    .await?;
    Ok(PlanRecord {
        id: record_id.to_string(),
        title,
        goal: row.try_get("goal").map_err(db_err)?,
        status,
        overview,
        level: row.try_get("level").map_err(db_err)?,
        weeks: weeks.max(0) as u32,
        study_days: study_days.0,
        day_marks,
        favorite: row.try_get("favorite").map_err(db_err)?,
        created_at: created.to_rfc3339(),
        plan,
    })
}

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
