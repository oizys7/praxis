import { ref, shallowRef } from "vue";
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  PlanProgressEvent,
  PlanRequest,
  PlanResponse,
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
 * 重生成请求在飞锁：模块级共享 ref（同一时间只允许一个重生成请求在飞），
 * 同一计划的阶段级 / 章节级多个「重新生成」按钮据此互斥（按钮禁用 + 入口守卫）。
 */
export const regenInFlight = ref(false);

/**
 * 重生成单个阶段：invoke('regenerate_phase') + Channel 进度推送（对齐
 * generate_plan 的 Channel 形态），返回 done 终态包络（调用方只判 status 成败）。
 * 调用方负责详情刷新（fetchPlanRecord）：成功与失败都要刷新（失败也可见最新 failed 态）。
 * invoke 抛出的原始错误不在此归一，由调用方经 normalizeError 处理。
 */
export async function regeneratePhase(
  planId: string,
  phaseIndex: number,
  onProgress?: (event: PlanProgressEvent) => void,
): Promise<PlanResponse> {
  regenInFlight.value = true;
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
    regenInFlight.value = false;
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
  regenInFlight.value = true;
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
    regenInFlight.value = false;
  }
}

/**
 * useAgent：与 Tauri 后端（Rust -> Python Agent 服务）通信的复用逻辑。
 *
 * 封装：
 * - generatePlan：invoke('generate_plan')，管理 loading / error / result 状态；
 *   经 tauri Channel 接收流式生成进度（outline / phaseDone / storageIssue），
 *   进度聚合在 progress / storageIssue 两个响应式状态，也可经 onProgress 逐事件回调
 * - regeneratePhase / regenerateChapter（模块级导出）：阶段/章节级重生成，
 *   模块级 regenInFlight 锁保证同一时间只有一个请求在飞
 * - pingAgent：invoke('ping_agent')，探测 Python 服务是否在线
 * - normalizeError：把 invoke 抛出的错误统一归一化为 CommandError 对象
 */
export function useAgent() {
  /** 是否正在生成计划 */
  const loading = ref(false);
  /** 最近一次错误（null 表示无错误） */
  const error = shallowRef<CommandError | null>(null);
  /** 最近一次成功响应（统一包络） */
  const result = shallowRef<PlanResponse | null>(null);
  /** 生成进度：outline 事件定总数，phaseDone 事件推进（done 为 0-based index + 1） */
  const progress = shallowRef<{ done: number; total: number } | null>(null);
  /** 云端自动入库异常提示（storageIssue 事件；null 表示入库正常或不涉及） */
  const storageIssue = shallowRef<string | null>(null);

  /**
   * 请求 Agent 生成学习计划。
   * @param onProgress 可选：每个进度事件的原样回调（进度聚合见返回的 progress / storageIssue）
   */
  async function generatePlan(
    request: PlanRequest,
    onProgress?: (event: PlanProgressEvent) => void,
  ) {
    loading.value = true;
    error.value = null;
    result.value = null;
    progress.value = null;
    storageIssue.value = null;
    // 进度通道：key "channel" 对应 Rust 端签名 generate_plan(channel: Option<JavaScriptChannelId>)
    const channel = new Channel<PlanProgressEvent>();
    channel.onmessage = (event) => {
      if (event.type === "outline") {
        progress.value = { done: 0, total: event.phaseCount };
      } else if (event.type === "phaseDone") {
        // index 为 0-based 阶段索引（Python 端 phase_drafts 的 dict key）
        progress.value = { done: event.index + 1, total: event.total };
      } else if (event.type === "storageIssue") {
        storageIssue.value = event.message;
      }
      onProgress?.(event);
    };
    try {
      // 第二个参数的 key "request" 对应 Rust 端命令签名：generate_plan(request: PlanRequest)
      result.value = await invoke<PlanResponse>("generate_plan", {
        request,
        channel,
      });
    } catch (e) {
      error.value = normalizeError(e);
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
    progress.value = null;
    storageIssue.value = null;
  }

  return {
    loading,
    error,
    result,
    progress,
    storageIssue,
    generatePlan,
    pingAgent,
    reset,
  };
}
