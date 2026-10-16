<script setup lang="ts">
/**
 * DayDrawer：日钻取抽屉（右滑弹层）。
 * - title 当天日期（衬线 font-display）+ 各计划分组的当日分配章节清单
 *   （阶段名 + 章节名），每条带完成徽章（✓ 已完成 / ◌ 未完成，
 *   口径 = 章 status=complete）
 * - 每组头部「编辑学习日」次级按钮 → 内嵌周几 chip 群（初始值 = 该计划
 *   studyDays），保存经宿主调 set_plan_study_days 后触发重算
 * - 每组尾部「当日已学」checkbox：打卡独立于章节完成（Task E 双语义位），
 *   切换经宿主调 set_study_day_mark 落库（幂等）
 * - 交互契约：Esc 关闭 + 点击遮罩关闭 + 开启时 focus 移入抽屉
 *   （role=dialog aria-modal，键盘可达性对齐 Web Interface Guidelines）
 * - 数据全部来自宿主注入的真计划分配（useStudyAllocation 纯函数产出）
 */
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import { CalendarDays, Check, ChevronDown, Circle, X } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { normalizeStudyDays } from "@/composables/useStudyAllocation";

/** 单个计划当日的分配条目 */
export interface DrawerAssignment {
  phaseIndex: number;
  phaseName: string;
  chapterIndex: number;
  chapterName: string;
  /** 章 status=complete */
  done: boolean;
}

/** 按计划分组的当日安排（含打卡态与学习日编辑初值） */
export interface DrawerPlanGroup {
  planId: string;
  planTitle: string;
  /** 该日已打卡（当日已学） */
  marked: boolean;
  /** 该计划的每周学习日（1=周一..7=周日，编辑 chips 初值） */
  studyDays: number[];
  assignments: DrawerAssignment[];
}

const props = defineProps<{
  open: boolean;
  date: Date | null;
  groups: DrawerPlanGroup[];
  /** 打卡/保存学习日请求在飞（禁用对应控件防重复提交） */
  saving: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "toggle-mark", planId: string, done: boolean): void;
  (e: "save-study-days", planId: string, days: number[]): void;
  /** 跳转学习计划详情（组头计划名点击） */
  (e: "open-plan", planId: string): void;
}>();

const panelRef = ref<HTMLElement | null>(null);

/** 学习日编辑态：正在编辑的计划 id + 草稿数组（null = 收起） */
const editingPlanId = ref<string | null>(null);
const draftDays = ref<number[]>([]);

/** 打卡在飞的 planId 集合（checkbox 禁用防抖） */
const markingIds = ref<Set<string>>(new Set());

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
  props.groups.reduce((sum, g) => sum + g.assignments.length, 0),
);
const doneCount = computed(() =>
  props.groups.reduce(
    (sum, g) => sum + g.assignments.filter((a) => a.done).length,
    0,
  ),
);

/** 进入/退出某计划的学习日编辑态 */
function startEditStudyDays(group: DrawerPlanGroup) {
  editingPlanId.value = group.planId;
  draftDays.value = normalizeStudyDays(group.studyDays);
}
function cancelEditStudyDays() {
  editingPlanId.value = null;
  draftDays.value = [];
}
function toggleDraftDay(day: number) {
  const idx = draftDays.value.indexOf(day);
  if (idx >= 0) {
    // 至少保留一个学习日（与 Rust/Python 校验同语义，前置拦截）
    if (draftDays.value.length === 1) return;
    draftDays.value.splice(idx, 1);
  } else {
    draftDays.value.push(day);
    draftDays.value.sort((a, b) => a - b);
  }
}
function submitStudyDays(group: DrawerPlanGroup) {
  if (props.saving) return;
  emit("save-study-days", group.planId, [...draftDays.value]);
  cancelEditStudyDays();
}

/** 打卡切换（在飞由宿主回写 marked 后自然同步；这里仅禁用本组控件） */
function onToggleMark(group: DrawerPlanGroup, done: boolean) {
  if (markingIds.value.has(group.planId) || props.saving) return;
  markingIds.value = new Set(markingIds.value).add(group.planId);
  emit("toggle-mark", group.planId, done);
}
/** 宿主在请求收尾后调用清在飞标记（经 defineExpose） */
function settleMark(planId: string) {
  const next = new Set(markingIds.value);
  next.delete(planId);
  markingIds.value = next;
}
defineExpose({ settleMark });

