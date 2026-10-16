<script setup lang="ts">
/**
 * CalendarView：学习进度页（重构版，视觉壳 mock 先行）。
 * 页面重心 =「计划学习进度」，一屏上下两段：
 *
 * 1. 第一屏「进行中的学习计划」：计划卡列表——标题衬线、完成百分比大数字、
 *    总章/完成章徽（tabular-nums）、进度条（bg-primary/15 内 bar bg-primary）、
 *    阶段徽章序列、剩余天数（临近截止朱批色）
 * 2. 第二屏「学习日历」：工具条（年 | 月 chip 切换 + 翻页 + 今天）+
 *    双视图——月 = 6×7 日历（MonthGrid）/ 年 = 12 行热力图（YearHeatmap）
 * 3. 日钻取：点击任一天 → DayDrawer 右滑抽屉（当天分配 + 完成情况）
 *
 * 数据全部来自 src/calendarMocks.ts（Task E 将替换为 get_plan/list_marks 真数据）；
 * 风格对齐「自习室纸面」：零 dark: 变体，色阶全走语义令牌。
 */
import { computed, ref } from "vue";
import { ChevronLeft, ChevronRight } from "lucide-vue-next";
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
import DayDrawer from "@/components/calendar/DayDrawer.vue";
import { MOCK_PLANS, type PhaseStatus } from "@/calendarMocks";

// ---- 第一屏：进行中的学习计划 ----

/** 完成百分比（取整） */
function percent(p: { completedChapters: number; totalChapters: number }) {
  return Math.round((p.completedChapters / p.totalChapters) * 100);
}

/** 剩余天数是否临近截止（<= 7 天转朱批提醒色） */
function isUrgent(daysLeft: number) {
  return daysLeft <= 7;
}

/** 阶段徽章样式（三态：已完成实绿 / 进行中墨蓝描边 / 未开始沙框灰） */
const PHASE_CLASSES: Record<PhaseStatus, string> = {
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

    <!-- 第一屏：进行中的学习计划 -->
    <section aria-labelledby="plans-heading" class="mb-8">
      <h3 id="plans-heading" class="mb-4 font-display text-base font-semibold">
        进行中的学习计划
      </h3>
      <div class="grid items-start gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <Card
          v-for="plan in MOCK_PLANS"
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
              已完成 {{ plan.completedChapters }} / {{ plan.totalChapters }} 章
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
            @select-day="openDay"
          />
          <!-- 年视图：热力图 -->
          <YearHeatmap
            v-else
            :year="viewDate.getFullYear()"
            @select-day="openDay"
          />
        </CardContent>
      </Card>
    </section>

    <!-- 日钻取抽屉 -->
    <DayDrawer
      :open="drawerOpen"
      :date="selectedDate"
      @close="drawerOpen = false"
    />
  </div>
</template>
