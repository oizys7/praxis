//! 学习日程（打卡日历真数据）：开始学习落定逐章日程、修改学习日重排未来、
//! 行级打卡、按天组织的月历查询与年热力图查询。
//!
//! - 分配算法移植自前端 useStudyAllocation（已删除）：阶段按 weekRange
//!   （"第 X-Y 周"）切分学习日区间，章节 floor(i*m/n) 均匀铺到区间内学习日；
//!   解析失败的阶段归兜底组铺全期——与旧前端行为逐条对齐；
//! - 新模型差异：起点从「created_at 所在周周一」改为「今天」，且只排今天及
//!   未来的学习日（开始学习/重排当天提交，历史行永不重写）；
//! - done 口径：行级独立打卡（每章独立勾选），与章节生成状态（chapters.status）
//!   解耦；打卡仅允许 date ≤ 今天，未来行恒为未打卡。

use chrono::{Datelike, NaiveDate};
use sqlx::PgPool;
use sqlx::Row;
use tracing::info;
use uuid::Uuid;

use crate::db::{db_err, fail};
use crate::error::AgentError;
use crate::protocol::{MonthCalendarDay, ScheduleItem, YearHeatmapDay};

/// 今天的本地日期（日程锚点；应用为单机单用户桌面场景，取系统本地时区）
fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// 归一化每周学习日：非空、取值 1..=7、去重升序（非法输入拒绝，
/// 与原 set_study_days 校验口径一致）
fn normalize_days(days: &[u8]) -> Result<Vec<u8>, AgentError> {
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
    Ok(normalized)
}

/// 周区间解析（对齐前端 parseWeekRange）：「第 X-Y 周」「第X–Y周」
/// 「第 X 至 Y 周」「第 X 周」（含全角破折号与「至」）；解析不出返回 None。
fn parse_week_range(text: &str) -> Option<(u32, u32)> {
    let chars: Vec<char> = text.chars().collect();
    let pos_of = |needle: char| chars.iter().position(|c| *c == needle);
    let start = pos_of('第')?;
    let end = pos_of('周')?;
    if end <= start {
        return None;
    }
    let body: String = chars[start + 1..end].iter().collect();
    let body = body.replace(' ', "");
    // 拆分数字部分：分隔符支持 - – — ~ 至
    let split_at = body
        .char_indices()
        .find(|(_, c)| matches!(c, '-' | '–' | '—' | '~' | '至'));
    let (first, second) = match split_at {
        Some((idx, _)) => (
            // idx 是分隔符起始字节，安全边界；分隔符可能是多字节（–/—/至），
            // 第二段必须按字符跳过分隔符，不能字节 +1（会切进 UTF-8 中间）
            body[..idx].to_string(),
            Some(body[idx..].chars().skip(1).collect::<String>()),
        ),
        None => (body.clone(), None),
    };
    let w1: u32 = first.trim().parse().ok()?;
    let w2: u32 = match second {
        Some(s) => s.trim().parse().ok()?,
        None => w1,
    };
    Some(if w1 <= w2 { (w1, w2) } else { (w2, w1) })
}

/// 全期学习日序列（从 start 起按日历顺序收集学习日，共 weeks × days.len()
/// 个，全部 ≥ start）。等价于旧前端「周一锚 + (w-1)*7+(d-1) 偏移」的序列，
/// 但锚定为今天、天然排除已过去的日期。
fn study_slots(start: NaiveDate, weeks: u32, days: &[u8]) -> Vec<NaiveDate> {
    let want = weeks.max(1) as usize * days.len();
    let mut slots = Vec::with_capacity(want);
    let mut cursor = start;
    while slots.len() < want {
        // ISO 星期几：1=周一 .. 7=周日，与 studyDays 语义一致
        if days.contains(&(cursor.weekday().number_from_monday() as u8)) {
            slots.push(cursor);
        }
        cursor += chrono::Duration::days(1);
    }
    slots
}

