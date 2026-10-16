import { invoke } from "@tauri-apps/api/core";
import { normalizeError } from "./useAgent";
import type { PlanRecord, PlanSummary } from "@/types/agent";

/**
 * usePlanHistory：学习计划云端持久化（Rust commands）的调用层。
 *
 * 设计为模块级独立 async 函数（无共享状态）：
 * - 列表页（PlanListView）自己管理 list / loading / error；
 * - 新计划的保存由 Rust 在生成链路内自动完成（不提供手动保存接口）；
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

/** 读取完整计划记录（含整份计划） */
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
 * 更新计划的每周学习日（Task E）：传 1..=7 的整数数组，Rust 侧归一
 * （去重升序）后落库，返回归一化结果（调用方直接采用，无需重拉详情）。
 */
export async function savePlanStudyDays(id: string, days: number[]): Promise<number[]> {
  try {
    return await invoke<number[]>("set_plan_study_days", { id, days });
  } catch (e) {
    throw normalizeError(e);
  }
}

/**
 * 当日已学打卡开关（Task E）：幂等；day 为本地日期 'YYYY-MM-DD'（dayKey）。
 * 失败抛 CommandError（中文文案）。
 */
export async function saveStudyDayMark(id: string, day: string, done: boolean): Promise<void> {
  try {
    await invoke("set_study_day_mark", { id, day, done });
  } catch (e) {
    throw normalizeError(e);
  }
}
