<script setup lang="ts">
/**
 * YearHeatmap：年视图热力图（GitHub contributions 经典横向形态）。
 * - 竖向 7 行 = 周一..周日，横向列 = 周；只显示「近一年」滚动窗口
 *   （今天往前推 364 天所在周的周一为本周起点，至本周为止）
 * - 月份标签悬在列上方：该列含当月 1 日即打「X 月」标签
 * - 每格五档色阶（语义令牌阶梯）：无安排/未学 bg-success/5 → 低 /20 →
 *   中 /45 → 高 /70 → 当日全满 bg-success，档位语义唯一来源是
 *   useStudyAllocation.intensityLevel
 * - 每格是 button：点击钻取当天（title + aria-label 双通道，不只靠颜色
 *   编码传达强度——色弱用户可从文字获得同等信息）
 * - 数据由宿主注入（dayKey → DayStat，真计划分配聚合）
 */
import { computed, ref } from "vue";
import {
  dayKey,
  intensityLevel,
  type DayStat,
} from "@/composables/useStudyAllocation";
import type { CalendarEvent } from "@/components/calendar/MonthGrid.vue";

const props = withDefaults(
  defineProps<{
    /** 日子键 → 当日统计（宿主聚合自真计划分配） */
    stats: Map<string, DayStat>;
    /** 日子键 → 当日章节事件块（悬浮卡任务清单） */
    events?: Map<string, CalendarEvent[]>;
  }>(),
  { events: () => new Map<string, CalendarEvent[]>() },
);

const emit = defineEmits<{ (e: "select-day", date: Date): void }>();

const EMPTY_STAT: DayStat = { total: 0, done: 0 };

/** 五档色阶 class（与图例共用；索引 = IntensityLevel） */
const LEVEL_CLASSES = [
  "bg-success/5",
  "bg-success/20",
  "bg-success/45",
  "bg-success/70",
  "bg-success",
] as const;

/** 行标文案：仅在第一/三/五行打字（GitHub 式留白） */
const ROW_LABELS = ["", "一", "", "三", "", "五", ""] as const;

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

/**
 * 热力图列数据（滚动近一年）：起点 = 今天往前推 364 天所在周的周一，
 * 终点 = 今天；每周一列、每列 7 行（周一..周日），行内按日期对齐。
 */
const columns = computed<(HeatCell | null)[][]>(() => {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const start = new Date(today);
  start.setDate(start.getDate() - 364);
  // 对齐该周的周一（周一首行）
  start.setDate(start.getDate() - ((start.getDay() + 6) % 7));

  const totalDays =
    Math.floor((today.getTime() - start.getTime()) / 86400000) + 1;
  const nWeeks = Math.ceil(totalDays / 7);

  const list: (HeatCell | null)[][] = [];
  for (let w = 0; w < nWeeks; w++) {
    const col: (HeatCell | null)[] = [];
    for (let r = 0; r < 7; r++) {
      const d = new Date(start);
      d.setDate(d.getDate() + w * 7 + r);
      if (d.getTime() > today.getTime()) {
        col.push(null);
        continue;
      }
      const stat = props.stats.get(dayKey(d)) ?? EMPTY_STAT;
      col.push({
        date: d,
        level: intensityLevel(stat),
        total: stat.total,
        done: stat.done,
      });
    }
    list.push(col);
  }
  return list;
});

/**
 * 列顶月份标签：列内含某月 1 日 → 「X 月」（同列多读月时取首个 1 日）；
 * 否则空串。
 */
const monthLabels = computed<string[]>(() =>
  columns.value.map((col) => {
    for (const cell of col) {
      if (cell && cell.date.getDate() === 1) {
        return `${cell.date.getMonth() + 1} 月`;
      }
    }
    return "";
  }),
);

/** 「今天」的日子键（描边圈高亮 + aria 标注用，本地时区安全） */
const todayKey = dayKey(new Date());

// ---- 悬浮详情卡（Teleport 到 body，滚动时收起）----

/** 悬浮卡状态：锚格 + 以 fixed 视口坐标定位（不受横向滚动容器剪裁） */
const hovered = ref<{
  date: Date;
  events: CalendarEvent[];
  x: number;
  y: number;
} | null>(null);

const CARD_WIDTH = 256;

/** 悬停/聚焦格子：记录锚点并把浮卡定位在格上方（左右出界向内收敛） */
function showCard(cell: HeatCell, e: MouseEvent | FocusEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const x = Math.min(
    Math.max(rect.left + rect.width / 2, CARD_WIDTH / 2 + 8),
    window.innerWidth - CARD_WIDTH / 2 - 8,
  );
  hovered.value = {
    date: cell.date,
    events: props.events?.get(dayKey(cell.date)) ?? [],
    x,
    y: rect.top - 10,
  };
}