/// 待排章节引用（schedule 行的冗余展示名一并携带，插入免回查）
#[derive(Debug, Clone)]
struct ChapterRef {
    phase_idx: i32,
    phase_name: String,
    /// chapters.idx 为 INTEGER（INT4），必须 i32 —— i64 会解码报错
    chapter_idx: i32,
    title: String,
    week_range: String,
}

/// 读取计划全部章节（骨架阶段即建齐全量 idx，覆盖 pending/complete/failed），
/// 阶段序号、章节序号升序；week_range 供分配切区间。
async fn load_chapters(pool: &PgPool, plan_id: Uuid) -> Result<Vec<ChapterRef>, AgentError> {
    let rows = sqlx::query(
        "SELECT p.idx AS phase_idx, p.name AS phase_name, p.week_range,
                c.idx AS chapter_idx, c.title
         FROM chapters c JOIN phases p ON c.phase_id = p.id
         WHERE p.plan_id = $1
         ORDER BY p.idx, c.idx",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("读取计划章节", e))?;
    let mut chapters = Vec::with_capacity(rows.len());
    for row in rows {
        chapters.push(ChapterRef {
            phase_idx: row.try_get("phase_idx").map_err(db_err)?,
            phase_name: row.try_get("phase_name").map_err(db_err)?,
            chapter_idx: row.try_get("chapter_idx").map_err(db_err)?,
            title: row.try_get("title").map_err(db_err)?,
            week_range: row.try_get("week_range").map_err(db_err)?,
        });
    }
    Ok(chapters)
}

/// 分配核心：把章节确定性铺到具体日期。
/// - 每「阶段组」用其 weekRange 对应的学习日区间（clamp 到 [1, weeks]）；
/// - weekRange 解析失败的章节归兜底组，统一铺全期学习日；
/// - 组内章节 floor(i*m/n) 均匀取点（n 章 m 个 slot，空组/空 slots 跳过）。
fn allocate(
    chapters: &[ChapterRef],
    slots: &[NaiveDate],
    weeks: u32,
) -> Vec<(NaiveDate, ChapterRef)> {
    let mut out = Vec::new();
    let mut fallback: Vec<&ChapterRef> = Vec::new();

    // 按阶段分组（输入已按 phase_idx, chapter_idx 升序，分组保序）
    let mut groups: Vec<(&ChapterRef, Vec<&ChapterRef>)> = Vec::new();
    for chapter in chapters {
        match groups.last_mut() {
            Some((head, members)) if head.phase_idx == chapter.phase_idx => {
                members.push(chapter);
            }
            _ => groups.push((chapter, vec![chapter])),
        }
    }

    for (head, members) in &groups {
        match parse_week_range(&head.week_range) {
            Some((w1, w2)) => {
                // clamp 到有效周界（weekRange 由 LLM 产出，防御越界）
                let lo = (w1.clamp(1, weeks) as usize - 1) * slots.len() / weeks.max(1) as usize;
                let hi = (w2.clamp(1, weeks) as usize) * slots.len() / weeks.max(1) as usize;
                spread(&mut out, members, &slots[lo..hi.max(lo)]);
            }
            None => fallback.extend(members.iter().copied()),
        }
    }
    if !fallback.is_empty() {
        spread(&mut out, &fallback, slots);
    }
    out
}

/// 章节序列均匀铺到 slots：floor(i*m/n) 单公式（与前端 spreadToSlots 一致；
/// slots 少于章节数时尾部收敛到末个学习日）
fn spread(out: &mut Vec<(NaiveDate, ChapterRef)>, chapters: &[&ChapterRef], slots: &[NaiveDate]) {
    let (n, m) = (chapters.len(), slots.len());
    if n == 0 || m == 0 {
        return;
    }
    for (i, chapter) in chapters.iter().enumerate() {
        out.push((slots[i * m / n], (*chapter).clone()));
    }
}

/// 开始学习：校验通过后写学习状态/开始日期/学习日，并把全部章节按学习日
/// 分配落表（done 恒 FALSE——开始学习不打卡今天）。事务原子。
pub async fn start_learning(pool: &PgPool, plan_id: Uuid, days: &[u8]) -> Result<(), AgentError> {
    let days = normalize_days(days)?;
    let row = sqlx::query("SELECT status, learning_status, weeks FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| fail("查询计划", e))?
        .ok_or_else(|| AgentError::Db(format!("计划不存在或已被删除：{plan_id}")))?;
    let learning_status: String = row.try_get("learning_status").map_err(db_err)?;
    if learning_status != "not_started" {
        return Err(AgentError::Db("该计划已开始学习，无需重复开始".into()));
    }
    let weeks: i32 = row.try_get("weeks").map_err(db_err)?;

    let chapters = load_chapters(pool, plan_id).await?;
    if chapters.is_empty() {
        return Err(AgentError::Db("计划大纲尚未就绪，暂不能开始学习".into()));
    }
    let start = today();
    let slots = study_slots(start, weeks.max(0) as u32, &days);
    let plan_rows = allocate(&chapters, &slots, weeks.max(1) as u32);

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| fail("开启开始学习事务", e))?;
    sqlx::query(
        "UPDATE plans SET learning_status = 'active', learning_started_at = $2,
                study_days = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(start)
    .bind(sqlx::types::Json(&days))
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("写入学习状态", e))?;
    // 防御清空（新计划本无日程行；重放/重试场景保证幂等）
    sqlx::query("DELETE FROM study_schedule WHERE plan_id = $1")
        .bind(plan_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("清理旧日程", e))?;
    for (date, chapter) in &plan_rows {
        sqlx::query(
            "INSERT INTO study_schedule
                (plan_id, date, phase_idx, chapter_idx, chapter_title, phase_name)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(plan_id)
        .bind(date)
        .bind(chapter.phase_idx)
        .bind(chapter.chapter_idx)
        .bind(&chapter.title)
        .bind(&chapter.phase_name)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("写入学习日程", e))?;
    }
    // 冗余打卡计数校准（列表直读免统计）：开始学习清旧插入后事务内一次校准
    sqlx::query(
        "UPDATE plans SET study_total = (SELECT COUNT(*) FROM study_schedule WHERE plan_id = $1)
         WHERE id = $1",
    )
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("同步打卡计数", e))?;
    tx.commit().await.map_err(|e| fail("提交开始学习事务", e))?;
    info!(%plan_id, days = ?days, rows = plan_rows.len(), "计划已开始学习，日程已落定");
    Ok(())
}

