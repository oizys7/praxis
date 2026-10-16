/**
 * 学习日自动分配纯函数集（Task E 需求 4）。
 *
 * 职责：把 plan（weeks / studyDays / phases[].weekRange / chapters）确定性
 * 分配到具体日期——**不落库不存储**，分配表即时计算，视图层每次重算，
 * determinism 是唯一不变式（同一输入恒得同一输出）。
 *
 * 算法（单公式双覆盖）：阶段区间内的学习日序列 slots（第 w1..w2 周，每周取
 * studyDays 对应的日期），章 i 落 slots[floor(i*m/n)]（m=|slots|，n=章数）：
 * - n ≤ m：各章占不同学习日（均匀散列，一日起一章）；
 * - n > m：相邻章自然碰撞同日（多章并排）。
 *
 * 兜底铁则（视图层防御，宽容处理）：
 * - plan.studyDays 缺省 / 空数组 / 全越界 → 兜底全周（周一..周日）；
 * - weekRange 解析失败 → 该阶段章节按章数均铺到全期学习日序列（第 1..weeks 周）；
 * - createdAt 无效 → 计划周锚兜底今天所在周的周一；
 * - weekRange 周号越界 → clamp 到 [1, weeks]，w1>w2 时交换。
 *
 * 周锚约定：计划第 1 周从 createdAt 所在周的周一开始（第 k 周周 d =
 * planStart + (k-1)*7 + (d-1) 天）；日期一律本地时区（dayKey 安全）。
 */
import type { PlanRecord, StudyPhase } from "@/types/agent";

// ---------------------------------------------------------------------------
// 类型（日历视图共用）
// ---------------------------------------------------------------------------

/** 热力图强度五档：0=无安排/未学 → 4=当日全满（语义唯一来源，图例共用） */
export type IntensityLevel = 0 | 1 | 2 | 3 | 4;

/** 阶段徽章状态：已完成 / 进行中 / 未开始（进度卡用） */
export type PhaseBadgeStatus = "done" | "active" | "todo";

/** 单日学习统计（月历点态与年热力图共用的最小单元） */
export interface DayStat {
  /** 当天安排章数 */
  total: number;
  /** 当天已完成章数（口径 = 章节 status=complete；恒 <= total） */
  done: number;
}

/** 单日分配条目（日抽屉一行；含完成与打卡两个正交语义位） */
export interface DayPlan {
  planId: string;
  planTitle: string;
  phaseIndex: number;
  phaseName: string;
  chapterIndex: number;
  chapterTitle: string;
  /** 'YYYY-MM-DD'（本地时区安全） */
  date: string;
  /** 章节完成：章 status=complete */
  done: boolean;
  /** 该日已打卡（当日已学 marks 含此日） */
  marked: boolean;
}

// ---------------------------------------------------------------------------
// 基础工具
// ---------------------------------------------------------------------------

