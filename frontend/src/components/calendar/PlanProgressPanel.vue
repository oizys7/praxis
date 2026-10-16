<script setup lang="ts">
/**
 * PlanProgressPanel：打卡日历页左侧的「计划进度管理」面板。
 *
 * 以计划（项目）为粒度管理学习进度，按打卡口径三态分组：
 * - 学习中：0 < 已打卡天数 < 总分配天数
 * - 未开始：已打卡天数为 0
 * - 已结束：已打卡天数 ≥ 总分配天数
 * 计划行级 generating/failed 不参与三态，单独以徽章提示。
 *
 * 学习日编辑（自 DayDrawer 迁入）：行内七枚周几 chip，点击即切换，
 * 由父组件乐观保存（toggle-study-day）。标题可点 → 跳计划详情。
 */
import { computed } from "vue";
import { SquareCheck } from "lucide-vue-next";
import { computeDayAssignments } from "@/composables/useStudyAllocation";
import type { PlanRecord } from "@/types/agent";

const props = defineProps<{
  records: PlanRecord[];
  /** planId → 已打卡 dayKey 集合 */
  marksByPlan: Map<string, Set<string>>;
  /** planId → 每周学习日（本地视图态，编辑后由父组件覆盖） */
  studyDaysByPlan: Map<string, number[]>;
  /** 保存请求在飞（chips 禁用防抖） */
  saving?: boolean;
}>();

/** 星期几中文（「一」..「日」，索引即 1..7） */
const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;

const emit = defineEmits<{
  (e: "open-plan", planId: string): void;
  (e: "toggle-study-day", planId: string, day: number): void;
}>();

interface ProgressRow {
  record: PlanRecord;
  /** 总分配学习天数 */
  total: number;
  /** 已打卡天数 */
  done: number;
  /** 生效学习日（本地视图态优先） */
  days: number[];
}

/** 逐计划聚合分配天数与打卡数（与月历/抽屉同一口径） */
const rows = computed<ProgressRow[]>(() =>
  props.records.map((record) => {
    const marks = props.marksByPlan.get(record.id) ?? new Set<string>();
    const view = {
      ...record,
      studyDays: props.studyDaysByPlan.get(record.id) ?? record.studyDays ?? [],
    };
    const assignments = computeDayAssignments(view, marks);
    return {
      record,
      total: assignments.length,
      done: assignments.filter((a) => a.done).length,
      days: view.studyDays,
    };
  }),
);

/** 三态分组：学习中在前，未开始次之，已结束殿后 */
const groups = computed(() => {
  const active: ProgressRow[] = [];
  const notStarted: ProgressRow[] = [];
  const finished: ProgressRow[] = [];
  for (const row of rows.value) {
    // 生成中/失败的计划不支持学习，一律归入「未开始」（状态点区分）
    if (row.record.status === "generating" || row.record.status === "failed") {
      notStarted.push(row);
      continue;
    }
    if (row.done === 0) notStarted.push(row);
    else if (row.done >= row.total && row.total > 0) finished.push(row);
    else active.push(row);
  }
  return [
    { key: "active", label: "学习中", rows: active },
    { key: "pending", label: "未开始", rows: notStarted },
    { key: "done", label: "已结束", rows: finished },
  ];
});

/** 行状态点样式（12px 圆点：朱批叉点 / 旋转墨环 / 空心 / 书绿勾 / 墨蓝半环） */
function dotClass(row: ProgressRow): string {
  if (row.record.status === "failed")
    return "border-destructive bg-destructive/25";
  if (row.record.status === "generating")
    return "border-primary/40 border-t-primary animate-spin";
  if (row.done === 0) return "border-muted-foreground/40 bg-transparent";
  if (row.done >= row.total && row.total > 0)
    return "border-success bg-success";
  return "border-primary bg-primary/20";
}

/** 状态点读屏文案（点即状态，行内不再重复文字徽章） */
function stateLabel(row: ProgressRow): string {
  if (row.record.status === "failed") return "生成失败";
  if (row.record.status === "generating") return "生成中";
  if (row.done === 0) return "未开始";
  if (row.done >= row.total && row.total > 0) return "已结束";
  return "学习中";
}

/** 生成中/失败的计划锁定学习日编辑（不支持学习） */
function isLocked(row: ProgressRow): boolean {
  return row.record.status === "generating" || row.record.status === "failed";
}

function percent(row: ProgressRow): number {
  return row.total === 0
    ? 0
    : Math.min(100, Math.round((row.done / row.total) * 100));
}

function dayLabel(day: number): string {
  return WEEKDAY_LABELS[day - 1] ?? String(day);
}

/** chip 是否选中：当前生效学习日 */
function isSelected(row: ProgressRow, day: number): boolean {
  return row.days.includes(day);
}
</script>