/// 修改学习日（仅学习中可调）：保留今天之前与今天已完成的日程行不动，
/// 删除今天未完成与未来的行，并把这些行对应的章节从今天起按新学习日重排。
/// 打卡仅允许 date ≤ 今天，故未来不存在已打卡行，删除集即未完成集。
pub async fn update_learning_days(
    pool: &PgPool,
    plan_id: Uuid,
    days: &[u8],
) -> Result<(), AgentError> {
    let days = normalize_days(days)?;
    let row = sqlx::query("SELECT learning_status, weeks FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| fail("查询计划", e))?
        .ok_or_else(|| AgentError::Db(format!("计划不存在或已被删除：{plan_id}")))?;
    let learning_status: String = row.try_get("learning_status").map_err(db_err)?;
    if learning_status != "active" {
        return Err(AgentError::Db("仅学习中的计划支持修改学习日".into()));
    }
    let weeks: i32 = row.try_get("weeks").map_err(db_err)?;
    let today = today();

    // 待重排集合 = 今天未完成 + 未来的行（携带冗余标题名，插入免回查）
    let removed = sqlx::query(
        "SELECT phase_idx, chapter_idx, chapter_title, phase_name
         FROM study_schedule
         WHERE plan_id = $1 AND (date > $2 OR (date = $2 AND NOT done))
         ORDER BY phase_idx, chapter_idx",
    )
    .bind(plan_id)
    .bind(today)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询待重排日程", e))?;
    if removed.is_empty() {
        // 无需重排也要落学习日（视图态一致性）
        sqlx::query("UPDATE plans SET study_days = $2, updated_at = now() WHERE id = $1")
            .bind(plan_id)
            .bind(sqlx::types::Json(&days))
            .execute(pool)
            .await
            .map_err(|e| fail("更新每周学习日", e))?;
        info!(%plan_id, days = ?days, "学习日已更新（无待重排日程）");
        return Ok(());
    }

    // 阶段区间对照：被删行冗余了名称，但 week_range 需查 phases 表
    let mut ranges: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    let range_rows = sqlx::query("SELECT idx, week_range FROM phases WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_all(pool)
        .await
        .map_err(|e| fail("读取阶段周区间", e))?;
    for row in range_rows {
        ranges.insert(
            row.try_get::<i32, _>("idx").map_err(db_err)?,
            row.try_get::<String, _>("week_range").map_err(db_err)?,
        );
    }
    let mut chapters: Vec<ChapterRef> = Vec::with_capacity(removed.len());
    for row in removed {
        let phase_idx: i32 = row.try_get("phase_idx").map_err(db_err)?;
        chapters.push(ChapterRef {
            phase_idx,
            phase_name: row.try_get("phase_name").map_err(db_err)?,
            chapter_idx: row.try_get("chapter_idx").map_err(db_err)?,
            title: row.try_get("chapter_title").map_err(db_err)?,
            week_range: ranges.get(&phase_idx).cloned().unwrap_or_default(),
        });
    }
    let slots = study_slots(today, weeks.max(0) as u32, &days);
    let plan_rows = allocate(&chapters, &slots, weeks.max(1) as u32);

    let mut tx = pool.begin().await.map_err(|e| fail("开启重排事务", e))?;
    sqlx::query(
        "DELETE FROM study_schedule
         WHERE plan_id = $1 AND (date > $2 OR (date = $2 AND NOT done))",
    )
    .bind(plan_id)
    .bind(today)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("删除待重排日程", e))?;
    for (date, chapter) in &plan_rows {
        sqlx::query(
            "INSERT INTO study_schedule
                (plan_id, date, phase_idx, chapter_idx, chapter_title, phase_name)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (plan_id, date, phase_idx, chapter_idx) DO NOTHING",
        )
        .bind(plan_id)
        .bind(date)
        .bind(chapter.phase_idx)
        .bind(chapter.chapter_idx)
        .bind(&chapter.title)
        .bind(&chapter.phase_name)
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("写入重排日程", e))?;
    }
    sqlx::query("UPDATE plans SET study_days = $2, updated_at = now() WHERE id = $1")
        .bind(plan_id)
        .bind(sqlx::types::Json(&days))
        .execute(&mut *tx)
        .await
        .map_err(|e| fail("更新每周学习日", e))?;
    // 冗余打卡计数校准：重排保留今天之前与今天已完成，事务内一次校准总数
    // （study_done 不动：删除的行全是未完成，已完成行不参与重排）
    sqlx::query(
        "UPDATE plans SET study_total = (SELECT COUNT(*) FROM study_schedule WHERE plan_id = $1)
         WHERE id = $1",
    )
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("同步打卡计数", e))?;
    tx.commit().await.map_err(|e| fail("提交重排事务", e))?;
    info!(%plan_id, days = ?days, rows = plan_rows.len(), "学习日已修改，未来日程已重排");
    Ok(())
}

