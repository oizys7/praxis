import { ref, shallowRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { normalizeError } from "./useAgent";
import type { AgentSettings } from "@/types/agent";

/**
 * useSettings：应用设置（Rust get_settings / save_settings）的复用逻辑。
 *
 * 封装：
 * - loadSettings：invoke('get_settings')，读取 settings.json（不存在时后端返回默认值）
 * - saveSettings：invoke('save_settings')，Rust 写盘 + 热推送 POST /api/config，
 *   保存成功后 Agent 服务立即使用新配置，无需重启应用或服务
 */
export function useSettings() {
  /** 当前持久化的设置（null = 尚未加载） */
  const settings = shallowRef<AgentSettings | null>(null);
  const loading = ref(false);
  const loadError = shallowRef<string | null>(null);
  const saving = ref(false);
  /** 保存结果提示：成功（含热加载语义）或失败原因 */
  const saveNotice = shallowRef<{ kind: "ok" | "warn"; text: string } | null>(
    null,
  );

  /** 读取已保存的设置 */
  async function loadSettings() {
    loading.value = true;
    loadError.value = null;
    try {
      settings.value = await invoke<AgentSettings>("get_settings");
    } catch (e) {
      loadError.value = normalizeError(e).message;
    } finally {
      loading.value = false;
    }
  }

  /** 保存设置（热推送失败由 Rust 端容错：下次服务启动时注入生效） */
  async function saveSettings(next: AgentSettings) {
    saving.value = true;
    saveNotice.value = null;
    try {
      // 参数名 settings 对应 Rust 端命令签名：save_settings(settings: AgentSettings)
      await invoke("save_settings", { settings: next });
      settings.value = next;
      saveNotice.value = {
        kind: "ok",
        text: "已保存并热加载：下次生成计划即使用新配置",
      };
    } catch (e) {
      saveNotice.value = { kind: "warn", text: normalizeError(e).message };
    } finally {
      saving.value = false;
    }
  }

  return {
    settings,
    loading,
    loadError,
    saving,
    saveNotice,
    loadSettings,
    saveSettings,
  };
}
