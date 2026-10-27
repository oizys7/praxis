//! 计划 / 阶段 / 章节状态的**唯一真源**。
//!
//! 背景（2026-10-22 架构审查）：状态推导曾分散在五处独立实现——`upsert_phase`
//! 的 Rust 内联判断、`upsert_chapter` 的 SQL CASE、`insert_chapter_row` 的另一段
//! Rust、`finalize_*` 与 `sweep_*` 各自比较字面量。口径已经开始漂移：同一份
//! 空正文，阶段整体写入判 `pending`（等待填充），单章写入判 `failed`（可重生成）
//! ——两种语义都正确，但此前没有任何地方写清楚这个区别。
//!
//! 本模块的三条约定：
//! 1. **语义只在这里定义**：枚举 + 纯函数是 spec，SQL 是它的机械翻译；
//! 2. **字面量只在这里出现**：其余模块一律用 `Status::Variant.as_str()`；
//! 3. **SQL 片段与纯函数配对**：`SQL_*` 常量必须与同名纯函数同语义，
//!    由 `plan_state::sql_pairing` 下的真实 DB 用例锁死（同一组输入两边比对）。
//!
//! 状态流转（章级为子，阶段/计划为父）：
//!
//! ```text
//! 章节：pending ──写入正文──> complete | failed
//!           └──重生成开始──> generating ──> complete | failed
//! 阶段：由子章汇总（phase_of_chapters）——父状态不独立存储语义
//! 计划：由全计划章节汇总（plan_of_chapters）——章级口径，不等 done 事件
//! ```

use std::fmt;

/// 章节状态。也是阶段状态（阶段由子章汇总，取值域相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChapterStatus {
    /// 骨架已播种、正文尚未生成
    Pending,
    /// 正在重生成（前端逐章点亮动画）
    Generating,
    /// 正文已写入
    Complete,
    /// 正文为空或生成失败，等待重生成
    Failed,
}

/// 阶段状态：取值域与章节相同，语义由 `phase_of_chapters` 汇总得出。
pub type PhaseStatus = ChapterStatus;

/// 计划状态：generate 期间 generating，收尾后 complete / partial / failed。
///
/// `Partial` 是「部分可用」这一格的正式名字：至少一章有正文、至少一章没有。
/// 它既不是成功也不是失败——把 7 章可用与 8 章全废压成同一个值，等于让响应
/// 对结果质量撒谎（HTTP 有 206 也是同一个道理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanStatus {
    Generating,
    Complete,
    Partial,
    Failed,
}

/// 学习状态：与生成状态解耦（「开始学习」推进 active，全章完成推进 completed）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LearningStatus {
    NotStarted,
    Active,
    Completed,
}