/// 行级打卡开关（每章独立勾选）：仅允许今天及之前的日程打卡（未来不可预勾）；
/// 行不存在（id/日期/序号不匹配）归一为中文错误。
pub async fn toggle_schedule_item(
    pool: &PgPool,
    plan_id: Uuid,
    date: NaiveDate,
    phase_idx: i32,
    chapter_idx: i32,
    done: bool,
) -> Result<(), AgentError> {
    if date > today() {
        return Err(AgentError::Db("不能为未来的学习安排打卡".into()));
    }
    let mut tx = pool.begin().await.map_err(|e| fail("开启打卡事务", e))?;
    let result = sqlx::query(
        "UPDATE study_schedule SET done = $2 WHERE plan_id = $1
         AND date = $3 AND phase_idx = $4 AND chapter_idx = $5",
    )
    .bind(plan_id)
    .bind(done)
    .bind(date)
    .bind(phase_idx)
    .bind(chapter_idx)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("更新打卡状态", e))?;
    if result.rows_affected() == 0 {
        return Err(AgentError::Db("学习安排不存在（可能已被重排）".into()));
    }
    // 冗余计数同步（GREATEST 防御存量 0 底数被减穿）
    sqlx::query(
        "UPDATE plans SET study_done = GREATEST(0, study_done + CASE WHEN $2 THEN 1 ELSE -1 END)
         WHERE id = $1",
    )
    .bind(plan_id)
    .bind(done)
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("同步打卡计数", e))?;
    tx.commit().await.map_err(|e| fail("提交打卡事务", e))?;
    info!(%plan_id, %date, phase_idx, chapter_idx, done, "章节打卡已更新");
    Ok(())
}

