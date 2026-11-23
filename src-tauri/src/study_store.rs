//! 研读页数据存储层（2026-11 功能包1）：章节聊天 / 选区标注 / 章节笔记。
//!
//! 与 plan_store 的分工：plan_store 负责「生成链路的计划内容」多表读写；
//! 本模块负责研读页的用户产出数据（对话 / 划线批注 / 笔记），三张表均以
//! (plan_id, phase_idx, chapter_idx) 复合键定位（DDL 见 db_ddl）：
//! - chapter_chats：一章一串有序消息（seq 章内自增，追加写不覆盖）；
//! - chapter_annotations：一章多条标注（id 由前端生成，保存即 upsert）；
//! - chapter_notes：一章至多一行 markdown（保存即 upsert，空串 = 清空）。
//!
//! 所有写入都要求计划行存在（外键自然兜底）；读取按章定位、时间升序。

use sqlx::{PgPool, Row};
use tracing::info;
use uuid::Uuid;

use crate::db::{db_err, fail};
use crate::error::AgentError;
use crate::protocol::{ChapterAnnotationDef, ChapterNoteDef, ChatMessageDef};

/// 章内消息序号下一值（现有 seq 最大值 + 1；空对话从 1 起）。
async fn next_chat_seq(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    plan_id: Uuid,
    phase_idx: i32,
    chapter_idx: i32,
) -> Result<i32, AgentError> {
    let max: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(seq) FROM chapter_chats
         WHERE plan_id = $1 AND phase_idx = $2 AND chapter_idx = $3",
    )
    .bind(plan_id)
    .bind(phase_idx)
    .bind(chapter_idx)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| fail("查询聊天序号", e))?;
    Ok(max.map_or(1, |m| m + 1))
}

/// 追加一轮问答（用户提问 + 助手回答，同事务两条、seq 连续）。
/// 只有整轮成功才可见（流式完成后调用，失败轮不落库）。
/// 消费方：commands::chat_stream（chatDone 终态后）。
pub async fn append_chat_exchange(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
    question: &str,
    answer: &str,
) -> Result<(), AgentError> {
    let phase_idx = i32::try_from(phase_index).unwrap_or(0);
    let chapter_idx = i32::try_from(chapter_index).unwrap_or(0);
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启聊天写入事务", e))?;
    let base = next_chat_seq(&mut tx, plan_id, phase_idx, chapter_idx).await?;
    sqlx::query(
        "INSERT INTO chapter_chats (plan_id, phase_idx, chapter_idx, seq, role, content)
         VALUES ($1, $2, $3, $4, 'user', $5), ($1, $2, $3, $4 + 1, 'assistant', $6)",
    )
    .bind(plan_id)
    .bind(phase_idx)
    .bind(chapter_idx)
    .bind(base)
    .bind(question)
    .bind(answer)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("写入聊天记录", e))?;
    tx.commit().await.map_err(|e| fail("提交聊天事务", e))?;
    info!(%plan_id, phase_index, chapter_index, "章节聊天已落库（一轮问答）");
    Ok(())
}

/// 读取本章聊天记录（seq 升序；role/content + 端特有 id/createdAt）。
pub async fn list_chat_messages(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<Vec<ChatMessageDef>, AgentError> {
    let rows = sqlx::query(
        "SELECT id, role, content, created_at FROM chapter_chats
         WHERE plan_id = $1 AND phase_idx = $2 AND chapter_idx = $3
         ORDER BY seq ASC",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询章节聊天", e))?;

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let id: Uuid = row.try_get("id").map_err(db_err)?;
        let created_at: chrono::DateTime<chrono::Utc> =
            row.try_get("created_at").map_err(db_err)?;
        messages.push(ChatMessageDef {
            role: row.try_get("role").map_err(db_err)?,
            content: row.try_get("content").map_err(db_err)?,
            id: Some(id.to_string()),
            created_at: Some(created_at.to_rfc3339()),
        });
    }
    Ok(messages)
}

