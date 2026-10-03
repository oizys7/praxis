import { ref, shallowRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { CommandError, PlanRequest, PlanResponse } from "@/types/agent";

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
 * useAgent：与 Tauri 后端（Rust -> Python Agent 服务）通信的复用逻辑。
 *
 * 封装：
 * - generatePlan：invoke('generate_plan')，管理 loading / error / result 状态
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

  /** 请求 Agent 生成学习计划 */
  async function generatePlan(request: PlanRequest) {
    loading.value = true;
    error.value = null;
    result.value = null;
    try {
      // 第二个参数的 key "request" 对应 Rust 端命令签名：generate_plan(request: PlanRequest)
      result.value = await invoke<PlanResponse>("generate_plan", { request });
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
  }

  return { loading, error, result, generatePlan, pingAgent, reset };
}