/** 抽屉关闭时复位编辑态 */
function requestClose() {
  cancelEditStudyDays();
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
      cancelEditStudyDays();
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
              {{ date.getFullYear() }} 年，共 {{ totalCount }} 项安排，已完成
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
            <!-- 组头：计划名 + 编辑学习日 -->
            <div
              class="flex items-center justify-between gap-3 border-b border-border px-4 py-3"
            >
              <button
                type="button"
                class="min-w-0 cursor-pointer truncate text-left text-sm font-medium text-foreground transition-colors hover:text-primary hover:underline hover:underline-offset-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :title="`打开计划详情：${group.planTitle}`"
                :aria-label="`打开计划详情：${group.planTitle}`"
                @click="emit('open-plan', group.planId)"
              >
                {{ group.planTitle }}
              </button>
              <Button
                variant="ghost"
                size="sm"
                type="button"
                class="h-7 shrink-0 gap-1 px-2 text-xs text-muted-foreground"
                :aria-expanded="editingPlanId === group.planId"
                @click="
                  editingPlanId === group.planId
                    ? cancelEditStudyDays()
                    : startEditStudyDays(group)
                "
              >
                <CalendarDays class="h-3.5 w-3.5" aria-hidden="true" />
                编辑学习日
                <ChevronDown
                  class="h-3 w-3 transition-transform duration-200"
                  :class="editingPlanId === group.planId ? 'rotate-180' : ''"
                  aria-hidden="true"
                />
              </Button>
            </div>

            <!-- 学习日编辑（内嵌展开） -->
            <div
              v-if="editingPlanId === group.planId"
              class="space-y-3 border-b border-border bg-accent/30 px-4 py-3"
            >
              <p class="text-xs text-muted-foreground">
                每周学习日（重新排布该计划的日历安排）
              </p>
              <div
                class="flex flex-wrap gap-1.5"
                role="group"
                aria-label="每周学习日选择"
              >
                <button
                  v-for="(label, i) in WEEKDAY_LABELS"
                  :key="label"
                  type="button"
                  class="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :aria-pressed="draftDays.includes(i + 1)"
                  @click="toggleDraftDay(i + 1)"
                >
                  <Badge
                    :variant="draftDays.includes(i + 1) ? 'default' : 'outline'"
                    class="cursor-pointer select-none px-2 transition-opacity hover:opacity-80"
                  >
                    周{{ label }}
                  </Badge>
                </button>
              </div>
              <div class="flex justify-end gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  type="button"
                  class="h-7 text-xs"
                  @click="cancelEditStudyDays"
                >
                  取消
                </Button>
                <Button
                  size="sm"
                  type="button"
                  class="h-7 text-xs"
                  :disabled="saving"
                  @click="submitStudyDays(group)"
                >
                  保存
                </Button>
              </div>
            </div>

            <!-- 条目清单 -->
            <ul v-if="group.assignments.length > 0" class="px-4 py-3">
              <li
                v-for="a in group.assignments"
                :key="`${a.phaseIndex}-${a.chapterIndex}`"
                class="flex items-start justify-between gap-3 border-b border-border/60 py-2.5 last:border-b-0"
              >
                <div class="min-w-0">
                  <p class="truncate text-sm">
                    {{ a.chapterName }}
                  </p>
                  <p
                    class="mt-0.5 truncate text-xs tabular-nums text-muted-foreground"
                  >
                    阶段 {{ a.phaseIndex + 1 }} · {{ a.phaseName }}
                  </p>
                </div>
                <Badge
                  :variant="a.done ? 'success' : 'outline'"
                  class="shrink-0"
                >
                  <Check
                    v-if="a.done"
                    class="mr-1 h-3 w-3"
                    aria-hidden="true"
                  />
                  <Circle v-else class="mr-1 h-3 w-3" aria-hidden="true" />
                  {{ a.done ? "已完成" : "未完成" }}
                </Badge>
              </li>
            </ul>
            <p
              v-else
              class="px-4 py-4 text-xs text-muted-foreground"
              aria-live="polite"
            >
              该计划当日无学习安排
            </p>

            <!-- 组尾：当日已学打卡 -->
            <label
              class="flex cursor-pointer items-center gap-3 border-t border-border bg-muted/20 px-4 py-3 transition-colors duration-200 hover:bg-accent/40"
            >
              <input
                type="checkbox"
                class="h-4 w-4 accent-success"
                :checked="group.marked"
                :disabled="markingIds.has(group.planId) || saving"
                :aria-label="`${group.planTitle} 当日已学`"
                @change="onToggleMark(group, !group.marked)"
              />
              <span class="text-sm">
                当日已学
                <span class="ml-1 text-xs text-muted-foreground">
                  勾选标记该计划当天已完成
                </span>
              </span>
            </label>
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