function hideCard() {
  hovered.value = null;
}

/** 每格 aria-label / title：「10 月 14 日，3 章中已完成 2」式 */
function cellLabel(cell: HeatCell): string {
  const base = `${cell.date.getMonth() + 1} 月 ${cell.date.getDate()} 日`;
  if (cell.total === 0) return `${base}，无学习安排`;
  return `${base}，${cell.total} 章中已完成 ${cell.done}`;
}
</script>

<template>
  <div>
    <!-- 横向滚动容器（近一年 53 列在窄视口下可平移） -->
    <div class="overflow-x-auto pb-1" @scroll.capture="hideCard">
      <div class="min-w-max">
        <!-- 列顶月份标签（与列位对齐） -->
        <div class="mb-1 flex items-end gap-[3px]" aria-hidden="true">
          <span class="w-7 shrink-0" />
          <span
            v-for="(label, i) in monthLabels"
            :key="i"
            class="w-3.5 text-left text-[10px] leading-none tabular-nums text-muted-foreground"
          >
            {{ label }}
          </span>
        </div>

        <!-- 左侧星期标签 + 7 行（周一..周日）× 周列 -->
        <div class="flex gap-[3px]" role="group" aria-label="近一年学习热力图">
          <!-- 行标竖列：只在第一/三/五行打字（GitHub 式留白） -->
          <div class="flex shrink-0 flex-col gap-[3px]" aria-hidden="true">
            <template v-for="(_, r) in 7" :key="r">
              <span
                class="h-3.5 w-7 text-right text-[10px] leading-none text-muted-foreground"
              >
                {{ ROW_LABELS[r] }}
              </span>
            </template>
          </div>
          <div
            v-for="(col, ci) in columns"
            :key="ci"
            class="flex flex-col gap-[3px]"
          >
            <template v-for="(cell, ri) in col" :key="ri">
              <!-- 窗口外/未来日期：占位保持列对齐 -->
              <span
                v-if="cell === null"
                class="h-3.5 w-3.5"
                aria-hidden="true"
              />
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
                @mouseenter="showCard(cell, $event)"
                @focus="showCard(cell, $event)"
                @mouseleave="hideCard"
                @blur="hideCard"
                :title="cellLabel(cell)"
                :aria-label="`${cellLabel(cell)}${dayKey(cell.date) === todayKey ? '（今日）' : ''}，点击查看当天安排`"
                @click="emit('select-day', cell.date)"
              />
            </template>
          </div>
        </div>
      </div>
    </div>

    <!-- 悬浮详情卡：日期 + 当日各小任务（点任务条 = 打开当天抽屉） -->
    <Teleport to="body">
      <div
        v-if="hovered"
        class="pointer-events-auto fixed z-50 w-64 max-w-[16rem] rounded-lg border border-border bg-popover p-3 shadow-md"
        :style="{
          left: `${hovered.x}px`,
          top: `${hovered.y}px`,
          transform: 'translate(-50%, -100%)',
        }"
        role="tooltip"
        @mouseleave="hideCard"
      >
        <p class="font-display text-sm font-semibold tabular-nums">
          {{ hovered.date.getMonth() + 1 }} 月 {{ hovered.date.getDate() }} 日
        </p>
        <p
          v-if="hovered.events.length === 0"
          class="mt-1 text-xs text-muted-foreground"
        >
          该日无学习安排
        </p>
        <ul v-else class="mt-2 space-y-1.5">
          <li v-for="(ev, i) in hovered.events" :key="i">
            <button
              type="button"
              class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-md px-1.5 py-1 text-left text-xs transition-colors duration-200 hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :aria-label="`打开 ${hovered.date.getMonth() + 1} 月 ${hovered.date.getDate()} 日 ${ev.planTitle} 的 ${ev.chapterName} 详情`"
              @click="
                emit('select-day', hovered.date);
                hideCard();
              "
            >
              <span class="min-w-0 truncate">
                <span
                  :class="ev.done ? 'text-success' : 'text-primary'"
                  class="mr-1"
                  aria-hidden="true"
                >
                  {{ ev.done ? "✓" : "◌" }}
                </span>
                {{ ev.chapterName }}
              </span>
              <span class="shrink-0 text-[10px] text-muted-foreground">
                {{ ev.planTitle }}
              </span>
            </button>
          </li>
        </ul>
        <p
          class="mt-2 border-t border-border/60 pt-1.5 text-[10px] text-muted-foreground"
        >
          点击格子查看当天完整安排
        </p>
      </div>
    </Teleport>

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