/// 月历查询（按天组织）：该月全部日程 JOIN 计划标题，按日期升序分组；
/// 只返回有日程的天（前端月网格自行补空天）。
pub async fn query_month(
    pool: &PgPool,
    year: i32,
    month: u32,
) -> Result<Vec<MonthCalendarDay>, AgentError> {
    let rows = sqlx::query(
        "SELECT s.date, s.plan_id, s.phase_idx, s.chapter_idx, s.done,
                s.chapter_title, s.phase_name, p.title AS plan_title
         FROM study_schedule s JOIN plans p ON p.id = s.plan_id
         WHERE EXTRACT(YEAR FROM s.date) = $1 AND EXTRACT(MONTH FROM s.date) = $2
         ORDER BY s.date, p.title, s.phase_idx, s.chapter_idx",
    )
    .bind(year)
    .bind(i32::try_from(month).unwrap_or(1))
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询月历日程", e))?;

    let mut days: std::collections::BTreeMap<NaiveDate, MonthCalendarDay> =
        std::collections::BTreeMap::new();
    for row in rows {
        let date: NaiveDate = row.try_get("date").map_err(db_err)?;
        let day = days.entry(date).or_insert_with(|| MonthCalendarDay {
            date: date.format("%Y-%m-%d").to_string(),
            items: Vec::new(),
            total: 0,
            done_count: 0,
        });
        let done: bool = row.try_get("done").map_err(db_err)?;
        day.items.push(ScheduleItem {
            plan_id: row
                .try_get::<Uuid, _>("plan_id")
                .map_err(db_err)?
                .to_string(),
            plan_title: row.try_get("plan_title").map_err(db_err)?,
            phase_idx: row.try_get::<i32, _>("phase_idx").map_err(db_err)?.max(0) as u32,
            phase_name: row.try_get("phase_name").map_err(db_err)?,
            chapter_idx: row.try_get::<i32, _>("chapter_idx").map_err(db_err)?.max(0) as u32,
            chapter_title: row.try_get("chapter_title").map_err(db_err)?,
            done,
        });
        day.total += 1;
        if done {
            day.done_count += 1;
        }
    }
    Ok(days.into_values().collect())
}

