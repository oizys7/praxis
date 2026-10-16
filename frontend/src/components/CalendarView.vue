<script setup lang="ts">
/**
 * CalendarView：打卡日历页（视觉版式先行，功能不实现）。
 * - 月视图 6 行 × 7 列网格（周一首）：日期数字 + 打卡点态
 *   （已打卡 = 书绿实心点 / 未打卡 = 沙框空心 / 今日 = 墨蓝描边圈）
 * - 顶部统计条（连续打卡 / 本月完成率，tabular-nums）+ 右侧「当日计划分配」侧卡
 * - 数据全部 mock 硬编码（以真实「今天」为锚点确定性生成，任何一天打开视觉都成立），
 *   无 IPC、无业务状态——翻月箭头仅视觉，不做翻页
 * - 风格对齐「自习室纸面」：serif 标题 font-display、沙框 border、语义色 badge，
 *   零 dark: 变体全走语义变量
 */

// ---- mock 数据（模块级一次性求值，非响应式） ----

const today = new Date();
const year = today.getFullYear();
const month = today.getMonth(); // 0 起始
const dayOfMonth = today.getDate();

/** 连续打卡天数（mock 常量，与打卡集合的连续段一致） */
const STREAK_DAYS = 12;

/** 已打卡日集合：今天往前连续 STREAK 天，加上更早的零散打卡日（不与连续段相接） */
const checkedDays = (() => {
  const set = new Set<number>();
  const streakStart = Math.max(1, dayOfMonth - STREAK_DAYS + 1);
  for (let d = streakStart; d <= dayOfMonth; d++) set.add(d);
  for (const d of [1, 3, 5, 7, 9]) {
    if (d < streakStart) set.add(d);
  }
  return set;
})();

/** 本月完成率：已打卡天数 / 本月已过天数（含今天），百分比取整 */
const monthRate = Math.round((checkedDays.size / dayOfMonth) * 100);

/** 月份标题「YYYY 年 M 月」 */
const monthTitle = `${year} 年 ${month + 1} 月`;

/** 当月天数 */
const daysInMonth = new Date(year, month + 1, 0).getDate();

/** 当月 1 号是周几（周一首：getDay() 周日 = 0，+6 取模平移） */
const firstWeekday = (new Date(year, month, 1).getDay() + 6) % 7;

/** 星期表头（周一起始） */
const WEEK_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

/** 单个日历格的渲染数据 */
interface CalendarCell {
  /** 日期数字（1 起始）；非本月补位格为 null */
  day: number | null;
  /** 打卡状态：已打卡 / 未打卡 / 未来日 */
  state: "checked" | "unchecked" | "future";
  /** 是否今日（墨蓝描边圈） */
  isToday: boolean;
}

/** 固定 6 行 × 7 列 = 42 格，当月不足前后以 null 补位格填满 */
const cells: CalendarCell[] = (() => {
  const list: CalendarCell[] = [];
  for (let i = 0; i < firstWeekday; i++) {
    list.push({ day: null, state: "future", isToday: false });
  }
  for (let d = 1; d <= daysInMonth; d++) {
    list.push({
      day: d,
      state:
        d > dayOfMonth
          ? "future"
          : checkedDays.has(d)
            ? "checked"
            : "unchecked",
      isToday: d === dayOfMonth,
    });
  }
  while (list.length < 42) {
    list.push({ day: null, state: "future", isToday: false });
  }
  return list;
})();

/** 「今日计划分配」侧卡条目（mock 硬编码） */
const todaySchedule = [
  {
    phase: "阶段一 · 基础巩固",
    chapter: "第 2 章 · 函数与作用域",
    time: "09:00 – 10:30",
    done: true,
  },
  {
    phase: "阶段二 · 框架实战",
    chapter: "第 5 节 · 组件通信与状态",
    time: "14:00 – 15:30",
    done: false,
  },
  {
    phase: "阶段三 · 综合项目",
    chapter: "项目周 · 联调与验收",
    time: "19:30 – 21:00",
    done: false,
  },
];

/** 日历格 aria-label：日期 + 打卡状态（读屏可感知当日打卡进度） */
function cellAria(cell: CalendarCell): string {
  const status = cell.isToday
    ? "今日"
    : cell.state === "checked"
      ? "已打卡"
      : cell.state === "future"
        ? "未到"
        : "未打卡";
  return `${month + 1} 月 ${cell.day} 日，${status}`;
}
</script>

