<script setup lang="ts">
/**
 * CalendarView：学习进度页（打卡日历真数据版）。
 * 页面重心 =「计划学习进度」，一屏上下两段：
 *
 * 1. 第一屏「进行中的学习计划」：计划卡列表——仅 list_plans（零 get_plan），
 *    完成度 = chapterDone/chapterTotal（summary 随行携带）；三分类 =
 *    learningStatus（not_started | active | completed，真字段）
 * 2. 第二屏「学习日历」：工具条（年 | 月 chip 切换 + 翻页 + 今天）+
 *    双视图——月 = 6×7 日历（MonthGrid）/ 年 = 12 行热力图（YearHeatmap），
 *    数据来自 query_month_calendar（按天组织）/ query_year_heatmap，
 *    强度 = 已学章节比例五档色阶
 * 3. 月历格内直接排布章事件块（微型日程表），点击任一天 → DayDrawer 右滑
 *    抽屉（完整当日安排 + 每章独立打卡勾选，仅今天及之前可勾）
 *
 * 学习日开始/修改：chips 在 PlanProgressPanel 仅本地暂存，提交（start-learning /
 * update-study-days）由本层调 IPC；提交后失效月/年缓存并重拉当前视图。
 */
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { ChevronLeft, ChevronRight, TriangleAlert } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import MonthGrid, {
  type CalendarEvent,
} from "@/components/calendar/MonthGrid.vue";
import YearHeatmap from "@/components/calendar/YearHeatmap.vue";
import DayDrawer, {
  type DrawerPlanGroup,
} from "@/components/calendar/DayDrawer.vue";
import { normalizeError } from "@/composables/useAgent";
import type { CommandError } from "@/types/agent";
import {
  queryMonthCalendar,
  queryYearHeatmap,
  startLearning,
  toggleScheduleItem,
  updateLearningDays,
  fetchPlanSummaries,
} from "@/composables/usePlanHistory";
import { dayKey, type DayStat } from "@/composables/useStudyAllocation";
import type {
  MonthCalendarDay,
  PlanSummary,
  ScheduleItem,
  YearHeatmapDay,
} from "@/types/agent";
import PlanProgressPanel from "@/components/calendar/PlanProgressPanel.vue";
import LoadingState from "@/components/LoadingState.vue";

const emit = defineEmits<{
  /** 抽屉内点计划名 → 跳转学习计划详情（App 层协调跨页导航） */
  (e: "open-plan", planId: string): void;
}>();

// ---- 数据装载（仅 list_plans + 月/年日程查询，零 get_plan）----

const summaries = shallowRef<PlanSummary[]>([]);
const loading = ref(true);
const loadError = ref<CommandError | null>(null);

/** 打卡 / 学习日提交请求在飞（抽屉与面板控件禁用与防抖） */
const saving = ref(false);

