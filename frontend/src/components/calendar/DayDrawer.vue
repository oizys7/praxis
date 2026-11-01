<script setup lang="ts">
/**
 * DayDrawer：日钻取抽屉（右滑弹层）。
 * - title 当天日期（衬线 font-display）+ 各计划分组的当日章节清单
 *   （阶段名 + 章节名）
 * - 行级打卡（每章独立勾选，study_schedule.done）：勾选 = 该章已学打卡，
 *   仅允许今天及之前的日程勾选（未来行禁用，与 Rust 校验一致）
 * - 组头为计划名（可点跳详情）；学习日编辑入口在日历页左侧计划进度面板
 * - 交互契约：Esc 关闭 + 点击遮罩关闭 + 开启时 focus 移入抽屉
 *   （role=dialog aria-modal，键盘可达性对齐 Web Interface Guidelines）
 * - 数据全部来自宿主注入的月历查询真数据（query_month_calendar 按天组织）
 */
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import { Check, Circle, LoaderCircle, X } from "lucide-vue-next";
import type { ScheduleItem } from "@/types/agent";

/** 按计划分组的当日安排（来自月历数据的该天条目） */
export interface DrawerPlanGroup {
  planId: string;
  planTitle: string;
  items: ScheduleItem[];
}

const props = defineProps<{
  open: boolean;
  date: Date | null;
  groups: DrawerPlanGroup[];
  /** 打卡请求在飞（禁用对应行防重复提交） */
  saving: boolean;
  /** 该抽屉日是否允许打卡（今天及之前为 true；未来日期整组只读展示） */
  markable: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  /** 行级打卡切换（每章独立勾选） */
  (e: "toggle-item", planId: string, item: ScheduleItem, done: boolean): void;
  /** 跳转学习计划详情（组头计划名点击） */
  (e: "open-plan", planId: string): void;
}>();

const panelRef = ref<HTMLElement | null>(null);

/** 打卡在飞行键集合（'planId:phaseIdx:chapterIdx'，行级禁用防抖） */
const togglingKeys = ref<Set<string>>(new Set());

/** 星期几中文（「一」..「日」，列序即 1..7） */
const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;

const weekdayLabel = computed(() => {
  if (!props.date) return "";
  return WEEKDAY_LABELS[props.date.getDay()];
});

/** 标题「10 月 14 日 · 周二」 */
const titleText = computed(() => {
  if (!props.date) return "";
  return `${props.date.getMonth() + 1} 月 ${props.date.getDate()} 日 · 周${weekdayLabel.value}`;
});

/** 全部组条目数与完成数（头部摘要） */
const totalCount = computed(() =>
  props.groups.reduce((sum, g) => sum + g.items.length, 0),
);
const doneCount = computed(() =>
  props.groups.reduce(
    (sum, g) => sum + g.items.filter((item) => item.done).length,
    0,
  ),
);

function itemKey(planId: string, item: ScheduleItem): string {
  return `${planId}:${item.phaseIdx}:${item.chapterIdx}`;
}

/** 行级打卡切换（在飞行由宿主刷新数据后自然同步；这里仅禁用本行） */
function onToggleItem(group: DrawerPlanGroup, item: ScheduleItem) {
  const key = itemKey(group.planId, item);
  if (togglingKeys.value.has(key) || props.saving || !props.markable) return;
  togglingKeys.value = new Set(togglingKeys.value).add(key);
  emit("toggle-item", group.planId, item, !item.done);
}

/** 宿主在请求收尾后调用清在飞行标记（经 defineExpose） */
function settleToggle(planId: string, item: ScheduleItem) {
  const key = itemKey(planId, item);
  const next = new Set(togglingKeys.value);
  next.delete(key);
  togglingKeys.value = next;
}
defineExpose({ settleToggle });

/** 该行打卡是否在飞（圈内换 spinner） */
function isToggling(planId: string, item: ScheduleItem): boolean {
  return togglingKeys.value.has(itemKey(planId, item));
}

/**
 * 印章态（自助习室纸面调色板的打卡隐喻）：已学 = 书绿实心章 + 纸白对勾，
 * 未学 = 细描边空心圈，行 hover 时描边转墨蓝、对勾淡显暗示可点。
 * 只做颜色/变换过渡（150-300ms），按压反馈由全局 button:active 提供。
 */
function stampClass(item: ScheduleItem): string {
  if (item.done) {
    return "border-success bg-success text-primary-foreground shadow-sm hover:border-success/70";
  }
  return "border-border bg-background text-transparent group-hover/row:border-primary/60 group-hover/row:text-primary/25";
}

function requestClose() {
  emit("close");
}

