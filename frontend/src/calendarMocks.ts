/**
 * 日历页 mock 数据中心（Task D 视觉壳专用，Task E 将替换为真数据）。
 *
 * ⚠ 对接说明：本文件所有导出的类型与函数签名即 Task E 的对接面——
 * 替换为 `get_plan` / `list_marks` 真 IPC 数据时保持签名不变、删实现即可：
 * - `MOCK_PLANS`        → get_plan 返回的进行中计划 + 进度聚合
 * - `dayStat()`         → list_marks 按日聚合（当天已学/待学章数）
 * - `dayAssignments()`  → 指定日期的分配章节清单（阶段/章节/时段/完成态）
 * - `intensityLevel()`  → 纯映射函数，预计可原样保留
 *
 * 设计约定：
 * - 全部数据确定性生成（以日期字符串 hash 为种子，无随机数）：任何一天打开、
 *   翻到任何月份/年份，视觉都成立且跨组件一致——年热力图、月历点态、日抽屉
 *   三层同源于 `dayStat()`，不会出现互相矛盾
 * - 「今天」为真实系统日期锚点：未来日期一律 done=0（还没学），今天之前的
 *   日期按 hash 生成 0..total 的完成数
 */

// ---------------------------------------------------------------------------
// 类型定义（Task E 对接面）
// ---------------------------------------------------------------------------

/** 阶段徽章状态：已完成 / 进行中 / 未开始 */
export type PhaseStatus = "done" | "active" | "todo";

/** 阶段徽章（计划卡上的序列） */
export interface PhaseBadgeMock {
  /** 阶段名（不含序号，如「基础巩固」） */
  name: string;
  status: PhaseStatus;
}

/** 第一屏「进行中的学习计划」进度卡（mock 期 completedChapters 为静态值，
 * Task E 替换后应由 list_marks 聚合计算驱动） */
export interface PlanProgressMock {
  id: string;
  /** 计划标题 */
  title: string;
  /** 水平档位（入门/进阶） */
  level: string;
  /** 总章数 */
  totalChapters: number;
  /** 已完成章数 */
  completedChapters: number;
  /** 距截止剩余天数 */
  daysLeft: number;
  /** 阶段徽章序列 */
  phases: PhaseBadgeMock[];
}

/** 单日分配条目（日抽屉里的一行） */
export interface DayAssignmentMock {
  /** 所属计划标题 */
  planTitle: string;
  /** 阶段名（含序号，如「阶段一 · 基础巩固」） */
  phaseName: string;
  /** 章节/任务名 */
  chapterName: string;
  /** 预估时段 */
  timeSlot: string;
  /** 完成态 */
  done: boolean;
}

/** 单日学习统计（月历点态与年热力图共用的最小单元） */
export interface DayStat {
  /** 当天安排章数 */
  total: number;
  /** 当天已完成章数（恒 <= total；未来日期恒 0） */
  done: number;
}

/** 热力图强度五档：0=无安排/未学 → 4=当日全满 */
export type IntensityLevel = 0 | 1 | 2 | 3 | 4;

// ---------------------------------------------------------------------------
// 静态 mock：进行中的计划 + 章节池
// ---------------------------------------------------------------------------

/** 第一屏计划卡数据（3 张：分别为剩余充裕 / 正常推进 / 临近截止三种样态） */
export const MOCK_PLANS: PlanProgressMock[] = [
  {
    id: "vue3",
    title: "Vue 3 全栈进阶 30 天",
    level: "进阶",
    totalChapters: 30,
    completedChapters: 18,
    daysLeft: 9,
    phases: [
      { name: "基础巩固", status: "done" },
      { name: "组件化实战", status: "active" },
      { name: "综合项目", status: "todo" },
    ],
  },
  {
    id: "algo",
    title: "算法面试冲刺 60 天",
    level: "进阶",
    totalChapters: 48,
    completedChapters: 21,
    daysLeft: 26,
    phases: [
      { name: "数组与字符串", status: "done" },
      { name: "动态规划", status: "active" },
      { name: "图论专题", status: "todo" },
    ],
  },
  {
    id: "writing",
    title: "英语学术写作 21 天",
    level: "入门",
    totalChapters: 21,
    completedChapters: 19,
    daysLeft: 2,
    phases: [
      { name: "语法重建", status: "done" },
      { name: "论文精读", status: "active" },
      { name: "独立写作", status: "todo" },
    ],
  },
];

/** 阶段池条目 */
interface StagePool {
  phase: string;
  chapters: string[];
}

/** 各计划的阶段 × 章节名池（dayAssignments 从中确定性轮转取名；
 * 以计划 id 为键，Record 索引签名便于按 plan.id 取用） */