/** 本地时区安全的日子键（YYYY-MM-DD） */
export function dayKey(date: Date): string {
  const m = `${date.getMonth() + 1}`.padStart(2, "0");
  const d = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

/** 全周学习日（周一..周日） */
function fullWeek(): number[] {
  return [1, 2, 3, 4, 5, 6, 7];
}

/**
 * 归一化每周学习日：去重升序、丢弃越界项；空 / 全越界 / 缺省兜底全周
 * （与 Python 校验层 / Rust DB 层的「拒绝非法」不同，视图层只做宽容兜底）
 */
export function normalizeStudyDays(days?: number[] | null): number[] {
  const cleaned = [...new Set(days ?? [])]
    .filter((d) => Number.isInteger(d) && d >= 1 && d <= 7)
    .sort((a, b) => a - b);
  return cleaned.length > 0 ? cleaned : fullWeek();
}

/**
 * 解析 weekRange 文本：「第 X-Y 周」/「第X-Y周」/「第 X - Y 周」/「第 X 周」
 * （含全角破折号与「至」）；解析不出返回 null。
 */
export function parseWeekRange(text: string): [number, number] | null {
  const span = text.match(/第\s*(\d+)\s*(?:[-–—~至]\s*(\d+)\s*)?周/);
  if (!span) return null;
  const w1 = Number(span[1]);
  const w2 = span[2] !== undefined ? Number(span[2]) : w1;
  if (!Number.isInteger(w1) || !Number.isInteger(w2)) return null;
  return w1 <= w2 ? [w1, w2] : [w2, w1];
}

/** 给定日期所在周的周一（本地时区零点） */
function mondayOf(date: Date): Date {
  const d = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d;
}

/**
 * 计划周锚：createdAt 所在周的周一（第 1 周起点）；
 * createdAt 无效（NaN）兜底今天所在周的周一。
 */
export function planStartDate(createdAt: string): Date {
  const parsed = new Date(createdAt);
  return Number.isNaN(parsed.getTime()) ? mondayOf(new Date()) : mondayOf(parsed);
}

/** 第 fromWeek..toWeek 周（1-based）内每周学习日的 dayKey 序列（升序） */
function studySlots(
  start: Date,
  weeks: number,
  studyDays: number[],
  fromWeek: number,
  toWeek: number,
): string[] {
  // clamp 到 [1, weeks]（weekRange 由 LLM 产出，防御越界）
  const w1 = Math.min(Math.max(fromWeek, 1), weeks);
  const w2 = Math.min(Math.max(toWeek, 1), weeks);
  const slots: string[] = [];
  for (let w = w1; w <= w2; w++) {
    for (const d of studyDays) {
      const date = new Date(start);
      date.setDate(date.getDate() + (w - 1) * 7 + (d - 1));
      slots.push(dayKey(date));
    }
  }
  return slots;
}

// ---------------------------------------------------------------------------
// 分配主入口
// ---------------------------------------------------------------------------

/**
 * 把整份计划的章节确定性分配到具体日期（纯函数，不落库）。
 * - 每 phase 用其 weekRange 区间内的学习日序列铺 chapters；
 * - weekRange 解析失败的 phase，章节归入兜底组统一铺全期学习日；
 * - done 口径 = 章 status=complete；marked = marks.has(date)（打卡独立成位）。
 */
export function computeDayAssignments(
  plan: PlanRecord,
  marks: Set<string>,
): DayPlan[] {
  const studyDays = normalizeStudyDays(plan.studyDays);
  const start = planStartDate(plan.createdAt);
  const weeks = Math.max(Math.round(plan.weeks) || 1, 1);
  const fullPeriodSlots = studySlots(start, weeks, studyDays, 1, weeks);

  const result: DayPlan[] = [];
  const fallbackChapters: Array<{
    phaseIndex: number;
    phaseName: string;
    chapterIndex: number;
    chapter: { title: string; status?: string };
  }> = [];

  plan.plan.phases.forEach((phase, phaseIndex) => {
    const chapters = phase.chapters ?? [];
    const range = parseWeekRange(phase.weekRange);
    if (range === null) {
      // 解析失败：归入兜底组，统一均铺全期学习日
      chapters.forEach((chapter, chapterIndex) => {
        fallbackChapters.push({ phaseIndex, phaseName: phase.name, chapterIndex, chapter });
      });
      return;
    }
    spreadToSlots(result, plan.id, plan.title, phaseIndex, phase.name, chapters, studySlots(start, weeks, studyDays, range[0], range[1]), marks);
  });

  if (fallbackChapters.length > 0) {
    spreadToSlots(
      result,
      plan.id,
      plan.title,
      fallbackChapters[0].phaseIndex,
      fallbackChapters[0].phaseName,
      fallbackChapters.map((c) => c.chapter),
      fullPeriodSlots,
      marks,
    );
    // 回填每个章自己的阶段索引/名（首章之外的可能来自不同阶段）
    const from = result.length - fallbackChapters.length;
    fallbackChapters.forEach((c, i) => {
      const row = result[from + i];
      row.phaseIndex = c.phaseIndex;
      row.phaseName = c.phaseName;
      row.chapterIndex = c.chapterIndex;
    });
  }

  return result;
}

/** 章节序列均匀铺到 slots（floor(i*m/n) 单公式；空 slots / 空章直接返回） */
function spreadToSlots(
  out: DayPlan[],
  planId: string,
  planTitle: string,
  phaseIndex: number,
  phaseName: string,
  chapters: Array<{ title: string; status?: string }>,
  slots: string[],
  marks: Set<string>,
): void {
  const n = chapters.length;
  const m = slots.length;
  if (n === 0 || m === 0) return;
  chapters.forEach((c, i) => {
    const date = slots[Math.floor((i * m) / n)];
    out.push({
      planId,
      planTitle,
      phaseIndex,
      phaseName,
      chapterIndex: i,
      chapterTitle: c.title,
      date,
      done: c.status === "complete",
      marked: marks.has(date),
    });
  });
}

// ---------------------------------------------------------------------------
// 聚合与强度（月/年视图色阶与进度统计的唯一口径）
// ---------------------------------------------------------------------------

/** 按日聚合分配条目：total=安排章数，done=章完成数（status=complete 口径） */
export function aggregateDayStats(assignments: DayPlan[]): Map<string, DayStat> {
  const stats = new Map<string, DayStat>();
  for (const a of assignments) {
    const stat = stats.get(a.date) ?? { total: 0, done: 0 };
    stat.total += 1;
    if (a.done) stat.done += 1;
    stats.set(a.date, stat);
  }
  return stats;
}

/**
 * 强度五档映射（当天完成比例）：
 * 0=无安排或未学 → 1=低(<1/3) → 2=中(<2/3) → 3=高(<满) → 4=满(全部完成)
 */
export function intensityLevel(stat: DayStat): IntensityLevel {
  if (stat.total === 0 || stat.done === 0) return 0;
  const ratio = stat.done / stat.total;
  if (ratio < 1 / 3) return 1;
  if (ratio < 2 / 3) return 2;
  if (ratio < 1) return 3;
  return 4;
}

/**
 * 阶段徽章三态（进度卡）：全部章 complete → done；有任一章 complete（或
 * 阶段行 status=complete 的非空阶段）→ active；其余 → todo。
 * 无章节的阶段恒 todo（骨架期不可判进行）。
 */
export function phaseBadgeStatus(phase: StudyPhase): PhaseBadgeStatus {
  const chapters = phase.chapters ?? [];
  if (chapters.length === 0) return "todo";
  if (chapters.every((c) => c.status === "complete")) return "done";
  if (chapters.some((c) => c.status === "complete")) return "active";
  return "todo";
}
