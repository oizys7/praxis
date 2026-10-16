<script setup lang="ts">
/**
 * CalendarView：学习进度页（Task E 真数据版）。
 * 页面重心 =「计划学习进度」，一屏上下两段：
 *
 * 1. 第一屏「进行中的学习计划」：计划卡列表——list_plans + get_plan 拉取，
 *    完成度 = 章节 status=complete 计数 / 总章数；阶段徽章由 phaseBadgeStatus 推导
 * 2. 第二屏「学习日历」：工具条（年 | 月 chip 切换 + 翻页 + 今天）+
 *    双视图——月 = 6×7 日历（MonthGrid）/ 年 = 12 行热力图（YearHeatmap），
 *    强度 = computeDayAssignments 按日聚合（章完成比例五档色阶）
 * 3. 月历格内直接排布章事件块（微型日程表），点击任一天 → DayDrawer 右滑
 *    抽屉（完整当安排 + 「当日已学」打卡 + 内嵌「编辑学习日」chips）
 *
 * 分配零存储：useStudyAllocation 纯函数即时重算（marks / studyDays 编辑
 * 仅更新本地视图态，分配结果随 computed 联动）；
 * 风格对齐「自习室纸面」：零 dark: 变体，色阶全走语义令牌。
 */
import { computed, onMounted, ref, shallowRef } from "vue";
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
  fetchPlanRecord,
  fetchPlanSummaries,
  savePlanStudyDays,
  saveStudyDayMark,
} from "@/composables/usePlanHistory";
import {
  aggregateDayStats,
  computeDayAssignments,
  dayKey,
  type DayStat,
} from "@/composables/useStudyAllocation";
import type { PlanRecord, PlanSummary } from "@/types/agent";
import PlanProgressPanel from "@/components/calendar/PlanProgressPanel.vue";
import LoadingState from "@/components/LoadingState.vue";

const emit = defineEmits<{
  /** 抽屉内点计划名 → 跳转学习计划详情（App 层协调跨页导航） */
  (e: "open-plan", planId: string): void;
}>();

// ---- 数据装载（list_plans + 逐计划 get_plan）----

const records = shallowRef<PlanRecord[]>([]);
const loading = ref(true);
const loadError = ref<CommandError | null>(null);

/** 打卡 / 学习日编辑请求在飞（抽屉控件禁用与防抖） */
const saving = ref(false);

/** 本地视图态：planId → 已打卡日集合（初始取自 dayMarks，打卡后本地同步） */
const marksByPlan = ref<Map<string, Set<string>>>(new Map());
/** 本地视图态：planId → 每周学习日（编辑保存后本地覆盖，分配即时重算） */
const studyDaysByPlan = ref<Map<string, number[]>>(new Map());

