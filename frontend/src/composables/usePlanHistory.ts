import { invoke } from "@tauri-apps/api/core";
import { normalizeError } from "./useAgent";
import type {
  MonthCalendarDay,
  PlanRecord,
  PlanSummary,
  YearHeatmapDay,
} from "@/types/agent";

/**
 * usePlanHistory：学习计划云端持久化（Rust commands）的调用层。
 *
 * 设计为模块级独立 async 函数（无共享状态）：
 * - 列表页（PlanListView）自己管理 list / loading / error；
 * - 新计划的保存由 Rust 在生成链路内自动完成（不提供手动保存接口）；
 * - 打卡日历（CalendarView）只用 list_plans + 月/年日程查询，零 get_plan；
 * 错误统一经 normalizeError 归一为 CommandError（code=message=hint）。
 */

/** 计划列表：收藏置顶 + 创建时间倒序；query 非空时模糊过滤标题/目标 */
export async function fetchPlanSummaries(query = ""): Promise<PlanSummary[]> {
  try {
    return await invoke<PlanSummary[]>("list_plans", { query });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 读取完整计划记录（含整份计划；学习计划详情页用，日历页不用） */
export async function fetchPlanRecord(id: string): Promise<PlanRecord> {
  try {
    return await invoke<PlanRecord>("get_plan", { id });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 删除计划（幂等） */
export async function removePlan(id: string): Promise<void> {
  try {
    await invoke("delete_plan", { id });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 切换收藏状态，返回切换后的值 */
export async function switchFavorite(id: string): Promise<boolean> {
  try {
    return await invoke<boolean>("toggle_favorite", { id });
  } catch (e) {
    throw normalizeError(e);
  }
}

/**
 * 开始学习（打卡日历需求 1）：提交每周学习日（1..=7），Rust 写学习状态
 * active / 开始日期今天，并把全部章节按学习日分配落 study_schedule；
 * 不打卡今天。chips 仅本地选择，点【从今天开始】一次性提交。
 */
export async function startLearning(id: string, days: number[]): Promise<void> {
  try {
    await invoke("start_learning", { id, days });
  } catch (e) {
    throw normalizeError(e);
  }
}

/**
 * 修改学习日（需求 3，仅学习中）：今天之前与今天已完成的日程不动，
 * 今天未完成与未来的章节按新学习日从今天起重排。
 */
export async function updateLearningDays(
  id: string,
  days: number[],
): Promise<void> {
  try {
    await invoke("update_learning_days", { id, days });
  } catch (e) {
    throw normalizeError(e);
  }
}

/**
 * 行级打卡开关（需求 7，每章独立勾选）：仅允许今天及之前的日程打卡；
 * date 为本地日期 'YYYY-MM-DD'（dayKey）。
 */
export async function toggleScheduleItem(
  id: string,
  date: string,
  phaseIdx: number,
  chapterIdx: number,
  done: boolean,
): Promise<void> {
  try {
    await invoke("toggle_schedule_item", {
      id,
      date,
      phaseIdx,
      chapterIdx,
      done,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 月历查询（需求 5）：该月日程按天组织（只含有数据的天） */
export async function queryMonthCalendar(
  year: number,
  month: number,
): Promise<MonthCalendarDay[]> {
  try {
    return await invoke<MonthCalendarDay[]>("query_month_calendar", {
      year,
      month,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 年热力图查询（需求 6）：按天聚合 total/done 强度 */
export async function queryYearHeatmap(
  year: number,
): Promise<YearHeatmapDay[]> {
  try {
    return await invoke<YearHeatmapDay[]>("query_year_heatmap", { year });
  } catch (e) {
    throw normalizeError(e);
  }
}
