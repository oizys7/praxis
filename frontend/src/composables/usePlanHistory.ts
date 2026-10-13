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