/**
 * Esc 关闭：open 期间挂 document 级 keydown（抽屉是顶层浮层，
 * 无需受组件树层级影响）
 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") requestClose();
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      document.addEventListener("keydown", onKeydown);
      // 开启时 focus 移入抽屉容器，键盘用户从抽屉内部开始 Tab
      void nextTick(() => panelRef.value?.focus());
    } else {
      document.removeEventListener("keydown", onKeydown);
    }
  },
);

onUnmounted(() => document.removeEventListener("keydown", onKeydown));
</script>

<template>
  <Teleport to="body">
    <!-- 遮罩：点击空白处关闭（.self 保证只响应遮罩自身点击） -->
    <Transition name="drawer-mask">
      <div
        v-if="open"
        class="fixed inset-0 z-40 bg-foreground/40"
        aria-hidden="true"
        @click.self="requestClose"
      />
    </Transition>

    <!-- 右滑抽屉面板 -->
    <Transition name="drawer-panel">
      <section
        v-if="open && date"
        ref="panelRef"
        role="dialog"
        aria-modal="true"
        aria-labelledby="calendar-drawer-title"
        tabindex="-1"
        class="fixed right-0 top-0 z-50 flex h-full w-full max-w-md flex-col border-l border-border bg-card shadow-xl focus:outline-none"
      >
        <!-- 头部：日期标题 + 关闭 -->
        <header
          class="flex items-start justify-between gap-4 border-b border-border px-6 py-5"
        >
          <div>
            <h3
              id="calendar-drawer-title"
              class="font-display text-lg font-semibold tracking-tight"
            >
              {{ titleText }}
            </h3>
            <p class="mt-1 text-xs tabular-nums text-muted-foreground">
              {{ date.getFullYear() }} 年，共 {{ totalCount }} 项安排，已学
              {{ doneCount }} 项
            </p>
          </div>
          <button
            type="button"
            class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="关闭抽屉"
            @click="requestClose"
          >
            <X class="h-4 w-4" aria-hidden="true" />
          </button>
        </header>

        <!-- 按计划分组的当日安排 -->
        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto px-6 py-5">
          <section
            v-for="group in groups"
            :key="group.planId"
            class="rounded-lg border border-border"
            :aria-label="`${group.planTitle} 当日安排`"
          >
            <!-- 组头：计划名（可点跳详情）+ 组完成度小字 -->
            <div
              class="flex items-center justify-between gap-3 border-b border-border px-4 py-3"
            >
              <button
                type="button"
                class="min-w-0 cursor-pointer truncate text-left text-sm font-medium text-foreground transition-colors hover:text-primary hover:underline hover:underline-offset-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                v-tip="`打开计划详情：${group.planTitle}`"
                :aria-label="`打开计划详情：${group.planTitle}`"
                @click="emit('open-plan', group.planId)"
              >
                {{ group.planTitle }}
              </button>
              <span
                class="shrink-0 text-xs tabular-nums text-muted-foreground"
                :aria-label="`${group.planTitle} 当日完成度`"
              >
                {{ group.items.filter((item) => item.done).length }}/{{
                  group.items.length
                }}
                已学
              </span>
            </div>

            <!-- 行级勾选清单（每章独立打卡） -->
            <ul v-if="group.items.length > 0" class="px-4 py-3">
              <li
                v-for="item in group.items"
                :key="`${item.phaseIdx}-${item.chapterIdx}`"
                class="border-b border-border/60 last:border-b-0"
              >
                <label
                  class="group/row -mx-2 flex cursor-pointer items-center justify-between gap-3 rounded-lg px-2 py-2.5 transition-colors duration-200 hover:bg-muted/50"
                  :class="{
                    'cursor-not-allowed opacity-60 hover:bg-transparent':
                      !markable,
                  }"
                >
                  <div class="min-w-0">
                    <p class="truncate text-sm">
                      {{ item.chapterTitle }}
                    </p>
                    <p
                      class="mt-0.5 truncate text-xs tabular-nums text-muted-foreground"
                    >
                      阶段 {{ item.phaseIdx + 1 }} · {{ item.phaseName }}
                    </p>
                  </div>
                  <span class="flex shrink-0 items-center gap-2">
                    <input
                      type="checkbox"
                      class="peer sr-only"
                      :checked="item.done"
                      :disabled="
                        !markable || saving || isToggling(group.planId, item)
                      "
                      :aria-label="`${item.chapterTitle} 已学打卡`"
                      @change="onToggleItem(group, item)"
                    />
                    <!-- 印章：已学实心盖章 / 未学空心待盖。以 done 为 key 重建，
                         打卡成功那一刻重放 stamp-in（盖章压下，240ms 弹性曲线） -->
                    <span
                      :key="item.done ? 'done' : 'todo'"
                      class="flex h-6 w-6 items-center justify-center rounded-full border transition-all duration-200 peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background"
                      :class="[stampClass(item), item.done ? 'stamp-in' : '']"
                      aria-hidden="true"
                    >
                      <LoaderCircle
                        v-if="isToggling(group.planId, item)"
                        class="h-3 w-3 animate-spin"
                      />
                      <Check
                        v-else
                        class="h-3 w-3 transition-[opacity,transform] duration-200"
                        :class="
                          item.done
                            ? 'scale-100 opacity-100'
                            : 'scale-50 opacity-0'
                        "
                        stroke-width="3.5"
                      />
                    </span>
                    <span
                      class="w-7 text-xs transition-colors duration-200"
                      :class="
                        item.done
                          ? 'font-medium text-success'
                          : 'text-muted-foreground/70'
                      "
                    >
                      {{ item.done ? "已学" : "未学" }}
                    </span>
                  </span>
                </label>
              </li>
            </ul>
            <p
              v-else
              class="px-4 py-4 text-xs text-muted-foreground"
              aria-live="polite"
            >
              该计划当日无学习安排
            </p>
          </section>

          <!-- 空态：当天无任何安排 -->
          <div
            v-if="groups.length === 0"
            class="flex flex-col items-center py-16 text-center"
          >
            <Circle
              class="h-8 w-8 text-muted-foreground/40"
              aria-hidden="true"
            />
            <p class="mt-3 text-sm text-muted-foreground">这一天没有学习安排</p>
          </div>
        </div>
      </section>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* 抽屉动效：遮罩淡入 + 面板右滑（prefers-reduced-motion 由全局样式退避） */
.drawer-mask-enter-active,
.drawer-mask-leave-active {
  transition: opacity 0.2s ease;
}
.drawer-mask-enter-from,
.drawer-mask-leave-to {
  opacity: 0;
}
.drawer-panel-enter-active,
.drawer-panel-leave-active {
  transition: transform 0.2s ease;
}
.drawer-panel-enter-from,
.drawer-panel-leave-to {
  transform: translateX(100%);
}
</style>