<template>
  <Card class="h-fit">
    <CardContent class="space-y-5 p-4">
      <div>
        <h3
          class="font-display text-xl font-bold tracking-[0.4em] text-foreground"
        >
          计划进度
        </h3>
        <div class="mt-1.5 h-0.5 w-10 bg-primary/70" role="presentation" />
        <p class="mt-2 text-xs text-muted-foreground">
          点周几可调整学习日，改动立即生效并重排日历
        </p>
      </div>

      <section
        v-for="group in groups"
        :key="group.key"
        class="space-y-1"
        :aria-label="`${group.label}（${group.rows.length}）`"
      >
        <!-- 组题：状态名 + 计数 + hairline -->
        <div class="flex items-center gap-2.5">
          <span class="text-xs font-medium text-muted-foreground">
            {{ group.label }}（{{ group.rows.length }}）
          </span>
          <div class="h-px flex-1 bg-primary/10" role="presentation" />
        </div>

        <!-- 空组淡示 -->
        <p
          v-if="group.rows.length === 0"
          class="py-1 text-xs text-muted-foreground/70"
        >
          {{
            group.key === "done"
              ? "还没有学完的计划"
              : group.key === "pending"
                ? "没有待开始的计划"
                : "没有进行中的计划"
          }}
        </p>

        <!-- 行列表：组内限高，超出内部滚动（细滚动条） -->
        <div
          v-if="group.rows.length > 0"
          class="max-h-80 space-y-1 overflow-y-auto pr-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:bg-border"
        >
          <!-- 行：状态点 + 标题 + 进度 + 学习日 chips -->
          <div
            v-for="row in group.rows"
            :key="row.record.id"
            class="group rounded-lg px-2 py-2.5 transition-colors hover:bg-secondary/40"
          >
            <div class="flex items-center gap-2.5">
              <span
                class="h-3 w-3 shrink-0 rounded-full border-2"
                :class="dotClass(row)"
                role="img"
                :aria-label="stateLabel(row)"
              />
              <button
                type="button"
                class="min-w-0 flex-1 cursor-pointer truncate text-left text-sm font-medium text-foreground transition-colors hover:text-primary hover:underline hover:underline-offset-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :title="`打开计划详情：${row.record.title}`"
                :aria-label="`打开计划详情：${row.record.title}，已完成 ${row.done} 天，共 ${row.total} 天`"
                @click="emit('open-plan', row.record.id)"
              >
                {{ row.record.title }}
              </button>
              <span
                v-if="!isLocked(row)"
                class="shrink-0 text-xs tabular-nums text-muted-foreground"
              >
                {{ row.done }}/{{ row.total }} 天
              </span>
            </div>

            <!-- 进度条（hairline 细条） -->
            <div
              v-if="
                row.record.status !== 'generating' &&
                row.record.status !== 'failed' &&
                row.total > 0
              "
              class="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
              role="progressbar"
              :aria-valuenow="percent(row)"
              aria-valuemin="0"
              aria-valuemax="100"
              :aria-label="`学习进度 ${percent(row)}%`"
            >
              <div
                class="h-full rounded-full transition-[width] duration-300"
                :class="
                  row.done >= row.total && row.total > 0
                    ? 'bg-success'
                    : 'bg-primary'
                "
                :style="{ width: `${percent(row)}%` }"
              />
            </div>

            <!-- 学习日 chips（点击即切换，父组件乐观保存）；锁定计划禁用 -->
            <div
              class="mt-2 flex flex-wrap gap-1"
              :class="isLocked(row) ? 'opacity-50' : ''"
              role="group"
              :aria-label="`${row.record.title} 每周学习日${isLocked(row) ? '（该计划暂不可学习）' : ''}`"
            >
              <button
                v-for="day in [1, 2, 3, 4, 5, 6, 7]"
                :key="day"
                type="button"
                class="cursor-pointer rounded-md px-1.5 py-0.5 text-[11px] leading-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                :class="
                  isSelected(row, day)
                    ? 'bg-primary/10 font-medium text-primary ring-1 ring-inset ring-primary/30'
                    : 'text-muted-foreground/70 hover:bg-muted hover:text-foreground'
                "
                :aria-pressed="isSelected(row, day)"
                :aria-label="`每周${dayLabel(day)}${isSelected(row, day) ? '（已选）' : '（未选）'}`"
                :disabled="saving || isLocked(row)"
                @click="emit('toggle-study-day', row.record.id, day)"
              >
                {{ dayLabel(day) }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 全空兜底（records 空时父组件不会渲染面板，此为防御） -->
      <p
        v-if="records.length === 0"
        class="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <SquareCheck class="h-4 w-4" aria-hidden="true" />
        暂无计划
      </p>
    </CardContent>
  </Card>
</template>