const CHAPTER_POOLS: Record<string, StagePool[]> = {
  vue3: [
    {
      phase: "阶段一 · 基础巩固",
      chapters: [
        "第 2 章 响应式系统原理",
        "第 3 章 计算属性与侦听器",
        "第 4 章 生命周期深度剖析",
      ],
    },
    {
      phase: "阶段二 · 组件化实战",
      chapters: [
        "第 9 节 组件通信与依赖注入",
        "第 11 节 组合式函数设计",
        "第 13 节 状态管理模式",
      ],
    },
    {
      phase: "阶段三 · 综合项目",
      chapters: ["项目周 · 后台管理联调", "项目周 · 性能优化与验收"],
    },
  ],
  algo: [
    {
      phase: "阶段一 · 数组与字符串",
      chapters: ["双指针与滑动窗口", "前缀和与差分", "字符串匹配 KMP"],
    },
    {
      phase: "阶段二 · 动态规划",
      chapters: ["背包问题九讲", "区间与状压 DP", "树形 DP 入门"],
    },
    {
      phase: "阶段三 · 图论专题",
      chapters: ["最短路三算法", "拓扑排序与 DAG", "并查集与最小生成树"],
    },
  ],
  writing: [
    {
      phase: "阶段一 · 语法重建",
      chapters: ["名词化与被动语态", "复杂句改造练习", "时态一致性自查"],
    },
    {
      phase: "阶段二 · 论文精读",
      chapters: [
        "精读：摘要与引言范式",
        "精读：方法段句式",
        "精读：讨论段逻辑衔接",
      ],
    },
    {
      phase: "阶段三 · 独立写作",
      chapters: ["仿写一段方法描述", "完成一篇短文初稿", "互评修改与定稿"],
    },
  ],
};

/** 预估时段池（与序号轮转配对） */
const TIME_SLOTS = [
  "09:00 – 10:30",
  "14:00 – 15:30",
  "16:00 – 17:30",
  "19:30 – 21:00",
];

// ---------------------------------------------------------------------------
// 确定性数据生成
// ---------------------------------------------------------------------------

/** FNV-1a 字符串 hash：同一日期永远得到同一整数（mock 一致性的根基） */
function hashString(input: string): number {
  let h = 2166136261;
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** 本地时区安全的日子键（YYYY-MM-DD） */
export function dayKey(date: Date): string {
  const m = `${date.getMonth() + 1}`.padStart(2, "0");
  const d = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

/** 当天零点的时间戳（本地时区；未来判定与「距今天数」都以它为锚） */
function todayStart(): number {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

/**
 * 单日学习统计：年热力图、月历点态、日抽屉三层 mock 的共同数据源。
 * - 未来日期：可以有安排（total>0）但一律 done=0
 * - 45 天前的远期：三成概率完全空档，营造真实学习节奏的稀疏感
 * - 周末：偏向安排 2 章以上（休息日集中学习）
 */
export function dayStat(date: Date): DayStat {
  const key = dayKey(date);
  const h = hashString(key);
  const start = todayStart();
  const isFuture = date.getTime() > start;
  const daysAgo = Math.floor((start - date.getTime()) / 86400000);

  let total = h % 5;
  if (daysAgo > 45 && h % 10 < 3) total = 0;
  const weekend = date.getDay() === 0 || date.getDay() === 6;
  if (weekend && total < 2) total = 2;

  const done =
    isFuture || total === 0 ? 0 : hashString(`${key}#done`) % (total + 1);
  return { total, done };
}

/** 指定日期的分配章节清单（前 done 条标记已完成，保证与 dayStat 同源一致） */
export function dayAssignments(date: Date): DayAssignmentMock[] {
  const stat = dayStat(date);
  // 以「纪元日数」为稳定日序，配合条目序号在池中轮转取名
  const dayIndex = Math.floor(date.getTime() / 86400000);
  const items: DayAssignmentMock[] = [];
  for (let i = 0; i < stat.total; i++) {
    const plan = MOCK_PLANS[(dayIndex + i) % MOCK_PLANS.length];
    const pool = CHAPTER_POOLS[plan.id];
    const stage = pool[(dayIndex + i) % pool.length];
    const chapter =
      stage.chapters[(dayIndex * 3 + i * 5) % stage.chapters.length];
    const slotIndex = (dayIndex + i * 2) % TIME_SLOTS.length;
    items.push({
      planTitle: plan.title,
      phaseName: stage.phase,
      chapterName: chapter,
      timeSlot: TIME_SLOTS[slotIndex],
      done: i < stat.done,
    });
  }
  // 按预估时段先后排序，抽屉里呈现一天的日程顺序
  return items.sort(
    (a, b) => TIME_SLOTS.indexOf(a.timeSlot) - TIME_SLOTS.indexOf(b.timeSlot),
  );
}

/**
 * 强度五档映射（当天已学/待学比例）：
 * 0=无安排或未学 → 1=低(<1/3) → 2=中(<2/3) → 3=高(<满) → 4=满(全部完成)
 * YearHeatmap 的色阶与图例共用此档位，语义只此一处。
 */
export function intensityLevel(stat: DayStat): IntensityLevel {
  if (stat.total === 0 || stat.done === 0) return 0;
  const ratio = stat.done / stat.total;
  if (ratio < 1 / 3) return 1;
  if (ratio < 2 / 3) return 2;
  if (ratio < 1) return 3;
  return 4;
}