/// 保存选区标注（id 由前端生成；同 id 重复保存 = 编辑回写，upsert 语义）。
/// 消费方：commands::save_annotation。
pub async fn save_annotation(
    pool: &PgPool,
    annotation: &ChapterAnnotationDef,
    plan_id: Uuid,
) -> Result<(), AgentError> {
    sqlx::query(
        "INSERT INTO chapter_annotations (id, plan_id, phase_idx, chapter_idx, quote, comment)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (plan_id, phase_idx, chapter_idx, id) DO UPDATE SET
             quote = EXCLUDED.quote,
             comment = EXCLUDED.comment",
    )
    .bind(&annotation.id)
    .bind(plan_id)
    .bind(i32::try_from(annotation.phase_index).unwrap_or(0))
    .bind(i32::try_from(annotation.chapter_index).unwrap_or(0))
    .bind(&annotation.quote)
    .bind(&annotation.comment)
    .execute(pool)
    .await
    .map_err(|e| fail("保存章节标注", e))?;
    Ok(())
}

/// 读取本章全部标注（创建时间升序）。
pub async fn list_annotations(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<Vec<ChapterAnnotationDef>, AgentError> {
    let rows = sqlx::query(
        "SELECT id, phase_idx, chapter_idx, quote, comment, created_at
         FROM chapter_annotations
         WHERE plan_id = $1 AND phase_idx = $2 AND chapter_idx = $3
         ORDER BY created_at ASC",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询章节标注", e))?;

    let mut annotations = Vec::with_capacity(rows.len());
    for row in rows {
        let phase_idx: i32 = row.try_get("phase_idx").map_err(db_err)?;
        let chapter_idx: i32 = row.try_get("chapter_idx").map_err(db_err)?;
        let created_at: chrono::DateTime<chrono::Utc> =
            row.try_get("created_at").map_err(db_err)?;
        annotations.push(ChapterAnnotationDef {
            id: row.try_get("id").map_err(db_err)?,
            phase_index: phase_idx.max(0) as u32,
            chapter_index: chapter_idx.max(0) as u32,
            quote: row.try_get("quote").map_err(db_err)?,
            comment: row.try_get("comment").map_err(db_err)?,
            created_at: created_at.to_rfc3339(),
        });
    }
    Ok(annotations)
}

/// 删除选区标注（幂等：不存在视同已删除）。
pub async fn delete_annotation(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
    annotation_id: &str,
) -> Result<(), AgentError> {
    sqlx::query(
        "DELETE FROM chapter_annotations
         WHERE plan_id = $1 AND phase_idx = $2 AND chapter_idx = $3 AND id = $4",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .bind(annotation_id)
    .execute(pool)
    .await
    .map_err(|e| fail("删除章节标注", e))?;
    Ok(())
}

/// 读取章节笔记（无笔记行返回 None；contentMd 空串行 = 已清空，按 Some 返回）。
pub async fn get_chapter_note(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
) -> Result<Option<ChapterNoteDef>, AgentError> {
    let row = sqlx::query(
        "SELECT content_md, updated_at FROM chapter_notes
         WHERE plan_id = $1 AND phase_idx = $2 AND chapter_idx = $3",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .fetch_optional(pool)
    .await
    .map_err(|e| fail("查询章节笔记", e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let updated_at: chrono::DateTime<chrono::Utc> =
        row.try_get("updated_at").map_err(db_err)?;
    Ok(Some(ChapterNoteDef {
        content_md: row.try_get("content_md").map_err(db_err)?,
        updated_at: updated_at.to_rfc3339(),
    }))
}

/// 保存章节笔记（一章至多一行，保存即 upsert；contentMd 空串 = 清空）。
/// 消费方：commands::save_chapter_note。
pub async fn save_chapter_note(
    pool: &PgPool,
    plan_id: Uuid,
    phase_index: u32,
    chapter_index: u32,
    content_md: &str,
) -> Result<(), AgentError> {
    sqlx::query(
        "INSERT INTO chapter_notes (plan_id, phase_idx, chapter_idx, content_md)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (plan_id, phase_idx, chapter_idx) DO UPDATE SET
             content_md = EXCLUDED.content_md",
    )
    .bind(plan_id)
    .bind(i32::try_from(phase_index).unwrap_or(0))
    .bind(i32::try_from(chapter_index).unwrap_or(0))
    .bind(content_md)
    .execute(pool)
    .await
    .map_err(|e| fail("保存章节笔记", e))?;
    info!(%plan_id, phase_index, chapter_index, "章节笔记已保存");
    Ok(())
}
