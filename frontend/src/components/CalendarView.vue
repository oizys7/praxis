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
 * 3. 日钻取：点击任一天 → DayDrawer 右滑抽屉（按计划分组的当日分配 +
 *    「当日已学」打卡 + 内嵌「编辑学习日」chips，双写 IPC 落库后本地重算）
 *
 * 分配零存储：useStudyAllocation 纯函数即时重算（marks / studyDays 编辑
 * 仅更新本地视图态，分配结果随 computed 联动）；
 * 风格对齐「自习室纸面」：零 dark: 变体，色阶全走语义令牌。
 */
import { computed, onMounted, ref, shallowRef } from "vue";
import { ChevronLeft, ChevronRight, LoaderCircle, TriangleAlert } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import MonthGrid from "@/components/calendar/MonthGrid.vue";
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
  phaseBadgeStatus,
  planStartDate,
  type DayStat,
  type PhaseBadgeStatus,
} from "@/composables/useStudyAllocation";
import type { PlanRecord, PlanSummary } from "@/types/agent";

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

/** 水平档位中文（旧数据未知值原样透传） */
const LEVEL_LABELS: Record<string, string> = {
  beginner: "入门",
  intermediate: "进阶",
  advanced: "高级",
};

/** 进度卡视图模型 */
interface PlanProgressCard {
  id: string;
  title: string;
  level: string;
  totalChapters: number;
  completedChapters: number;
  /** 距截止剩余天数（计划起点 = createdAt 所在周周一，跨周按 7 天/周计） */
  daysLeft: number;
  phases: Array<{ name: string; status: PhaseBadgeStatus }>;
}

const planCards = computed<PlanProgressCard[]>(() =>
  records.value.map((record) => {
    const chapters = record.plan.phases.flatMap((p) => p.chapters ?? []);
    const completedChapters = chapters.filter(
      (c) => c.status === "complete",
    ).length;
    const start = planStartDate(record.createdAt).getTime();
    const deadline =
      start + Math.max(Math.round(record.weeks) || 1, 1) * 7 * 86400000;
    const now = new Date();
    const todayMid = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate(),
    ).getTime();
    return {
      id: record.id,
      title: record.title,
      level: LEVEL_LABELS[record.level] ?? record.level,
      totalChapters: chapters.length,
      completedChapters,
      daysLeft: Math.max(0, Math.ceil((deadline - todayMid) / 86400000)),
      phases: record.plan.phases.map((p) => ({
        name: p.name,
        status: phaseBadgeStatus(p),
      })),
    };
  }),
);

/** 完成百分比（取整；零章防御） */
function percent(p: { completedChapters: number; totalChapters: number }) {
  if (p.totalChapters === 0) return 0;
  return Math.round((p.completedChapters / p.totalChapters) * 100);
}

/** 剩余天数是否临近截止（<= 7 天转朱批提醒色） */
function isUrgent(daysLeft: number) {
  return daysLeft <= 7;
}

/** 阶段徽章样式（三态：已完成实绿 / 进行中墨蓝描边 / 未开始沙框灰） */
const PHASE_CLASSES: Record<PhaseBadgeStatus, string> = {
  done: "border-transparent bg-success/15 text-success",
  active: "border-primary/40 text-primary",
  todo: "border-border text-muted-foreground",
};

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
 * 抽屉分组：选中日期当天的各计划分配（无安排且未打卡的计划不出现，
 * 避免空组噪音）。
 */
const drawerGroups = computed<DrawerPlanGroup[]>(() => {
  if (!selectedDate.value) return [];
  const key = dayKey(selectedDate.value);
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
});

