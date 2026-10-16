<script setup lang="ts">
/**
 * DayDrawer：日钻取抽屉（右滑弹层）。
 * - title 当天日期（衬线 font-display）+ 该日各计划的分配章节清单
 *   （阶段名 + 章节名 + 预估时段），每条带完成徽章（✓ 已完成 / ◌ 未完成）
 * - 底部「当日已学」checkbox：纯视觉 v-model 本地状态（初始化自当日完成度，
 *   持久化接口供 Task E 替换）
 * - 交互契约：Esc 关闭 + 点击遮罩关闭 + 开启时 focus 移入抽屉
 *   （role=dialog aria-modal，键盘可达性对齐 Web Interface Guidelines）
 * - 数据全部来自 calendarMocks（Task E 将替换为真数据源）
 */
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import { Check, Circle, X } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { dayAssignments, type DayAssignmentMock } from "@/calendarMocks";

const props = defineProps<{ open: boolean; date: Date | null }>();

const emit = defineEmits<{ (e: "close"): void }>();

const panelRef = ref<HTMLElement | null>(null);

/** 「当日已学」勾选态（视觉演示；持久化接口供 Task E 替换） */
const dayLearned = ref(false);

/** 抽屉日期的分配清单（date 为 null 时为空，配合 v-if 不渲染） */
const assignments = computed<DayAssignmentMock[]>(() =>
  props.date ? dayAssignments(props.date) : [],
);

/** 完成条数 */
const doneCount = computed(
  () => assignments.value.filter((a) => a.done).length,
);

/** 星期几中文（「一」..「日」） */
const weekdayLabel = computed(() => {
  if (!props.date) return "";
  return ["日", "一", "二", "三", "四", "五", "六"][props.date.getDay()];
});

/** 标题「10 月 14 日 · 周二」 */
const titleText = computed(() => {
  if (!props.date) return "";
  return `${props.date.getMonth() + 1} 月 ${props.date.getDate()} 日 · 周${weekdayLabel.value}`;
});

/**
 * Esc 关闭：open 期间挂 document 级 keydown（抽屉是顶层浮层，
 * 无需受组件树层级影响）
 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      // 勾选态与当日完成度对齐（全部完成则默认勾上），视觉自洽
      dayLearned.value =
        assignments.value.length > 0 && assignments.value.every((a) => a.done);
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
        @click.self="emit('close')"
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
            <p class="mt-1 text-xs text-muted-foreground">
              {{ date.getFullYear() }} 年 · 共 {{ assignments.length }} 项安排 ·
              已完成 {{ doneCount }}
            </p>
          </div>
          <button
            type="button"
            class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="关闭抽屉"
            @click="emit('close')"
          >
            <X class="h-4 w-4" aria-hidden="true" />
          </button>
        </header>

        <!-- 分配清单 -->
        <div class="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <ul v-if="assignments.length > 0" class="space-y-3">
            <li
              v-for="a in assignments"
              :key="a.chapterName"
              class="rounded-lg border border-border p-4"
            >
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <p class="truncate text-sm font-medium">
                    {{ a.chapterName }}
                  </p>
                  <p
                    class="mt-1 truncate text-xs tabular-nums text-muted-foreground"
                  >
                    {{ a.phaseName }} · {{ a.planTitle }}
                  </p>
                  <p class="mt-0.5 text-xs tabular-nums text-muted-foreground">
                    {{ a.timeSlot }}
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
              </div>
            </li>
          </ul>

          <!-- 空态：当天无安排 -->
          <div v-else class="flex flex-col items-center py-16 text-center">
            <Circle
              class="h-8 w-8 text-muted-foreground/40"
              aria-hidden="true"
            />
            <p class="mt-3 text-sm text-muted-foreground">这一天没有学习安排</p>
          </div>
        </div>

        <!-- 底部：当日已学勾选（视觉演示） -->
        <footer class="border-t border-border px-6 py-5">
          <label
            class="flex cursor-pointer items-center gap-3 rounded-lg border border-border px-4 py-3 transition-colors duration-200 hover:bg-accent/50"
          >
            <input
              v-model="dayLearned"
              type="checkbox"
              class="h-4 w-4 accent-success"
            />
            <span class="text-sm">
              当日已学
              <span class="ml-1 text-xs text-muted-foreground">
                勾选标记当天计划全部完成
              </span>
            </span>
          </label>
        </footer>
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
