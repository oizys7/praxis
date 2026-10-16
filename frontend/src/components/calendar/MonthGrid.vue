<script setup lang="ts">
/**
 * MonthGrid：月视图日历（6 行 × 7 列，周一起始）。
 * - 格内三层信息：日期数字（tabular-nums）+ 完成度点态 + 当日章数小字
 * - 点态沿用三态语义：完成 = 书绿实心（bg-success）/ 未完成 = 沙框空心 /
 *   今日 = 墨蓝描边圈（ring-primary）
 * - 每格是 button：点击钻取当天；aria-label 形如
 *   「10 月 14 日，共 3 章，已完成 2」——状态不只靠颜色传达
 * - 数据由宿主注入（dayKey → DayStat，Task E 真数据分配聚合），本组件无数据源依赖
 */
import { computed } from "vue";
import { dayKey, type DayStat } from "@/composables/useStudyAllocation";

const props = defineProps<{
  year: number;
  month: number;
  /** 日子键 → 当日统计（宿主聚合自真计划分配） */
  stats: Map<string, DayStat>;
}>();

const emit = defineEmits<{ (e: "select-day", date: Date): void }>();

const EMPTY_STAT: DayStat = { total: 0, done: 0 };

/** 星期表头（周一起始） */
const WEEK_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

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
}

/** 固定 6 行 × 7 列 = 42 格，当月不足前后以 null 补位格填满 */
const cells = computed<CalendarCell[]>(() => {
  const daysInMonth = new Date(props.year, props.month + 1, 0).getDate();
  const firstWeekday = (new Date(props.year, props.month, 1).getDay() + 6) % 7;
  const today = dayKey(new Date());

  const list: CalendarCell[] = [];
  for (let i = 0; i < firstWeekday; i++) {
    list.push({ day: null, total: 0, done: 0, isToday: false });
  }
  for (let d = 1; d <= daysInMonth; d++) {
    const date = new Date(props.year, props.month, d);
    const stat = props.stats.get(dayKey(date)) ?? EMPTY_STAT;
    list.push({
      day: d,
      date,
      total: stat.total,
      done: stat.done,
      isToday: dayKey(date) === today,
    });
  }
  while (list.length < 42) {
    list.push({ day: null, total: 0, done: 0, isToday: false });
  }
  return list;
});

/** 日历格 aria-label：日期 + 安排/完成数（读屏可感知当日进度） */
function cellLabel(cell: CalendarCell): string {
  const base = `${props.month + 1} 月 ${cell.day} 日`;
  if (cell.isToday)
    return `${base}（今日），共 ${cell.total} 章，已完成 ${cell.done}`;
  return `${base}，共 ${cell.total} 章，已完成 ${cell.done}`;
}
</script>

<template>
  <div>
    <!-- 星期表头（周一起始） -->
    <div class="mb-2 grid grid-cols-7 gap-1.5" aria-hidden="true">
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
      class="grid grid-cols-7 gap-1.5"
      role="group"
      :aria-label="`${year} 年 ${month + 1} 月日历`"
    >
      <template v-for="(cell, i) in cells" :key="i">
        <!-- 非本月补位格：纯占位 -->
        <div
          v-if="cell.day === null"
          class="aspect-square rounded-lg"
          aria-hidden="true"
        />
        <button
          v-else
          type="button"
          class="flex aspect-square cursor-pointer flex-col items-center justify-center gap-1 rounded-lg border border-transparent transition-colors duration-200 hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="
            cell.isToday
              ? 'font-semibold text-primary ring-[1.5px] ring-inset ring-primary'
              : ''
          "
          :aria-label="cellLabel(cell)"
          @click="cell.date && emit('select-day', cell.date)"
        >
          <span class="text-sm tabular-nums">{{ cell.day }}</span>
          <!-- 完成度点态：书绿实心（全完成）/ 沙框空心（未完成） -->
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
          <!-- 当日章数小字 -->
          <span
            v-if="cell.total > 0"
            class="text-[10px] leading-none tabular-nums text-muted-foreground"
          >
            {{ cell.total }} 章
          </span>
        </button>
      </template>
    </div>
  </div>
</template>