async function load() {
  loading.value = true;
  loadError.value = null;
  try {
    const summaries: PlanSummary[] = await fetchPlanSummaries();
    // 详情并发拉取（单用户桌面场景，列表上限内直拉全量）
    const details = await Promise.all(
      summaries.map((s) => fetchPlanRecord(s.id)),
    );
    records.value = details;
    const marks = new Map<string, Set<string>>();
    const days = new Map<string, number[]>();
    for (const record of details) {
      marks.set(record.id, new Set(record.dayMarks));
      days.set(record.id, record.studyDays);
    }
    marksByPlan.value = marks;
    studyDaysByPlan.value = days;
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

// ---- 第一屏：进行中的学习计划 ----

/** 页头统计：全部计划章完成率（零章防御） */
const overallStats = computed(() => {
  let total = 0;
  let done = 0;
  for (const record of records.value) {
    for (const chapter of record.plan.phases.flatMap((p) => p.chapters ?? [])) {
      total += 1;
      if (chapter.status === "complete") done += 1;
    }
  }
  return { total, done };
});

const overallPercent = computed(() =>
  overallStats.value.total === 0
    ? 0
    : Math.round((overallStats.value.done / overallStats.value.total) * 100),
);

/** 本月打卡天数（跨计划按「该日任一计划已打卡」去重，dayKey 前缀匹配） */
const monthMarkDays = computed(() => {
  const now = new Date();
  const prefix = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-`;
  const days = new Set<string>();
  for (const marks of marksByPlan.value.values()) {
    for (const key of marks) {
      if (key.startsWith(prefix)) days.add(key);
    }
  }
  return days.size;
});

// ---- 第二屏：学习日历 ----

/** 视图模式：月 = 日历网格（默认）/ 年 = 热力图 */
const viewMode = ref<"month" | "year">("month");

/** 当前查看的年月（初始锚今天） */
const viewDate = ref(new Date());

/** 日钻取抽屉状态 */
const selectedDate = ref<Date | null>(null);
const drawerOpen = ref(false);
const drawerRef = ref<InstanceType<typeof DayDrawer> | null>(null);

/**
 * 按日统计（真计划分配聚合）：每计划以视图态（本地 studyDays 覆盖 +
 * 本地 marks）重算分配，再跨计划按日累加。
 */
const dayStats = computed<Map<string, DayStat>>(() => {
  const merged = new Map<string, DayStat>();
  for (const record of records.value) {
    const view = {
      ...record,
      studyDays: studyDaysByPlan.value.get(record.id) ?? record.studyDays,
    };
    const assignments = computeDayAssignments(
      view,
      marksByPlan.value.get(record.id) ?? new Set(),
    );
    for (const [date, stat] of aggregateDayStats(assignments)) {
      const cur = merged.get(date) ?? { total: 0, done: 0 };
      cur.total += stat.total;
      cur.done += stat.done;
      merged.set(date, cur);
    }
  }
  return merged;
});

/**
 * 当日分组（通用）：给定日期当天各计划的分配清单（无安排且未打卡的
 * 计划不出现，避免空组噪音）。抽屉 drawerGroups 复用。
 */
function dayGroups(date: Date): DrawerPlanGroup[] {
  const key = dayKey(date);
  const groups: DrawerPlanGroup[] = [];
  for (const record of records.value) {
    const marks = marksByPlan.value.get(record.id) ?? new Set();
    const view = {
      ...record,
      studyDays: studyDaysByPlan.value.get(record.id) ?? record.studyDays,
    };
    const dayPlans = computeDayAssignments(view, marks).filter(
      (a) => a.date === key,
    );
    if (dayPlans.length === 0 && !marks.has(key)) continue;
    groups.push({
      planId: record.id,
      planTitle: record.title,
      marked: marks.has(key),
      studyDays: view.studyDays,
      assignments: dayPlans.map((a) => ({
        phaseIndex: a.phaseIndex,
        phaseName: a.phaseName,
        chapterIndex: a.chapterIndex,
        chapterName: a.chapterTitle,
        done: a.done,
      })),
    });
  }
  return groups;
}

/** 抽屉：选中日期的分组 */
const drawerGroups = computed<DrawerPlanGroup[]>(() =>
  selectedDate.value ? dayGroups(selectedDate.value) : [],
);

/** 月历事件块：dayKey → 章日程（跨计划聚合，明细供格内渲染） */
const calendarEvents = computed<Map<string, CalendarEvent[]>>(() => {
  const merged = new Map<string, CalendarEvent[]>();
  for (const record of records.value) {
    const marks = marksByPlan.value.get(record.id) ?? new Set();
    const view = {
      ...record,
      studyDays: studyDaysByPlan.value.get(record.id) ?? record.studyDays,
    };
    for (const a of computeDayAssignments(view, marks)) {
      const list = merged.get(a.date) ?? [];
      list.push({
        planTitle: record.title,
        chapterName: a.chapterTitle,
        done: a.done,
      });
      merged.set(a.date, list);
    }
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

/** 日钻取：打开任一天的抽屉 */
function openDay(date: Date) {
  selectedDate.value = date;
  drawerOpen.value = true;
}

// ---- IPC 回写：打卡 / 编辑学习日（落库后本地视图态同步，分配重算）----

/** 当日已学打卡开关（date 缺省 = 抽屉选中日；侧卡显式传今天） */
async function onToggleMark(planId: string, done: boolean, date?: Date) {
  const target = date ?? selectedDate.value;
  if (!target) return;
  const day = dayKey(target);
  saving.value = true;
  try {
    await saveStudyDayMark(planId, day, done);
    const set = new Set(marksByPlan.value.get(planId) ?? []);
    if (done) set.add(day);
    else set.delete(day);
    marksByPlan.value = new Map(marksByPlan.value).set(planId, set);
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    saving.value = false;
    drawerRef.value?.settleMark(planId);
  }
}

/** 学习日切换（进度面板 chip 点击）：乐观更新 + IPC，失败回滚；至少保留一天 */
async function onToggleStudyDay(planId: string, day: number) {
  if (saving.value) return;
  const current =
    studyDaysByPlan.value.get(planId) ??
    records.value.find((r) => r.id === planId)?.studyDays ??
    [];
  const next = current.includes(day)
    ? current.filter((d) => d !== day)
    : [...current, day].sort((a, b) => a - b);
  if (next.length === 0) return; // 至少保留一个学习日
  saving.value = true;
  studyDaysByPlan.value = new Map(studyDaysByPlan.value).set(planId, next);
  try {
    const normalized = await savePlanStudyDays(planId, next);
    studyDaysByPlan.value = new Map(studyDaysByPlan.value).set(
      planId,
      normalized,
    );
  } catch (e) {
    // 回滚到切换前，重排随之还原
    studyDaysByPlan.value = new Map(studyDaysByPlan.value).set(planId, current);
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
      整体完成 {{ overallPercent }}%，本月打卡 {{ monthMarkDays }} 天，
      {{ records.length }} 份计划进行中
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
      v-else-if="records.length === 0"
      class="rounded-lg border border-dashed border-border px-6 py-16 text-center"
    >
      <p class="text-sm text-muted-foreground">
        还没有学习计划——去「学习计划」页生成第一份计划，日历将自动排布每日章节。
      </p>
    </div>

    <template v-else>
      <!-- 双栏：左计划进度管理 / 右日历（控件行 + 主体）；窄屏日历在上 -->
      <div
        class="grid items-start gap-5 lg:grid-cols-[320px_minmax(0,1fr)] 2xl:grid-cols-[380px_minmax(0,1fr)]"
      >
        <!-- 左：计划进度管理（学习日编辑入口在此，点周几即改即存） -->
        <div class="order-2 lg:order-1">
          <PlanProgressPanel
            :records="records"
            :marks-by-plan="marksByPlan"
            :study-days-by-plan="studyDaysByPlan"
            :saving="saving"
            @open-plan="(planId) => emit('open-plan', planId)"
            @toggle-study-day="onToggleStudyDay"
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

    <!-- 日钻取抽屉 -->
    <DayDrawer
      ref="drawerRef"
      :open="drawerOpen && !loading"
      :date="selectedDate"
      :groups="drawerGroups"
      :saving="saving"
      @close="drawerOpen = false"
      @toggle-mark="onToggleMark"
      @open-plan="(planId) => emit('open-plan', planId)"
    />
  </div>
</template>
