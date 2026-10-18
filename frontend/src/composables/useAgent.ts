import { ref, shallowRef } from "vue";
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  PlanProgressEvent,
  PlanRequest,
  PlanResponse,
  PlanSubmitResult,
} from "@/types/agent";

/**
 * Tauri command 的 Err / panic 会以任意形态抛到 JS 侧：
 * 字符串或 { code, message, hint } 对象，统一归一化处理。
 * 模块级导出：useAgent 与 useSettings 共用。
 */
export function normalizeError(e: unknown): CommandError {
  if (typeof e === "string") {
    return { code: "unknown", message: e };
  }
  if (e && typeof e === "object" && "message" in e) {
    return e as CommandError;
  }
  return { code: "unknown", message: String(e) };
}

/**
 * 重生成请求在飞锁（keyed 化，修复问题①）：模块级 scope 集合，
 * key 形如 "{planId}:{phaseIndex}" / "{planId}:{phaseIndex}:{chapterIndex}"。
 * 语义要点：scope 的 add 在模块级 regeneratePhase / regenerateChapter 启动时、
 * delete 在同一函数的 try/finally——finally 挂在模块级 promise 上，页面切换、
 * 组件卸载都拦不住它的执行，切走再进不会出现「锁死了根本没在飞的请求」。
 * 同一计划的阶段级 / 章节级多个「重新生成」按钮据此互斥（按钮禁用 + 入口守卫）。
 */
export const regenScopes = ref<Set<string>>(new Set());

/** 同计划任意阶段/章节 scope 在飞（PhaseCard 按钮禁用与文案判据；同 plan 互斥） */
export function isRegenInFlight(planId: string): boolean {
  const prefix = `${planId}:`;
  for (const key of regenScopes.value) {
    if (key.startsWith(prefix)) return true;
  }
  return false;
}

/** 指定 scope 是否在飞（阶段级 / 章节级按需精确判定） */
export function isScopeInFlight(key: string): boolean {
  return regenScopes.value.has(key);
}

/** 阶段级 scope key（导出：ChapterSection 判断「所属阶段重生成中」复用同一约定） */
export function phaseScope(planId: string, phaseIndex: number): string {
  return `${planId}:${phaseIndex}`;
}

/**
 * 重生成单个阶段：invoke('regenerate_phase') + Channel 进度推送
 * （PhaseDone{index, total=阶段总数}；generate_plan 已改走全局事件，此路径不变），
 * 返回 done 终态包络（调用方只判 status 成败）。
 * 调用方负责详情刷新（fetchPlanRecord）：成功与失败都要刷新（失败也可见最新 failed 态）。
 * invoke 抛出的原始错误不在此归一，由调用方经 normalizeError 处理。
 */
export async function regeneratePhase(
  planId: string,
  phaseIndex: number,
  onProgress?: (event: PlanProgressEvent) => void,
): Promise<PlanResponse> {
  const scope = phaseScope(planId, phaseIndex);
  regenScopes.value.add(scope);
  try {
    const channel = new Channel<PlanProgressEvent>();
    channel.onmessage = (event) => onProgress?.(event);
    // 参数名对应 Rust 端 regenerate_phase(planId, phaseIndex, channel?)（camelCase）
    return await invoke<PlanResponse>("regenerate_phase", {
      planId,
      phaseIndex,
      channel,
    });
  } finally {
    // finally 在模块级 promise 上：页面切换 / 组件卸载后依然释放（keyed 锁关键语义）
    regenScopes.value.delete(scope);
  }
}

/**
 * 重生成单个章节：invoke('regenerate_chapter')，形态同 [regeneratePhase]。
 * 单章进度事件为 PhaseDone{index=章节索引, total=1}：调用方对 total=1 的展示
 * 弱化为单行 spinner（不显示 1/1 计数）。
 */
export async function regenerateChapter(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
  onProgress?: (event: PlanProgressEvent) => void,
): Promise<PlanResponse> {
  const scope = `${planId}:${phaseIndex}:${chapterIndex}`;
  regenScopes.value.add(scope);
  try {
    const channel = new Channel<PlanProgressEvent>();
    channel.onmessage = (event) => onProgress?.(event);
    // 参数名对应 Rust 端 regenerate_chapter(planId, phaseIndex, chapterIndex, channel?)
    return await invoke<PlanResponse>("regenerate_chapter", {
      planId,
      phaseIndex,
      chapterIndex,
      channel,
    });
  } finally {
    // 同 phaseScope 的 delete 语义：模块级 finally，页面生命周期不截断
    regenScopes.value.delete(scope);
  }
}

/**
 * useAgent：与 Tauri 后端（Rust -> Python Agent 服务）通信的复用逻辑。
 *
 * 封装：
 * - generatePlan：invoke('generate_plan')，提交即返 {planId}（不等终态，
 *   任务 22 起 Rust 已任务化）；生成进度经全局事件 'plan-progress' 推送，
 *   由 useGenerationProgress 的模块级 store 观测，此处不再传 Channel。
 *   错误归一进 error 状态并返回 null（调用方判空后读 error）。
 * - regeneratePhase / regenerateChapter（模块级导出）：阶段/章节级重生成，
 *   仍走流式命令 + tauri Channel 推送进度；模块级 keyed 锁（regenScopes）保证
 *   同 plan 的重生成请求互斥，且锁释放不依赖页面存活（模块级 finally）
 * - pingAgent：invoke('ping_agent')，探测 Python 服务是否在线
 * - normalizeError：把 invoke 抛出的错误统一归一化为 CommandError 对象
 */
export function useAgent() {
  /** 提交请求是否在飞（仅提交握手窗口，不等生成终态） */
  const loading = ref(false);
  /** 最近一次错误（null 表示无错误） */
  const error = shallowRef<CommandError | null>(null);
  /** 当前展示的计划（生成终态回读 / 历史查看注入；新生成不再等包络） */
  const result = shallowRef<PlanResponse | null>(null);

  /**
   * 提交生成任务：返回 {planId}（后台生成已在跑）；失败返回 null（error 已置值）。
   * 调用方须**先** await ensureProgressListener（useGenerationProgress），
   * 再调用本函数——outline 事件可能先于命令返回值到达。
   */
  async function generatePlan(
    request: PlanRequest,
  ): Promise<PlanSubmitResult | null> {
    loading.value = true;
    error.value = null;
    try {
      // key "request" 对应 Rust 端命令签名：generate_plan(request: PlanRequest)；
      // 返回类型对应 PlanSubmitResult { planId }（camelCase wire）
      return await invoke<PlanSubmitResult>("generate_plan", { request });
    } catch (e) {
      error.value = normalizeError(e);
      return null;
    } finally {
      loading.value = false;
    }
  }

  /** 探测 Python Agent 服务是否在线（GET /health） */
  async function pingAgent(): Promise<boolean> {
    try {
      return await invoke<boolean>("ping_agent");
    } catch {
      return false;
    }
  }

  /** 清空状态 */
  function reset() {
    error.value = null;
    result.value = null;
  }

  return {
    loading,
    error,
    result,
    generatePlan,
    pingAgent,
    reset,
  };
}
