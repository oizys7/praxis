import { invoke } from "@tauri-apps/api/core";
import { normalizeError } from "./useAgent";
import type { PlanRecord, PlanSummary, SavePlanInput } from "@/types/agent";

/**
 * usePlanHistory：学习计划云端持久化（Rust commands）的调用层。
 *
 * 设计为模块级独立 async 函数（无共享状态）：
 * - 列表页（PlanListView）自己管理 list / loading / error；
 * - 保存按钮（PlanResult）自己管理 saving / saved；
 * 错误统一经 normalizeError 归一为 CommandError（code=message=hint）。
 */

/** 保存学习计划（新记录），返回 Rust 生成的计划 id */
export async function savePlan(input: SavePlanInput): Promise<string> {
  try {
    // record 对应 Rust 端签名：save_plan(record: SavePlanRequest)
    return await invoke<string>("save_plan", { record: input });
  } catch (e) {
    throw normalizeError(e);
  }
}

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