async function load() {
  loading.value = true;
  loadError.value = null;
  try {
    summaries.value = await fetchPlanSummaries();
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

// ---- 第二屏：学习日历（月/年真数据） ----

/** 视图模式：月 = 日历网格（默认）/ 年 = 热力图 */
const viewMode = ref<"month" | "year">("month");

/** 当前查看的年月（初始锚今天） */
const viewDate = ref(new Date());

/** 月数据缓存：'YYYY-MM' → 按天组织的当月日程（提交打卡/学习日后失效） */
const monthCache = ref<Map<string, MonthCalendarDay[]>>(new Map());
/** 年数据缓存：年份字符串 → 按天强度 */
const yearCache = ref<Map<string, YearHeatmapDay[]>>(new Map());
/** 日历查询在飞（与 saving 分开：打卡禁用控件，查询禁用翻页不必要） */
const calendarLoading = ref(false);

function monthKeyOf(year: number, month: number): string {
  return `${year}-${String(month + 1).padStart(2, "0")}`;
}

/** 当前视图月份键 */
const currentMonthKey = computed(() =>
  monthKeyOf(viewDate.value.getFullYear(), viewDate.value.getMonth()),
);

/** 当月按天数据（月视图 / 抽屉共用；无数据天不出现在数组中） */
const monthDays = computed<MonthCalendarDay[]>(
  () => monthCache.value.get(currentMonthKey.value) ?? [],
);

/** 当前年数据（年视图热力图） */
const yearDays = computed<YearHeatmapDay[]>(
  () => yearCache.value.get(String(viewDate.value.getFullYear())) ?? [],
);

/** 确保当前视图的月数据已拉取（缓存未命中才请求） */
async function ensureMonthData() {
  const key = currentMonthKey.value;
  if (monthCache.value.has(key)) return;
  const [year, month] = key.split("-").map(Number);
  calendarLoading.value = true;
  try {
    const days = await queryMonthCalendar(year, month);
    monthCache.value = new Map(monthCache.value).set(key, days);
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    calendarLoading.value = false;
  }
}

/** 确保当前视图的年数据已拉取（缓存未命中才请求） */
async function ensureYearData() {
  const year = String(viewDate.value.getFullYear());
  if (yearCache.value.has(year)) return;
  calendarLoading.value = true;
  try {
    const days = await queryYearHeatmap(Number(year));
    yearCache.value = new Map(yearCache.value).set(year, days);
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    calendarLoading.value = false;
  }
}

/** 视图切换 / 翻页 / 首次进入：按需拉取对应月或年的数据（
 *  不受列表 loading 影响：日历数据独立于 summaries，守卫会吞掉 immediate 首跑） */
watch(
  [viewMode, viewDate],
  () => {
    if (viewMode.value === "month") void ensureMonthData();
    else void ensureYearData();
  },
  { immediate: true },
);

/** 任何日程变更（打卡/开始学习/改学习日）后失效全部月/年缓存并重拉当前视图 */
function invalidateCalendar() {
  monthCache.value = new Map();
  yearCache.value = new Map();
  if (viewMode.value === "month") void ensureMonthData();
  else void ensureYearData();
}

// ---- 第一屏：进度统计（summary 自足，零 get_plan）----

/** 页头统计：全部计划打卡进度（study_schedule 口径，冗余计数直读） */
const overallStats = computed(() => {
  let total = 0;
  let done = 0;
  for (const summary of summaries.value) {
    total += summary.studyTotal;
    done += summary.studyDone;
  }
  return { total, done };
});

const overallPercent = computed(() =>
  overallStats.value.total === 0
    ? 0
    : Math.round((overallStats.value.done / overallStats.value.total) * 100),
);

/** 学习中计划数（learningStatus = active 真字段计数） */
const activeCount = computed(
  () => summaries.value.filter((s) => s.learningStatus === "active").length,
);

/** 本月已学天数（当月数据中 doneCount > 0 的天） */
const monthMarkedDays = computed(
  () => monthDays.value.filter((d) => d.doneCount > 0).length,
);

// ---- 日历主体数据构建（保持 MonthGrid/YearHeatmap 消费形状不变）----

/**
 * 按日统计：月视图用当月按天数据；年视图用年强度数据。
 * done 口径 = 行级打卡（study_schedule.done），与章节生成状态解耦。
 */
const dayStats = computed<Map<string, DayStat>>(() => {
  const merged = new Map<string, DayStat>();
  const source: Array<{ date: string; total: number; done: number }> =
    viewMode.value === "month"
      ? monthDays.value.map((d) => ({
          date: d.date,
          total: d.total,
          done: d.doneCount,
        }))
      : yearDays.value;
  for (const d of source) {
    merged.set(d.date, { total: d.total, done: d.done });
  }
  return merged;
});

/** 月历事件块：dayKey → 章事件（跨计划；年视图无明细传空，悬浮卡退化统计） */
const calendarEvents = computed<Map<string, CalendarEvent[]>>(() => {
  const merged = new Map<string, CalendarEvent[]>();
  if (viewMode.value !== "month") return merged;
  for (const day of monthDays.value) {
    merged.set(
      day.date,
      day.items.map((item) => ({
        planTitle: item.planTitle,
        chapterName: item.chapterTitle,
        done: item.done,
      })),
    );
  }
  return merged;
});

/** 工具条标题：「2026 年 10 月」/「2026 年」 */
const viewTitle = computed(() => {
  const y = viewDate.value.getFullYear();
  if (viewMode.value === "year") return "近一年";
  return `${y} 年 ${viewDate.value.getMonth() + 1} 月`;
});

/** 翻页（月视图 ±1 月，年视图 ±1 年；本地时区安全用 Date 构造平移） */
function shiftView(delta: number) {
  const d = new Date(viewDate.value);
  if (viewMode.value === "month") d.setMonth(d.getMonth() + delta);
  else d.setFullYear(d.getFullYear() + delta);
  viewDate.value = d;
}

/** 回到今天 */
function goToday() {
  viewDate.value = new Date();
}

// ---- 日钻取抽屉 ----

const selectedDate = ref<Date | null>(null);
const drawerOpen = ref(false);
const drawerRef = ref<InstanceType<typeof DayDrawer> | null>(null);

/** 该抽屉日是否允许打卡（今天及之前；'YYYY-MM-DD' 字典序即时间序） */
const drawerMarkable = computed(() => {
  if (!selectedDate.value) return false;
  return dayKey(selectedDate.value) <= dayKey(new Date());
});

/** 抽屉分组：当月数据中该天的条目按计划分组 */
const drawerGroups = computed<DrawerPlanGroup[]>(() => {
  if (!selectedDate.value) return [];
  const key = dayKey(selectedDate.value);
  const day = monthDays.value.find((d) => d.date === key);
  if (!day || day.items.length === 0) return [];
  const groups = new Map<string, DrawerPlanGroup>();
  for (const item of day.items) {
    const group = groups.get(item.planId) ?? {
      planId: item.planId,
      planTitle: item.planTitle,
      items: [] as ScheduleItem[],
    };
    group.items.push(item);
    groups.set(item.planId, group);
  }
  return [...groups.values()];
});

/** 日钻取：打开任一天的抽屉 */
function openDay(date: Date) {
  selectedDate.value = date;
  drawerOpen.value = true;
}

// ---- IPC 回写：行级打卡 / 开始学习 / 修改学习日 ----

/** 行级打卡切换（每章独立勾选；仅今天及之前，Rust 侧同样校验） */
async function onToggleItem(planId: string, item: ScheduleItem, done: boolean) {
  const target = selectedDate.value;
  if (!target || saving.value) return;
  saving.value = true;
  try {
    await toggleScheduleItem(
      planId,
      dayKey(target),
      item.phaseIdx,
      item.chapterIdx,
      done,
    );
    invalidateCalendar();
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    saving.value = false;
    drawerRef.value?.settleToggle(planId, item);
  }
}

/** 开始学习（未开始行【从今天开始】）：提交学习日，落定日程且不打卡今天 */
async function onStartLearning(planId: string, days: number[]) {
  if (saving.value) return;
  saving.value = true;
  try {
    await startLearning(planId, days);
    summaries.value = summaries.value.map((s) =>
      s.id === planId
        ? {
            ...s,
            learningStatus: "active",
            learningStartedAt: dayKey(new Date()),
            studyDays: days,
          }
        : s,
    );
    invalidateCalendar();
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    saving.value = false;
  }
}

/** 修改学习日（学习中行「保存学习日」）：保留今天之前与今天已完成，未来重排 */
async function onUpdateStudyDays(planId: string, days: number[]) {
  if (saving.value) return;
  saving.value = true;
  try {
    await updateLearningDays(planId, days);
    summaries.value = summaries.value.map((s) =>
      s.id === planId ? { ...s, studyDays: days } : s,
    );
    invalidateCalendar();
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="mx-auto w-full max-w-7xl px-6 py-8 2xl:max-w-[1600px]">
    <!-- 页头：衬线大标题 + 摘行靠左；视图切换与翻页下沉到日历右栏上方 -->
    <div class="mb-2">
      <h2
        class="font-display text-3xl font-semibold tracking-tight tabular-nums"
        aria-live="polite"
      >
        {{ viewTitle }}
      </h2>
    </div>

    <!-- 摘行（真数据；中文逗号分隔） -->
    <p class="mb-6 text-sm text-muted-foreground">
      整体完成 {{ overallPercent }}%，本月已学 {{ monthMarkedDays }} 天，
      {{ activeCount }} 份计划学习中
    </p>

    <!-- 加载 / 错误 / 空态 -->
    <LoadingState v-if="loading" text="正在加载学习进度…" />
    <div
      v-else-if="loadError"
      class="flex flex-col items-start gap-3 rounded-lg border border-destructive/40 bg-destructive/5 px-5 py-4"
      role="alert"
    >
      <p class="flex items-center gap-2 text-sm text-destructive">
        <TriangleAlert class="h-4 w-4" aria-hidden="true" />
        {{ loadError.message || "学习进度加载失败" }}
      </p>
      <Button variant="outline" size="sm" type="button" @click="load">
        重试
      </Button>
    </div>
    <div
      v-else-if="summaries.length === 0"
      class="rounded-lg border border-dashed border-border px-6 py-16 text-center"
    >
      <p class="text-sm text-muted-foreground">
        还没有学习计划——去「学习计划」页生成第一份计划，点「开始学习」后排定每日日程。
      </p>
    </div>

    <template v-else>
      <!-- 双栏：左计划进度管理 / 右日历（控件行 + 主体）；窄屏日历在上 -->
      <div
        class="grid items-start gap-5 lg:grid-cols-[320px_minmax(0,1fr)] 2xl:grid-cols-[380px_minmax(0,1fr)]"
      >
        <!-- 左：计划进度管理（三分类 + 学习日本地暂存编辑入口） -->
        <div class="order-2 lg:order-1">
          <PlanProgressPanel
            :summaries="summaries"
            :saving="saving"
            @open-plan="(planId) => emit('open-plan', planId)"
            @start-learning="onStartLearning"
            @update-study-days="onUpdateStudyDays"
          />
        </div>

        <!-- 右：视图控件行 + 日历主体 -->
        <div class="order-1 space-y-3 lg:order-2">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div
              class="flex items-center rounded-full bg-muted/40 p-1"
              role="group"
              aria-label="日历视图切换"
            >
              <button
                type="button"
                class="cursor-pointer rounded-full px-3.5 py-1 text-sm transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :class="
                  viewMode === 'month'
                    ? 'bg-card font-medium text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground'
                "
                :aria-pressed="viewMode === 'month'"
                @click="viewMode = 'month'"
              >
                月
              </button>
              <button
                type="button"
                class="cursor-pointer rounded-full px-3.5 py-1 text-sm transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :class="
                  viewMode === 'year'
                    ? 'bg-card font-medium text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground'
                "
                :aria-pressed="viewMode === 'year'"
                @click="viewMode = 'year'"
              >
                年
              </button>
            </div>
            <div v-if="viewMode === 'month'" class="flex items-center gap-1.5">
              <button
                type="button"
                class="flex h-9 w-9 cursor-pointer items-center justify-center rounded-full border border-border text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label="上一月"
                title="上一月"
                @click="shiftView(-1)"
              >
                <ChevronLeft class="h-4 w-4" aria-hidden="true" />
              </button>
              <button
                type="button"
                class="flex h-9 w-9 cursor-pointer items-center justify-center rounded-full border border-border text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label="下一月"
                title="下一月"
                @click="shiftView(1)"
              >
                <ChevronRight class="h-4 w-4" aria-hidden="true" />
              </button>
              <Button
                variant="ghost"
                size="sm"
                type="button"
                class="ml-1 cursor-pointer"
                @click="goToday"
              >
                回到今天
              </Button>
            </div>
          </div>
          <Card>
            <CardContent class="p-3 sm:p-4">
              <!-- 月视图：日历网格 -->
              <MonthGrid
                v-if="viewMode === 'month'"
                :year="viewDate.getFullYear()"
                :month="viewDate.getMonth()"
                :stats="dayStats"
                :events="calendarEvents"
                @select-day="openDay"
              />
              <!-- 年视图：热力图 -->
              <YearHeatmap v-else :stats="dayStats" @select-day="openDay" />
            </CardContent>
          </Card>
        </div>
      </div>
    </template>

    <!-- 日钻取抽屉（行级打卡；未来日期只读展示） -->
    <DayDrawer
      ref="drawerRef"
      :open="drawerOpen && !loading"
      :date="selectedDate"
      :groups="drawerGroups"
      :saving="saving"
      :markable="drawerMarkable"
      @close="drawerOpen = false"
      @toggle-item="onToggleItem"
      @open-plan="(planId) => emit('open-plan', planId)"
    />
  </div>
</template>
