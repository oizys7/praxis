<script setup lang="ts">
/**
 * YearHeatmap：年视图热力图（GitHub contributions 风格的纵向变体）。
 * - 12 行（1 月..12 月）× 7 列（周一起始）：每格代表一天，空缺以占位补齐
 * - 每格五档色阶（语义令牌阶梯）：无安排/未学 bg-success/5 → 低 /20 →
 *   中 /45 → 高 /70 → 当日全满 bg-success，档位语义唯一来源是
 *   calendarMocks.intensityLevel
 * - 每格是 button：点击钻取当天（title + aria-label 双通道，不只靠颜色
 *   编码传达强度——色弱用户可从文字获得同等信息）
 */
import { computed } from "vue";
import { dayKey, dayStat, intensityLevel } from "@/calendarMocks";

const props = defineProps<{ year: number }>();

const emit = defineEmits<{ (e: "select-day", date: Date): void }>();

/** 五档色阶 class（与图例共用；索引 = IntensityLevel） */
const LEVEL_CLASSES = [
  "bg-success/5",
  "bg-success/20",
  "bg-success/45",
  "bg-success/70",
  "bg-success",
] as const;

/** 星期表头（周一起始，与列位对齐） */
const WEEK_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

/** 单格渲染数据 */
interface HeatCell {
  date: Date;
  /** 五档强度 */
  level: number;
  /** aria 用：安排章数 */
  total: number;
  /** aria 用：完成章数 */
  done: number;
}

/** 12 个月的行数据：行首月份标签 + 当月格子（周一首偏移，首尾 null 占位） */
const rows = computed<{ month: number; cells: (HeatCell | null)[] }[]>(() => {
  const list: { month: number; cells: (HeatCell | null)[] }[] = [];
  for (let m = 0; m < 12; m++) {
    const daysInMonth = new Date(props.year, m + 1, 0).getDate();
    const firstWeekday = (new Date(props.year, m, 1).getDay() + 6) % 7;
    const cells: (HeatCell | null)[] = [];
    for (let i = 0; i < firstWeekday; i++) cells.push(null);
    for (let d = 1; d <= daysInMonth; d++) {
      const date = new Date(props.year, m, d);
      const stat = dayStat(date);
      cells.push({
        date,
        level: intensityLevel(stat),
        total: stat.total,
        done: stat.done,
      });
    }
    list.push({ month: m, cells });
  }
  return list;
});

/** 「今天」的日子键（描边圈高亮 + aria 标注用，本地时区安全） */
const todayKey = dayKey(new Date());

/** 每格 aria-label / title：「10 月 14 日，3 章中已完成 2」式 */
function cellLabel(cell: HeatCell): string {
  const base = `${cell.date.getMonth() + 1} 月 ${cell.date.getDate()} 日`;
  if (cell.total === 0) return `${base}，无学习安排`;
  return `${base}，${cell.total} 章中已完成 ${cell.done}`;
}
</script>

<template>
  <div>
    <!-- 星期表头（列位对齐：留出行首月份标签的宽度） -->
    <div class="mb-1.5 flex items-center gap-[3px]" aria-hidden="true">
      <span class="w-9 shrink-0" />
      <span
        v-for="label in WEEK_LABELS"
        :key="label"
        class="w-3.5 text-center text-[10px] leading-none text-muted-foreground"
      >
        {{ label }}
      </span>
    </div>

    <!-- 12 行 × 7 列 -->
    <div role="group" :aria-label="`${year} 年学习热力图`">
      <div
        v-for="row in rows"
        :key="row.month"
        class="mb-[3px] flex items-center gap-[3px]"
      >
        <span
          class="w-9 shrink-0 text-right text-xs tabular-nums text-muted-foreground"
        >
          {{ row.month + 1 }} 月
        </span>
        <template v-for="(cell, i) in row.cells" :key="i">
          <!-- 非本月占位：保持列对齐 -->
          <span v-if="cell === null" class="h-3.5 w-3.5" aria-hidden="true" />
          <button
            v-else
            type="button"
            class="h-3.5 w-3.5 cursor-pointer rounded-[3px] transition-colors duration-200 hover:ring-1 hover:ring-primary/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="[
              LEVEL_CLASSES[cell.level],
              dayKey(cell.date) === todayKey
                ? 'ring-[1.5px] ring-inset ring-primary'
                : '',
            ]"
            :title="cellLabel(cell)"
            :aria-label="`${cellLabel(cell)}${dayKey(cell.date) === todayKey ? '（今日）' : ''}，点击查看当天安排`"
            @click="emit('select-day', cell.date)"
          />
        </template>
      </div>
    </div>

    <!-- 图例：五档色阶（数值型 legend，非纯色编码） -->
    <div
      class="mt-4 flex items-center justify-end gap-1.5 text-xs text-muted-foreground"
    >
      <span>少</span>
      <span
        v-for="(cls, i) in LEVEL_CLASSES"
        :key="i"
        class="h-3 w-3 rounded-[3px]"
        :class="cls"
        aria-hidden="true"
      />
      <span>多</span>
    </div>
  </div>
</template>
