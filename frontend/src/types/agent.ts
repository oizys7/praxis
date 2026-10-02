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