macro_rules! status_str {
    ($ty:ty, { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $ty {
            /// DB / wire 中的字面量（全仓唯一出现处）
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            /// 从 DB / wire 字面量解析；未知值返回 None（调用方决定兜底）。
            ///
            /// 保留为对称 API：`ChapterStatus` 已被 `all_chapters_complete`
            /// 使用，`PlanStatus` / `LearningStatus` 目前只在测试与未来读取
            /// 路径上使用，故整体放行 dead_code，而不是按类型分别裁剪。
            #[allow(dead_code)]
            pub fn parse(raw: &str) -> Option<Self> {
                match raw {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

status_str!(ChapterStatus, {
    Pending => "pending",
    Generating => "generating",
    Complete => "complete",
    Failed => "failed",
});

status_str!(PlanStatus, {
    Generating => "generating",
    Complete => "complete",
    Partial => "partial",
    Failed => "failed",
});

status_str!(LearningStatus, {
    NotStarted => "not_started",
    Active => "active",
    Completed => "completed",
});

// ---------------------------------------------------------------------------
// 纯函数：唯一算法
// ---------------------------------------------------------------------------

/// 单章写入时的状态定价：正文 strip 后为空 → `Failed`，否则 `Complete`。
///
/// `Failed` 而非 `Complete` 是重生成入口的判定链发出点：前端按 failed/pending
/// 态露出「重新生成此章」按钮。消费方：`plan_store::insert_chapter_row`。
pub fn chapter_of_content(content_md: &str) -> ChapterStatus {
    if content_md.trim().is_empty() {
        ChapterStatus::Failed
    } else {
        ChapterStatus::Complete
    }
}

/// 阶段**整体写入**（首展 phase 事件 / 整阶段重生成）时的单章定价：
/// 空正文 → `Pending`（尚未生成），非空 → `Complete`。
///
/// 与 [`chapter_of_content`] 的差异是刻意的：整体写入时还没有「尝试并失败」
/// 这回事，空正文只表示该章还没填；单章写入时空正文才是失败。
pub fn chapter_after_bulk_write(content_md: &str) -> ChapterStatus {
    if content_md.trim().is_empty() {
        ChapterStatus::Pending
    } else {
        ChapterStatus::Complete
    }
}

/// 阶段状态：由子章状态汇总（唯一算法）。
///
/// 优先级固定为 generating → complete → failed → pending：
/// - 有任意生成中章 → `Generating`（整阶段重生成时逐章推进动画，防止中途闪烁）；
/// - 非空且全 `Complete` → `Complete`；
/// - 有 `Failed` → `Failed`（暴露重生成入口）；
/// - 其余（含空集合、全 `Pending`）→ `Pending`。
pub fn phase_of_chapters(statuses: &[ChapterStatus]) -> PhaseStatus {
    if statuses.contains(&ChapterStatus::Generating) {
        ChapterStatus::Generating
    } else if !statuses.is_empty() && statuses.iter().all(|s| *s == ChapterStatus::Complete) {
        ChapterStatus::Complete
    } else if statuses.contains(&ChapterStatus::Failed) {
        ChapterStatus::Failed
    } else {
        ChapterStatus::Pending
    }
}

/// 计划终态：**章级三值口径**。
///
/// - 零章，或没有任何一章 `Complete` → `Failed`（纯骨架 / 全程失败）
/// - 全部 `Complete` → `Complete`
/// - 其余（有可用的、也有不可用的）→ `Partial`
///
/// 这条口径取代了历史上三版二值判定（any-phase-rows → has_substantive_phase_rows
/// → all_chapters_complete）。二值的问题是：自愈链跑完之后，一份 7 章可用、
/// 1 章空缺的计划会被标成 failed，用户看到「全部失败」，而数据库里躺着 7 章真内容。
pub fn plan_of_chapters(statuses: &[ChapterStatus]) -> PlanStatus {
    let complete = statuses
        .iter()
        .filter(|status| **status == ChapterStatus::Complete)
        .count();
    if complete == 0 {
        PlanStatus::Failed
    } else if complete == statuses.len() {
        PlanStatus::Complete
    } else {
        PlanStatus::Partial
    }
}

// ---------------------------------------------------------------------------
// SQL 片段：纯函数的机械翻译（唯一 SQL 文本来源）
// ---------------------------------------------------------------------------

/// 阶段状态汇总（`$1` = phases.id），与 [`phase_of_chapters`] 同语义。
///
/// 注意 `complete` 分支必须带 `EXISTS (任一章节)` 保护：`NOT EXISTS (非 complete)`
/// 在**零章节**时恒为真，会把空阶段判成 complete——与纯函数（空集合 → Pending）
/// 不一致。该边界由配对测试 `sql_phase_status_matches_pure_function` 抓到并锁死。
///
/// 使用处拼接：`format!("UPDATE phases SET status = {SQL_PHASE_STATUS_CASE} WHERE id = $1")`
/// 一致性由 `tests_plan_store::sql_phase_status_matches_pure_function` 在真实 DB 上锁定。
pub const SQL_PHASE_STATUS_CASE: &str = "CASE
             WHEN EXISTS (
                 SELECT 1 FROM chapters c WHERE c.phase_id = phases.id AND c.status = 'generating'
             ) THEN 'generating'
             WHEN EXISTS (
                 SELECT 1 FROM chapters c WHERE c.phase_id = phases.id
             ) AND NOT EXISTS (
                 SELECT 1 FROM chapters c WHERE c.phase_id = phases.id AND c.status <> 'complete'
             ) THEN 'complete'
             WHEN EXISTS (
                 SELECT 1 FROM chapters c WHERE c.phase_id = phases.id AND c.status = 'failed'
             ) THEN 'failed'
             ELSE 'pending' END";

/// 章节「正文 → 状态」的 SQL 翻译（`c` 为 chapters 表引用）。
///
/// 与 [`chapter_of_content`] 同语义：正文去空白后非空即 complete，否则 failed。
/// 自愈时用它从最下层反推整棵状态树。刻意不使用绑定参数（状态字面量只允许出现
/// 在本模块，风格同 [`SQL_PHASE_STATUS_CASE`]），这样调用方可自由组合自己的
/// `$1/$2` 而不发生编号冲突。
///
/// 一致性由 `tests_plan_store::sql_chapter_status_matches_pure_function` 锁定。
pub const SQL_CHAPTER_STATUS_FROM_CONTENT: &str =
    "CASE WHEN length(btrim(coalesce(c.content_md, ''))) > 0 THEN 'complete' ELSE 'failed' END";

/// 计划「全章 complete」判定（`$1` = plans.id），与 [`plan_of_chapters`] 的
/// Complete 分支同语义：至少一章存在，且不存在非 complete 章。
pub const SQL_PLAN_ALL_CHAPTERS_COMPLETE: &str = "EXISTS (
               SELECT 1 FROM chapters c2 JOIN phases p2 ON c2.phase_id = p2.id
               WHERE p2.plan_id = $1
           ) AND NOT EXISTS (
               SELECT 1 FROM chapters c JOIN phases p ON c.phase_id = p.id
               WHERE p.plan_id = $1 AND c.status <> 'complete'
           )";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_status_is_priced_by_content() {
        assert_eq!(chapter_of_content(""), ChapterStatus::Failed);
        assert_eq!(chapter_of_content("   \n\t "), ChapterStatus::Failed);
        assert_eq!(chapter_of_content("正文"), ChapterStatus::Complete);
    }

    #[test]
    fn bulk_write_treats_empty_as_pending_not_failed() {
        // 关键区别：整体写入的空正文是「还没生成」，不是「生成失败」
        assert_eq!(chapter_after_bulk_write(""), ChapterStatus::Pending);
        assert_eq!(chapter_after_bulk_write("正文"), ChapterStatus::Complete);
    }

    #[test]
    fn phase_status_priority_is_generating_first() {
        use ChapterStatus::*;
        assert_eq!(phase_of_chapters(&[Generating, Complete]), Generating);
        // 优先级压在 failed 之上：整阶段重生成中途不能闪成失败
        assert_eq!(phase_of_chapters(&[Generating, Failed]), Generating);
    }

    #[test]
    fn phase_status_covers_every_combination() {
        use ChapterStatus::*;
        assert_eq!(phase_of_chapters(&[]), Pending, "空集合 → pending");
        assert_eq!(phase_of_chapters(&[Pending]), Pending);
        assert_eq!(phase_of_chapters(&[Complete]), Complete);
        assert_eq!(phase_of_chapters(&[Complete, Complete]), Complete);
        assert_eq!(phase_of_chapters(&[Complete, Failed]), Failed);
        assert_eq!(phase_of_chapters(&[Failed, Pending]), Failed);
        assert_eq!(phase_of_chapters(&[Complete, Pending]), Pending);
    }

    #[test]
    fn plan_status_is_chapter_level() {
        use ChapterStatus::*;
        assert_eq!(plan_of_chapters(&[]), PlanStatus::Failed, "零章 → failed");
        assert_eq!(plan_of_chapters(&[Pending, Pending]), PlanStatus::Failed);
        assert_eq!(
            plan_of_chapters(&[Failed, Failed]),
            PlanStatus::Failed,
            "一章都没成 → failed，不是 partial"
        );
        // 部分可用：至少一章有正文、至少一章没有——这是 partial 唯一的定义
        assert_eq!(plan_of_chapters(&[Complete, Failed]), PlanStatus::Partial);
        assert_eq!(plan_of_chapters(&[Failed, Complete]), PlanStatus::Partial);
        assert_eq!(
            plan_of_chapters(&[Complete, Generating]),
            PlanStatus::Partial,
            "重生成中的章算不可用"
        );
        assert_eq!(plan_of_chapters(&[Complete]), PlanStatus::Complete);
        assert_eq!(
            plan_of_chapters(&[Complete, Complete]),
            PlanStatus::Complete
        );
    }

    #[test]
    fn status_literals_roundtrip() {
        for status in [
            ChapterStatus::Pending,
            ChapterStatus::Generating,
            ChapterStatus::Complete,
            ChapterStatus::Failed,
        ] {
            assert_eq!(ChapterStatus::parse(status.as_str()), Some(status));
        }
        for status in [
            PlanStatus::Generating,
            PlanStatus::Complete,
            PlanStatus::Partial,
            PlanStatus::Failed,
        ] {
            assert_eq!(PlanStatus::parse(status.as_str()), Some(status));
        }
        for status in [
            LearningStatus::NotStarted,
            LearningStatus::Active,
            LearningStatus::Completed,
        ] {
            assert_eq!(LearningStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ChapterStatus::parse("bogus"), None);
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(ChapterStatus::Complete.to_string(), "complete");
        assert_eq!(PlanStatus::Failed.to_string(), "failed");
        assert_eq!(LearningStatus::NotStarted.to_string(), "not_started");
    }
}
