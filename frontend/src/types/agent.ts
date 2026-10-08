/**
 * Praxis 三端统一的数据类型定义（TypeScript 侧）。
 *
 * 字段命名与以下两处严格对齐（JSON 层均为 camelCase）：
 * - src-tauri/src/commands.rs（serde rename_all = "camelCase"）
 * - agent-service/app/schemas.py（Pydantic alias_generator = to_camel）
 */

/** 当前水平 */
export type Level = "beginner" | "intermediate" | "advanced";

/** 学习计划生成请求（前端 -> Tauri invoke -> Rust -> Python） */
export interface PlanRequest {
  /** 学习目标 */
  goal: string;
  /** 当前水平 */
  level: Level;
  /** 每周可用小时数 */
  weeklyHours: number;
  /** 期望总周数 */
  weeks: number;
  /** 偏好方向（多选标签） */
  interests: string[];
  /** 已有基础描述 */
  background: string;
}

/** 阶段收尾项目及其验收标准 */
export interface PhaseProject {
  name: string;
  description: string;
  acceptanceCriteria: string[];
}

/** 学习计划中的一个阶段：理论 / 实操 / 作业 / 项目 四位一体 */
export interface StudyPhase {
  name: string;
  weekRange: string;
  objectives: string[];
  theory: string[];
  practice: string[];
  assignments: string[];
  project: PhaseProject;
}

/** 推荐学习资源 */
export interface StudyResource {
  title: string;
  /** book | course | docs | video | other */
  type: string;
  note: string;
}

/** 完整学习计划 */
export interface StudyPlan {
  title: string;
  overview: string;
  estimatedTotalHours: number;
  phases: StudyPhase[];
  resources: StudyResource[];
  reviewNotes: string;
}

/** 错误详情 */
export interface AgentError {
  code: string;
  message: string;
}

/**
 * 三端统一的响应包络：
 * - "plan"：生成成功，plan 非空
 * - "clarification"：输入信息不完整，clarifications 为澄清问题列表
 * - "error"：失败，error 非空
 */
export type PlanResponse =
  | { status: "plan"; plan: StudyPlan; clarifications: null; error: null }
  | {
      status: "clarification";
      plan: null;
      clarifications: string[];
      error: null;
    }
  | { status: "error"; plan: null; clarifications: null; error: AgentError };

/** Tauri command 抛出的错误（Rust 侧序列化为 { code, message, hint }） */
export interface CommandError {
  code: string;
  message: string;
  hint?: string;
}

/** 下拉框选项 */
export const LEVEL_OPTIONS: Array<{ value: Level; label: string }> = [
  { value: "beginner", label: "零基础入门" },
  { value: "intermediate", label: "有一定基础" },
  { value: "advanced", label: "进阶提升" },
];

/** 偏好方向的可选标签（预设，用户点选切换） */
export const INTEREST_PRESETS = [
  "Web 全栈",
  "后端 / 系统",
  "数据 / AI",
  "移动端",
  "DevOps",
  "安全",
  "游戏",
  "嵌入式",
] as const;

// ---------------------------------------------------------------------------
// 应用设置（AgentSettings）：三端协议对齐 settings.rs / schemas.py
// ---------------------------------------------------------------------------

/** LLM 提供商：五家独立 Key / 模型名，切换互不覆盖 */
export type LlmProvider =
  "openai" | "anthropic" | "deepseek" | "glm" | "opencode";

/** Agent 运行配置：设置页读写的载体，Rust 持久化到 settings.json 并热推给 Python */
export interface AgentSettings {
  llmProvider: LlmProvider;
  openaiApiKey: string;
  openaiModel: string;
  anthropicApiKey: string;
  anthropicModel: string;
  deepseekApiKey: string;
  deepseekModel: string;
  glmApiKey: string;
  glmModel: string;
  opencodeApiKey: string;
  opencodeModel: string;
  allowMockLlm: boolean;
}

/** 提供商展示元数据（设置页下拉与表单渲染） */
export interface ProviderMeta {
  id: LlmProvider;
  label: string;
  keyPlaceholder: string;
  modelPlaceholder: string;
  /** 接入方式说明 */
  endpointNote: string;
}

/** 提供商下拉选项（顺序即展示顺序） */
export const PROVIDER_OPTIONS: ProviderMeta[] = [
  {
    id: "openai",
    label: "通用 OpenAI",
    keyPlaceholder: "sk-…",
    modelPlaceholder: "gpt-4o-mini",
    endpointNote: "官方 OpenAI API",
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    keyPlaceholder: "sk-…",
    modelPlaceholder: "deepseek-chat",
    endpointNote: "官方 SDK · api.deepseek.com",
  },
  {
    id: "glm",
    label: "智谱 GLM",
    keyPlaceholder: "…",
    modelPlaceholder: "glm-5.3-flash",
    endpointNote: "官方集成 · open.bigmodel.cn",
  },
  {
    id: "opencode",
    label: "OpenCode Go",
    keyPlaceholder: "订阅 API Key…",
    modelPlaceholder: "grok-4.7",
    endpointNote: "OpenAI 兼容 · opencode.ai/zen/go",
  },
  {
    id: "anthropic",
    label: "Anthropic Claude",
    keyPlaceholder: "sk-ant-…",
    modelPlaceholder: "claude-3-5-sonnet-latest",
    endpointNote: "官方 SDK",
  },
];

/** 空白设置（默认值，与 Rust with_defaults / Python Settings 默认对齐） */
export function defaultAgentSettings(): AgentSettings {
  return {
    llmProvider: "openai",
    openaiApiKey: "",
    openaiModel: "gpt-4o-mini",
    anthropicApiKey: "",
    anthropicModel: "claude-3-5-sonnet-latest",
    deepseekApiKey: "",
    deepseekModel: "deepseek-chat",
    glmApiKey: "",
    glmModel: "glm-5.3-flash",
    opencodeApiKey: "",
    opencodeModel: "grok-4.7",
    allowMockLlm: false,
  };
}

// ---------------------------------------------------------------------------
// 学习计划历史（云端持久化）：仅 Rust <-> 前端两端，不涉及 Python
// ---------------------------------------------------------------------------

/** 保存学习计划请求：id 与保存时间由 Rust 端生成 */
export interface SavePlanInput {
  title: string;
  goal: string;
  overview: string;
  level: string;
  weeks: number;
  plan: StudyPlan;
}

/** 学习计划历史记录（完整行：元数据 + 整份计划） */
export interface PlanRecord {
  id: string;
  title: string;
  goal: string;
  overview: string;
  level: string;
  weeks: number;
  favorite: boolean;
  createdAt: string;
  plan: StudyPlan;
}

/** 计划列表行（不含整份计划，列表页轻量加载） */
export interface PlanSummary {
  id: string;
  title: string;
  goal: string;
  overview: string;
  level: string;
  weeks: number;
  favorite: boolean;
  createdAt: string;
}
