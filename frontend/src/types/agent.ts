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
  /**
   * 每周学习日（1=周一..7=周日，需求 5）：缺省 = 全周；仅作日历分配元数据，
   * 不走 LLM 提示词。Rust 侧缺省/ null 归一为全周数组转发 Python。
   */
  studyDays?: number[];
}

/** 阶段收尾项目及其验收标准 */
export interface PhaseProject {
  name: string;
  description: string;
  /** 项目背景与动机（旧云端数据无此字段） */
  background?: string;
  /** 建议实施步骤（有序，旧云端数据无此字段） */
  steps?: string[];
  /** 预计投入学时（旧云端数据无此字段） */
  estimatedHours?: number;
  acceptanceCriteria: string[];
}

/**
 * 两级状态机的状态位（阶段级与章节级共用）：
 * - pending：目标行已置「生成中」（重生成刚发起——骨架章或重生成中）；
 * - complete：生成收尾完成（成功入库，写入侧由 contentMd 定价）；
 * - failed：生成失败终态。
 * undefined = 旧 plan_json / 缺省流式载荷（无徽标语义）。
 */
export type PhaseStatus = "pending" | "complete" | "failed";

/** 阶段内的学习章节：完整 markdown 正文（status 见 PhaseStatus；图示内联 ```mermaid 码块） */
export interface Chapter {
  title: string;
  summary: string;
  /** 章节 markdown 正文全文；空串表示有意义的空正文（骨架章 / 生成中） */
  contentMd: string;
  status?: PhaseStatus;
}

/**
 * 大纲阶段载荷（outline 事件的 phases 载荷 / RegenOutline.phases 共用）：
 * 仅骨架级字段，与 src-tauri/src/protocol.rs::OutlinePhasePayload 逐字对齐；
 * seed_outline 按此播种骨架行。
 */
export interface OutlinePhasePayload {
  name: string;
  weekRange: string;
  chapterTitles: string[];
  /** 项目简介仅 regenerate_chapter 的重建请求载荷携带；outline 事件恒缺省 */
  projectBrief?: string;
}

/** 学习计划中的一个阶段：章节 / 实操 / 作业 / 项目（status 见 PhaseStatus） */
export interface StudyPhase {
  name: string;
  weekRange: string;
  objectives: string[];
  /** 章节列表（章节化新结构） */
  chapters?: Chapter[];
  practice: string[];
  assignments: string[];
  project: PhaseProject;
  status?: PhaseStatus;
}

/**
 * Rust -> 前端的生成进度事件本体（payload）。
 * 与 src-tauri/src/protocol.rs 的 PlanProgressEvent serde 序列化逐字对齐
 * （tag = "type" + camelCase 字段，见该文件 progress_event_uses_type_tag 测试）；
 * outline 的 phases/overview 为大纲骨架载荷（渐进式骨架渲染，可缺省兼容旧载荷）。
 *
 * 两条消费路径（任务 22/23）同用该形状：
 * - generate 链路：全局事件 'plan-progress' 的 payload（见 PlanProgressEnvelope）；
 * - regenerate 双命令：仍经 invoke 的 tauri Channel 逐事件推送。
 */
export type PlanProgressEvent =
  | {
      type: "outline";
      phaseCount: number;
      title: string;
      overview?: string;
      phases?: OutlinePhasePayload[];
    }
  | { type: "phaseDone"; index: number; total: number }
  /** 单章重生成完成（task-B）：与 protocol.rs ChapterDone 变体逐字对齐 */
  | { type: "chapterDone"; phaseIndex: number; chapterIndex: number }
  | { type: "storageIssue"; message: string };

/**
 * 全局进度事件 'plan-progress' 的载荷信封：与
 * src-tauri/src/protocol.rs::PlanProgressEnvelope 逐字对齐（camelCase wire，
 * 见该文件 plan_progress_envelope_wraps_payload 测试）。
 * planId 与 generate_plan 命令返回值一致；kind 恒 "plan"（regen 不走该事件）。
 */
export type PlanProgressEnvelope = {
  /** 归属计划 id（generate_plan 预置行 id） */
  planId: string;
  /** 链路种类：generate 恒 "plan" */
  kind: string;
  /** 进度事件本体（outline / phaseDone / chapterDone / storageIssue 四变体） */
  payload: PlanProgressEvent;
};

/**
 * generate_plan 命令的返回值：提交即返（不等终态），字段与
 * src-tauri/src/commands.rs::PlanSubmitResult 对齐（camelCase wire）。
 * 生成进度经 'plan-progress' 全局事件异步推送（见 PlanProgressEnvelope）。
 */
export type PlanSubmitResult = {
  planId: string;
};

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

/** 学习计划历史记录（完整行：元数据 + 整份计划）。生成成功即由 Rust 自动入库，前端不再手动保存 */
export interface PlanRecord {
  id: string;
  title: string;
  goal: string;
  /**
   * 行级生成状态：generating | complete | failed（数据库字面量原样透传，
   * 无 camel 歧义不加 rename；与 src-tauri/src/protocol.rs::PlanRecord.status
   * 逐字对齐——wire 单测 plan_summary_and_record_carry_row_status_verbatim
   * 为权威；旧 wire 缺该字段时 Rust 侧 serde default 补空串，前端按未知值
   * 不加标处理）
   */
  status: string;
  overview: string;
  level: string;
  weeks: number;
  /**
   * 每周学习日（1=周一..7=周日，Task E）：列默认全周，读回恒为数组；
   * 旧 wire 缺省空数组（前端按全周兑底）
   */
  studyDays: number[];
  /** 已打卡日（当日已学，'YYYY-MM-DD' 升序，Task E）；旧 wire 缺省空数组 */
  dayMarks: string[];
  favorite: boolean;
  createdAt: string;
  plan: StudyPlan;
}

/** 计划列表行（不含整份计划，列表页轻量加载） */
export interface PlanSummary {
  id: string;
  title: string;
  goal: string;
  /** 行级生成状态（同 PlanRecord.status，数据库字面量原样透传，wire 单测为权威） */
  status: string;
  overview: string;
  level: string;
  weeks: number;
  /** 每周学习日（1=周一..7=周日，Task E，同 PlanRecord.studyDays） */
  studyDays: number[];
  favorite: boolean;
  createdAt: string;
}