/** 工具条标题：「2026 年 10 月」/「2026 年」 */
const viewTitle = computed(() => {
  const y = viewDate.value.getFullYear();
  if (viewMode.value === "year") return `${y} 年`;
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

/** 当日已学打卡开关 */
async function onToggleMark(planId: string, done: boolean) {
  if (!selectedDate.value) return;
  const day = dayKey(selectedDate.value);
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

/** 保存每周学习日（Rust 归一化返回后本地覆盖，分配/统计联动重算） */
async function onSaveStudyDays(planId: string, days: number[]) {
  saving.value = true;
  try {
    const normalized = await savePlanStudyDays(planId, days);
    studyDaysByPlan.value = new Map(studyDaysByPlan.value).set(
      planId,
      normalized,
    );
  } catch (e) {
    loadError.value = normalizeError(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="mx-auto w-full max-w-5xl px-6 py-8">
    <!-- 页头 -->
    <header class="mb-6">
      <h2 class="font-display text-2xl font-semibold tracking-tight">
        学习进度
      </h2>
      <p class="mt-1 text-sm text-muted-foreground">
        计划推进总览 · 每日一勤，功不唐捐
      </p>
    </header>

    <!-- 加载 / 错误 / 空态 -->
    <div
      v-if="loading"
      class="flex items-center gap-2 py-16 text-sm text-muted-foreground"
      role="status"
    >
      <LoaderCircle class="h-4 w-4 animate-spin" aria-hidden="true" />
      正在加载学习进度…
    </div>
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
      v-else-if="planCards.length === 0"
      class="rounded-lg border border-dashed border-border px-6 py-16 text-center"
    >
      <p class="text-sm text-muted-foreground">
        还没有学习计划——去「学习计划」页生成第一份计划，日历将自动排布每日章节。
      </p>
    </div>

    <template v-else>
      <!-- 第一屏：进行中的学习计划 -->
      <section aria-labelledby="plans-heading" class="mb-8">
        <h3
          id="plans-heading"
          class="mb-4 font-display text-base font-semibold"
        >
          进行中的学习计划
        </h3>
        <div class="grid items-start gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <Card
            v-for="plan in planCards"
            :key="plan.id"
            class="transition-colors duration-200 hover:border-primary/40"
          >
            <CardHeader>
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <CardTitle class="font-display text-base leading-snug">
                    {{ plan.title }}
                  </CardTitle>
                  <CardDescription class="mt-1">
                    {{ plan.level }}
                    <span
                      class="ml-1 tabular-nums"
                      :class="isUrgent(plan.daysLeft) ? 'text-destructive' : ''"
                    >
                      · 剩余 {{ plan.daysLeft }} 天
                    </span>
                  </CardDescription>
                </div>
                <!-- 完成百分比大数字 -->
                <p
                  class="shrink-0 font-display text-3xl font-semibold tabular-nums text-primary"
                  :aria-label="`完成度 ${percent(plan)}%`"
                >
                  {{ percent(plan) }}<span class="text-sm font-normal">%</span>
                </p>
              </div>
            </CardHeader>
            <CardContent>
              <!-- 进度条 -->
              <div
                class="h-2 overflow-hidden rounded-full bg-primary/15"
                role="progressbar"
                :aria-valuenow="percent(plan)"
                aria-valuemin="0"
                aria-valuemax="100"
                :aria-label="`${plan.title} 完成进度`"
              >
                <div
                  class="h-full rounded-full bg-primary transition-[width] duration-500"
                  :style="{ width: `${percent(plan)}%` }"
                />
              </div>
              <p class="mt-2 text-xs tabular-nums text-muted-foreground">
                已完成 {{ plan.completedChapters }} /
                {{ plan.totalChapters }} 章
              </p>
              <!-- 阶段徽章序列 -->
              <div class="mt-3 flex flex-wrap gap-1.5">
                <span
                  v-for="(phase, i) in plan.phases"
                  :key="phase.name"
                  class="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium"
                  :class="PHASE_CLASSES[phase.status]"
                >
                  阶段 {{ i + 1 }} · {{ phase.name }}
                </span>
              </div>
            </CardContent>
          </Card>
        </div>
      </section>

      <!-- 第二屏：学习日历 -->
      <section aria-labelledby="calendar-heading">
        <h3 id="calendar-heading" class="sr-only">学习日历</h3>
        <Card>
          <CardHeader class="space-y-3">
            <div
              class="flex flex-wrap items-center justify-between gap-x-4 gap-y-3"
            >
              <!-- 视图 chip 切换（年 = 热力图 / 月 = 日历，默认月） -->
              <div
                class="flex items-center rounded-lg border border-border p-0.5"
                role="group"
                aria-label="日历视图切换"
              >
                <button
                  type="button"
                  class="cursor-pointer rounded-md px-3 py-1 text-sm transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :class="
                    viewMode === 'month'
                      ? 'bg-secondary font-medium text-secondary-foreground'
                      : 'text-muted-foreground hover:text-foreground'
                  "
                  :aria-pressed="viewMode === 'month'"
                  @click="viewMode = 'month'"
                >
                  月
                </button>
                <button
                  type="button"
                  class="cursor-pointer rounded-md px-3 py-1 text-sm transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :class="
                    viewMode === 'year'
                      ? 'bg-secondary font-medium text-secondary-foreground'
                      : 'text-muted-foreground hover:text-foreground'
                  "
                  :aria-pressed="viewMode === 'year'"
                  @click="viewMode = 'year'"
                >
                  年
                </button>
              </div>

              <!-- 翻页 + 今天 -->
              <div class="flex items-center gap-1">
                <button
                  type="button"
                  class="flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :aria-label="viewMode === 'month' ? '上一月' : '上一年'"
                  :title="viewMode === 'month' ? '上一月' : '上一年'"
                  @click="shiftView(-1)"
                >
                  <ChevronLeft class="h-4 w-4" aria-hidden="true" />
                </button>
                <p
                  class="min-w-[7.5rem] text-center font-display text-base font-semibold tabular-nums"
                  aria-live="polite"
                >
                  {{ viewTitle }}
                </p>
                <button
                  type="button"
                  class="flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :aria-label="viewMode === 'month' ? '下一月' : '下一年'"
                  :title="viewMode === 'month' ? '下一月' : '下一年'"
                  @click="shiftView(1)"
                >
                  <ChevronRight class="h-4 w-4" aria-hidden="true" />
                </button>
                <Button
                  variant="outline"
                  size="sm"
                  type="button"
                  class="ml-2"
                  @click="goToday"
                >
                  今天
                </Button>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <!-- 月视图：日历网格 -->
            <MonthGrid
              v-if="viewMode === 'month'"
              :year="viewDate.getFullYear()"
              :month="viewDate.getMonth()"
              :stats="dayStats"
              @select-day="openDay"
            />
            <!-- 年视图：热力图 -->
            <YearHeatmap
              v-else
              :year="viewDate.getFullYear()"
              :stats="dayStats"
              @select-day="openDay"
            />
          </CardContent>
        </Card>
      </section>
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
      @save-study-days="onSaveStudyDays"
    />
  </div>
</template>
