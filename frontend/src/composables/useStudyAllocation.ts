/**
 * useStudyAllocation：日历视图共用的小工具（精简版）。
 *
 * 章节到日期的分配算法已迁至 Rust `src-tauri/src/schedule.rs`（start_learning /
 * update_learning_days 落定 study_schedule 日程表，前端零分配逻辑）；
 * 月历/热力图数据来自 query_month_calendar / query_year_heatmap 真接口。
 * 本模块仅保留视图层共用：日子键、单日统计类型与强度色阶映射。
 */

/** 热力图强度五档：0=无安排/未学 → 4=当日全满（语义唯一来源，图例共用） */
export type IntensityLevel = 0 | 1 | 2 | 3 | 4;

/** 单日学习统计（月历点态与年热力图共用的最小单元） */
export interface DayStat {
  /** 当天安排章数 */
  total: number;
  /** 当天已学章数（口径 = study_schedule.done 行级打卡；恒 <= total） */
  done: number;
}

/** 本地时区安全的日子键（YYYY-MM-DD） */
export function dayKey(date: Date): string {
  const m = `${date.getMonth() + 1}`.padStart(2, "0");
  const d = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

/**
 * 强度五档映射（当天完成比例）：
 * 0=无安排或未学 → 1=低(<1/3) → 2=中(<2/3) → 3=高(<满) → 4=满(全部完成)
 */
export function intensityLevel(stat: DayStat): IntensityLevel {
  if (stat.total === 0 || stat.done === 0) return 0;
  const ratio = stat.done / stat.total;
  if (ratio < 1 / 3) return 1;
  if (ratio < 2 / 3) return 2;
  if (ratio < 1) return 3;
  return 4;
}