/// 年热力图查询：按天聚合 total/done（强度色阶前端映射）。
pub async fn query_year(pool: &PgPool, year: i32) -> Result<Vec<YearHeatmapDay>, AgentError> {
    let rows = sqlx::query(
        "SELECT date, COUNT(*)::BIGINT AS total,
                COUNT(*) FILTER (WHERE done)::BIGINT AS done
         FROM study_schedule
         WHERE EXTRACT(YEAR FROM date) = $1
         GROUP BY date ORDER BY date",
    )
    .bind(year)
    .fetch_all(pool)
    .await
    .map_err(|e| fail("查询年热力图", e))?;
    let mut days = Vec::with_capacity(rows.len());
    for row in rows {
        days.push(YearHeatmapDay {
            date: row
                .try_get::<NaiveDate, _>("date")
                .map_err(db_err)?
                .format("%Y-%m-%d")
                .to_string(),
            total: row.try_get::<i64, _>("total").map_err(db_err)?.max(0) as u32,
            done: row.try_get::<i64, _>("done").map_err(db_err)?.max(0) as u32,
        });
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::NaiveDate;

    fn chapter(phase_idx: i32, chapter_idx: i32, week_range: &str) -> ChapterRef {
        ChapterRef {
            phase_idx,
            phase_name: format!("阶段{phase_idx}"),
            chapter_idx,
            title: format!("章节{chapter_idx}"),
            week_range: week_range.to_string(),
        }
    }

    fn slots_from(start: &str, weeks: u32, days: &[u8]) -> Vec<NaiveDate> {
        study_slots(
            NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap(),
            weeks,
            days,
        )
    }

    /// 归一化：去重升序、越界拒绝、空拒绝
    #[test]
    fn normalize_days_rejects_invalid_and_sorts() {
        assert_eq!(normalize_days(&[5, 1, 3, 1]).unwrap(), vec![1, 3, 5]);
        assert!(normalize_days(&[]).is_err());
        assert!(normalize_days(&[0, 1]).is_err());
        assert!(normalize_days(&[1, 8]).is_err());
    }

    /// 周区间解析：多形态分隔符 + 单周 + 非法
    #[test]
    fn parse_week_range_matches_frontend_semantics() {
        assert_eq!(parse_week_range("第 1-2 周"), Some((1, 2)));
        assert_eq!(
            parse_week_range("第X–Y周".replace('X', "1").replace('Y', "3").as_str()),
            Some((1, 3))
        );
        assert_eq!(parse_week_range("第 4 周"), Some((4, 4)));
        assert_eq!(parse_week_range("第 5 至 2 周"), Some((2, 5)));
        assert_eq!(parse_week_range("前三周"), None);
    }

    /// 学习日序列：从今天起只收学习日、数量 = weeks × days、全部 ≥ start
    #[test]
    fn study_slots_start_today_and_skip_non_study_days() {
        // 2026-01-05 是周一；只学周二(2)/周四(4)
        let slots = slots_from("2026-01-05", 2, &[2, 4]);
        assert_eq!(slots.len(), 4);
        assert_eq!(slots[0].to_string(), "2026-01-06");
        assert_eq!(slots[1].to_string(), "2026-01-08");
        assert_eq!(slots[2].to_string(), "2026-01-13");
        assert_eq!(slots[3].to_string(), "2026-01-15");
        // 起点是周二：当天（周二）即首个 slot，周一已过不补排
        let slots = slots_from("2026-01-06", 1, &[1, 2]);
        assert_eq!(slots[0].to_string(), "2026-01-06");
        assert_eq!(slots.len(), 2);
    }

    /// 分配：阶段区间铺章 + 解析失败归兜底组铺全期
    #[test]
    fn allocate_spreads_by_week_range_with_fallback() {
        let weeks = 4;
        let slots = slots_from("2026-01-05", weeks, &[1, 2, 3, 4, 5, 6, 7]); // 全周 28 slots
        let chapters = vec![
            chapter(0, 0, "第 1-2 周"),
            chapter(0, 1, "第 1-2 周"),
            chapter(0, 2, "第 1-2 周"),
            chapter(1, 0, "第 3-4 周"),
            chapter(2, 0, "无周区间"),
        ];
        let rows = allocate(&chapters, &slots, weeks);
        // 阶段 0 三章铺第 1-2 周（slots 0..14）
        assert_eq!(rows[0].0, slots[0]);
        assert_eq!(rows[1].0, slots[4]); // floor(1*14/3)=4
        assert_eq!(rows[2].0, slots[9]); // floor(2*14/3)=9
                                         // 阶段 1 一章铺第 3-4 周（slots 14..28）
        assert_eq!(rows[3].0, slots[14]);
        // 兜底组铺全期：落在末段
        assert_eq!(rows[4].0, slots[0]);
        assert_eq!(rows[4].1.phase_idx, 2);
    }

    /// slots 少于章节数：尾部收敛到末个学习日（与前端一致）
    #[test]
    fn allocate_collapses_tail_when_slots_scarce() {
        let slots = slots_from("2026-01-05", 1, &[1]); // 仅 1 个 slot
        let chapters = vec![chapter(0, 0, "第 1 周"), chapter(0, 1, "第 1 周")];
        let rows = allocate(&chapters, &slots, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, slots[0]);
        assert_eq!(rows[1].0, slots[0]);
    }
}
