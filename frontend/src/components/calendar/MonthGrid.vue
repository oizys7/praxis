<script setup lang="ts">
/**
 * MonthGrid：月视图日历（6 行 × 7 列，周一起始）——「微型日程表」版式。
 * - 每格即当日计划：日期 + 完成点 + 至多两枚章事件块（agenda 式），
 *   溢出以「+N」示意；格子更高，直接在月历上读当天安排
 * - 语义映射沿用自习室纸面：完成章块 = 书绿浅底 / 待学章块 = 墨蓝描边；
 *   今日 = 墨蓝描边圈；非本月补位格做淡出处理
 * - 每格是 button：点击钻取当天；aria-label 形如
 *   「10 月 14 日（今日），共 3 章已完成 2」——状态不只靠颜色传达
 * - 数据由宿主注入（stats：日聚合计数；events：日程明细），无数据源依赖
 */
import { computed } from "vue";
import { dayKey, type DayStat } from "@/composables/useStudyAllocation";

/** 格内事件块（宿主由真计划分配聚合） */
export interface CalendarEvent {
  /** 所属计划标题（aria 完整名，视觉上省略） */
  planTitle: string;
  chapterName: string;
  done: boolean;
}

const props = withDefaults(
  defineProps<{
    year: number;
    month: number;
    /** 日子键 → 当日统计（宿主聚合自真计划分配） */
    stats: Map<string, DayStat>;
    /** 日子键 → 当日章节事件块 */
    events?: Map<string, CalendarEvent[]>;
  }>(),
  { events: () => new Map() },
);

const emit = defineEmits<{ (e: "select-day", date: Date): void }>();

const EMPTY_STAT: DayStat = { total: 0, done: 0 };

/** 星期表头（周一起始） */
const WEEK_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

/** 格内最多展示的事件块数，超出折为 +N */
const MAX_VISIBLE_EVENTS = 2;

/** 单个日历格的渲染数据 */
interface CalendarCell {
  /** 日期数字（1 起始）；非本月补位格为 null */
  day: number | null;
  /** 完整日期（可点击格才有） */
  date?: Date;
  /** 当天安排章数 */
  total: number;
  /** 当天已完成章数 */
  done: number;
  /** 是否今日 */
  isToday: boolean;
  /** 格内事件块（已截断至可见数） */
  events: CalendarEvent[];
  /** 比可见数多出的事件数 */
  overflow: number;
}

/** 固定 6 行 × 7 列 = 42 格，当月不足前后以 null 补位格填满 */
const cells = computed<CalendarCell[]>(() => {
  const daysInMonth = new Date(props.year, props.month + 1, 0).getDate();
  const firstWeekday = (new Date(props.year, props.month, 1).getDay() + 6) % 7;
  const today = dayKey(new Date());

  const list: CalendarCell[] = [];
  for (let i = 0; i < firstWeekday; i++) {
    list.push({
      day: null,
      total: 0,
      done: 0,
      isToday: false,
      events: [],
      overflow: 0,
    });
  }
  for (let d = 1; d <= daysInMonth; d++) {
    const date = new Date(props.year, props.month, d);
    const key = dayKey(date);
    const stat = props.stats.get(key) ?? EMPTY_STAT;
    const all = props.events?.get(key) ?? [];
    list.push({
      day: d,
      date,
      total: stat.total,
      done: stat.done,
      isToday: key === today,
      events: all.slice(0, MAX_VISIBLE_EVENTS),
      overflow: Math.max(0, all.length - MAX_VISIBLE_EVENTS),
    });
  }
  while (list.length < 42) {
    list.push({
      day: null,
      total: 0,
      done: 0,
      isToday: false,
      events: [],
      overflow: 0,
    });
  }
  return list;
});

/** 日历格 aria-label：日期 + 安排/完成数（读屏可感知当日进度） */
function cellLabel(cell: CalendarCell): string {
  const base = `${props.month + 1} 月 ${cell.day} 日`;
  const detail = `共 ${cell.total} 章，已完成 ${cell.done}`;
  if (cell.isToday) return `${base}（今日），${detail}`;
  return `${base}，${detail}`;
}
</script>

<template>
  <div>
    <!-- 星期表头（周一起始） -->
    <div class="mb-2 grid grid-cols-7 gap-2" aria-hidden="true">
      <div
        v-for="label in WEEK_LABELS"
        :key="label"
        class="text-center text-xs font-medium text-muted-foreground"
      >
        {{ label }}
      </div>
    </div>

    <!-- 6 × 7 月历网格 -->
    <div
      class="grid grid-cols-7 gap-2"
      role="group"
      :aria-label="`${year} 年 ${month + 1} 月日历`"
    >
      <template v-for="(cell, i) in cells" :key="i">
        <!-- 非本月补位格：纯占位 -->
        <div
          v-if="cell.day === null"
          class="min-h-[5.5rem] rounded-lg bg-muted/20"
          aria-hidden="true"
        />
        <button
          v-else
          type="button"
          class="flex min-h-[5.5rem] cursor-pointer flex-col items-start gap-1 rounded-lg border p-1.5 text-left transition-colors duration-200 hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="
            cell.isToday
              ? 'border-primary/60 bg-primary/[0.05] ring-2 ring-inset ring-primary/50'
              : cell.total > 0
                ? 'border-border bg-card shadow-[inset_0_0_0_1px_hsl(var(--border))]'
                : 'border-transparent bg-muted/30'
          "
          :aria-label="cellLabel(cell)"
          @click="cell.date && emit('select-day', cell.date)"
        >
          <!-- 日期 + 完成点 -->
          <span class="flex w-full items-center gap-1.5">
            <span
              class="text-xs tabular-nums"
              :class="
                cell.isToday
                  ? 'font-semibold text-primary'
                  : cell.total === 0
                    ? 'text-muted-foreground/70'
                    : ''
              "
            >
              {{ cell.day }}
            </span>
            <span
              v-if="cell.total > 0"
              class="h-1.5 w-1.5 rounded-full"
              :class="
                cell.done === cell.total
                  ? 'bg-success'
                  : 'border border-muted-foreground/50'
              "
              aria-hidden="true"
            />
          </span>
          <!-- 章事件块（至多两枚，溢出折叠） -->
          <ul v-if="cell.events.length > 0" class="w-full space-y-1">
            <li
              v-for="(ev, ei) in cell.events"
              :key="ei"
              class="truncate rounded-md border px-1.5 py-0.5 text-[11px] leading-tight"
              :class="
                ev.done
                  ? 'border-transparent bg-success/20 text-success'
                  : 'border-primary/30 bg-primary/[0.08] text-foreground/90'
              "
              :title="`${ev.planTitle} · ${ev.chapterName}`"
            >
              {{ ev.chapterName }}
            </li>
            <li
              v-if="cell.overflow > 0"
              class="pl-1 text-[10px] leading-none tabular-nums text-muted-foreground"
            >
              还有 {{ cell.overflow }} 章
            </li>
          </ul>
        </button>
      </template>
    </div>
  </div>
</template>