<template>
  <div class="mx-auto w-full max-w-5xl px-6 py-8">
    <!-- 页头：标题 + 统计条 -->
    <header
      class="mb-6 flex flex-wrap items-end justify-between gap-x-6 gap-y-4"
    >
      <div>
        <h2 class="font-display text-2xl font-semibold tracking-tight">
          打卡日历
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">每日一勤，功不唐捐</p>
      </div>
      <dl class="flex items-center gap-6">
        <div class="text-right">
          <dt class="text-xs text-muted-foreground">连续打卡</dt>
          <dd
            class="font-display text-xl font-semibold tabular-nums text-success"
          >
            {{ STREAK_DAYS }}<span class="ml-0.5 text-xs font-normal">天</span>
          </dd>
        </div>
        <div class="h-9 w-px bg-border" aria-hidden="true" />
        <div class="text-right">
          <dt class="text-xs text-muted-foreground">本月完成率</dt>
          <dd
            class="font-display text-xl font-semibold tabular-nums text-primary"
          >
            {{ monthRate }}<span class="ml-0.5 text-xs font-normal">%</span>
          </dd>
        </div>
      </dl>
    </header>

    <div class="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
      <!-- 月历卡 -->
      <Card>
        <CardHeader class="flex-row items-center justify-between space-y-0">
          <CardTitle class="font-display text-base">{{ monthTitle }}</CardTitle>
          <!-- 翻月箭头：仅视觉，不接状态 -->
          <div class="flex items-center gap-1">
            <button
              type="button"
              aria-label="上一月"
              title="上一月"
              class="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
            >
              <ChevronLeft class="h-4 w-4" aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="下一月"
              title="下一月"
              class="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
            >
              <ChevronRight class="h-4 w-4" aria-hidden="true" />
            </button>
          </div>
        </CardHeader>
        <CardContent>
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
          <div class="grid grid-cols-7 gap-1.5">
            <template v-for="(cell, i) in cells" :key="i">
              <!-- 非本月补位格：纯占位 -->
              <div
                v-if="cell.day === null"
                class="aspect-square rounded-lg"
                aria-hidden="true"
              />
              <div
                v-else
                class="flex aspect-square cursor-default flex-col items-center justify-center gap-1.5 rounded-lg text-sm transition-colors hover:bg-accent/50"
                :class="
                  cell.isToday
                    ? 'font-semibold text-primary ring-[1.5px] ring-inset ring-primary'
                    : ''
                "
                :aria-label="cellAria(cell)"
              >
                <span
                  :class="
                    cell.state === 'future' ? 'text-muted-foreground/50' : ''
                  "
                >
                  {{ cell.day }}
                </span>
                <!-- 打卡点态：书绿实心 / 沙框空心 -->
                <span
                  class="h-1.5 w-1.5 rounded-full"
                  :class="
                    cell.state === 'checked'
                      ? 'bg-success'
                      : cell.state === 'future'
                        ? 'border border-border/70'
                        : 'border border-muted-foreground/40'
                  "
                  aria-hidden="true"
                />
              </div>
            </template>
          </div>
        </CardContent>
      </Card>

      <!-- 当日计划分配侧卡（mock） -->
      <Card>
        <CardHeader>
          <CardTitle class="font-display text-base">今日计划分配</CardTitle>
          <CardDescription>
            {{ month + 1 }} 月 {{ dayOfMonth }} 日 · 共
            {{ todaySchedule.length }} 节
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ul class="space-y-4">
            <li
              v-for="item in todaySchedule"
              :key="item.chapter"
              class="flex items-start justify-between gap-3"
            >
              <div class="min-w-0">
                <p class="truncate text-sm font-medium">{{ item.chapter }}</p>
                <p class="mt-0.5 text-xs tabular-nums text-muted-foreground">
                  {{ item.phase }} · {{ item.time }}
                </p>
              </div>
              <Badge
                :variant="item.done ? 'success' : 'outline'"
                class="shrink-0"
              >
                {{ item.done ? "已完成" : "待完成" }}
              </Badge>
            </li>
          </ul>
          <!-- 假按钮：仅视觉占位，无跳转逻辑 -->
          <Button variant="outline" type="button" class="mt-6 w-full">
            查看计划
          </Button>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
