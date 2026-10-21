<script setup lang="ts">
/**
 * PlanProgressPanel：打卡日历页左侧的「计划进度管理」面板。
 *
 * 按学习状态（plans.learning_status，真数据）三分类：
 * - 学习中 active / 未开始 not_started / 已完成 completed；
 * 生成中/失败（行级 generating/failed）不支持学习，归「未开始」组以状态点区分。
 *
 * 学习日 chips 仅本地暂存（点击不调接口）：
 * - 未开始行：展开区选学习日 → 点【从今天开始】一次性提交（start-learning），
 *   Rust 落定逐章日程且不打卡今天；
 * - 学习中行：chips 变更后行内出现「保存学习日」确认（update-study-days），
 *   Rust 保留今天之前与今天已完成、只重排未来。
 * 进度口径 = 章节完成度（chapterDone/chapterTotal，list_plans 随行携带）。
 */
import { computed, ref } from "vue";
import { CalendarCheck, Play, SquareCheck } from "lucide-vue-next";
import type { PlanSummary } from "@/types/agent";

const props = defineProps<{
  summaries: PlanSummary[];
  /** 保存请求在飞（chips 禁用防抖） */
  saving?: boolean;
}>();

/** 星期几中文（「一」..「日」，索引即 1..7） */
const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;

const emit = defineEmits<{
  (e: "open-plan", planId: string): void;
  /** 学习中行保存学习日：今天之前与今天已完成不动，未来重排 */
  (e: "update-study-days", planId: string, days: number[]): void;
  /** 开始学习（未开始行）：提交学习日，从今天起落定日程（不打卡今天） */
  (e: "start-learning", planId: string, days: number[]): void;
}>();

interface ProgressRow {
  summary: PlanSummary;
  /** 生效学习日（本地草稿优先，回退 summary.studyDays） */
  days: number[];
  /** chips 相对库值有未保存改动 */
  dirty: boolean;
}

/** 学习日本地草稿：planId → 未保存的每周学习日（chips 只改这里，不调接口） */
const draftDays = ref<Map<string, number[]>>(new Map());

/** 展开开始学习流程的计划 id（未开始组行内编辑，单开） */
const expandedStart = ref<string | null>(null);

function effectiveDays(summary: PlanSummary): number[] {
  return draftDays.value.get(summary.id) ?? summary.studyDays ?? [];
}

function isDirty(summary: PlanSummary): boolean {
  const draft = draftDays.value.get(summary.id);
  if (draft === undefined) return false;
  const base = [...(summary.studyDays ?? [])].sort().join(",");
  return [...draft].sort().join(",") !== base;
}

/** chip 点击：仅更新本地草稿（至少保留一天；由父组件统一提交） */
function toggleDraftDay(summary: PlanSummary, day: number) {
  const current = [...effectiveDays(summary)];
  const next = current.includes(day)
    ? current.filter((d) => d !== day)
    : [...current, day].sort((a, b) => a - b);
  if (next.length === 0) return;
  draftDays.value = new Map(draftDays.value).set(summary.id, next);
}

function submitStart(summary: PlanSummary) {
  emit("start-learning", summary.id, [...effectiveDays(summary)]);
}

function submitUpdate(summary: PlanSummary) {
  emit("update-study-days", summary.id, [...effectiveDays(summary)]);
}

/** 逐行视图态（草稿合并） */
const rows = computed<ProgressRow[]>(() =>
  props.summaries.map((summary) => ({
    summary,
    days: effectiveDays(summary),
    dirty: isDirty(summary),
  })),
);

/** 三分类分组：学习中在前，未开始次之，已完成殿后 */
const groups = computed(() => {
  const active: ProgressRow[] = [];
  const notStarted: ProgressRow[] = [];
  const completed: ProgressRow[] = [];
  for (const row of rows.value) {
    // 生成中/失败的计划不支持学习流程，一律归「未开始」（状态点区分）
    if (
      row.summary.status === "generating" ||
      row.summary.status === "failed" ||
      row.summary.learningStatus === "not_started"
    ) {
      notStarted.push(row);
    } else if (row.summary.learningStatus === "completed") {
      completed.push(row);
    } else {
      active.push(row);
    }
  }
  return [
    { key: "active", label: "学习中", rows: active },
    { key: "pending", label: "未开始", rows: notStarted },
    { key: "done", label: "已完成", rows: completed },
  ];
});

/** 行状态点样式（12px 圆点：朱批叉点 / 旋转墨环 / 空心 / 书绿勾 / 墨蓝半环） */
function dotClass(row: ProgressRow): string {
  if (row.summary.status === "failed")
    return "border-destructive bg-destructive/25";
  if (row.summary.status === "generating")
    return "border-primary/40 border-t-primary animate-spin";
  if (row.summary.learningStatus === "completed")
    return "border-success bg-success";
  if (row.summary.learningStatus === "active")
    return "border-primary bg-primary/20";
  return "border-muted-foreground/40 bg-transparent";
}

/** 状态点读屏文案（点即状态，行内不再重复文字徽章） */
function stateLabel(row: ProgressRow): string {
  if (row.summary.status === "failed") return "生成失败";
  if (row.summary.status === "generating") return "生成中";
  if (row.summary.learningStatus === "completed") return "已完成";
  if (row.summary.learningStatus === "active") return "学习中";
  return "未开始";
}

/** 生成中/失败的计划锁定学习日编辑（不支持学习） */
function isLocked(row: ProgressRow): boolean {
  return row.summary.status === "generating" || row.summary.status === "failed";
}

function percent(row: ProgressRow): number {
  const { studyTotal, studyDone } = row.summary;
  return studyTotal === 0
    ? 0
    : Math.min(100, Math.round((studyDone / studyTotal) * 100));
}

function dayLabel(day: number): string {
  return WEEKDAY_LABELS[day - 1] ?? String(day);
}

function isSelected(row: ProgressRow, day: number): boolean {
  return row.days.includes(day);
}

function toggleStart(planId: string) {
  expandedStart.value = expandedStart.value === planId ? null : planId;
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
          学习日仅本地暂存，点「保存学习日 / 从今天开始」统一提交并重排日历
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
              ? "还没有已完成的计划"
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
            :key="row.summary.id"
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
                v-tip="`打开计划详情：${row.summary.title}`"
                :aria-label="`打开计划详情：${row.summary.title}，已学 ${row.summary.studyDone} 章，共排 ${row.summary.studyTotal} 章`"
                @click="emit('open-plan', row.summary.id)"
              >
                {{ row.summary.title }}
              </button>
              <button
                v-if="group.key === 'pending' && !isLocked(row)"
                type="button"
                class="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-md border border-primary/40 bg-primary/[0.06] px-2 py-1 text-xs font-medium text-primary transition-colors hover:bg-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :aria-expanded="expandedStart === row.summary.id"
                :aria-label="`开始学习：${row.summary.title}`"
                @click="toggleStart(row.summary.id)"
              >
                <Play class="h-3 w-3" aria-hidden="true" />
                开始学习
              </button>
              <span
                v-else-if="!isLocked(row) && row.summary.studyTotal > 0"
                class="shrink-0 text-xs tabular-nums text-muted-foreground"
              >
                已学 {{ row.summary.studyDone }}/{{ row.summary.studyTotal }} 章
              </span>
            </div>

            <!-- 学习进度条（打卡口径：studyDone/studyTotal；hairline 细条） -->
            <div
              v-if="
                row.summary.status !== 'generating' &&
                row.summary.status !== 'failed' &&
                row.summary.studyTotal > 0
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
                  row.summary.learningStatus === 'completed'
                    ? 'bg-success'
                    : 'bg-primary'
                "
                :style="{ width: `${percent(row)}%` }"
              />
            </div>

            <!-- 学习日 chips（学习中/已完成常显：点击仅本地暂存；
                 学习中 dirty 时行内出现保存确认；锁定计划不显示） -->
            <div
              v-if="group.key !== 'pending'"
              class="mt-2 flex flex-wrap items-center gap-1"
              role="group"
              :aria-label="`${row.summary.title} 每周学习日`"
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
                @click="toggleDraftDay(row.summary, day)"
              >
                {{ dayLabel(day) }}
              </button>
              <button
                v-if="group.key === 'active'"
                type="button"
                class="ml-auto rounded-md bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground transition-opacity duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                :class="
                  row.dirty ? 'opacity-100' : 'pointer-events-none opacity-0'
                "
                :aria-hidden="!row.dirty"
                :tabindex="row.dirty ? 0 : -1"
                :aria-label="`保存学习日：${row.summary.title}，今天之前与今天已完成不动`"
                :disabled="saving"
                @click="submitUpdate(row.summary)"
              >
                保存学习日
              </button>
            </div>

            <!-- 开始学习展开区：选学习日（本地暂存）→【从今天开始】一次性提交 -->
            <div
              v-if="
                group.key === 'pending' &&
                !isLocked(row) &&
                expandedStart === row.summary.id
              "
              class="mt-2 rounded-md border border-primary/20 bg-primary/[0.04] px-3 py-2.5"
            >
              <p class="text-xs text-muted-foreground">选择每周学习日</p>
              <div
                class="mt-1.5 flex flex-wrap gap-1"
                role="group"
                :aria-label="`${row.summary.title} 每周学习日`"
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
                  :disabled="saving"
                  @click="toggleDraftDay(row.summary, day)"
                >
                  {{ dayLabel(day) }}
                </button>
              </div>
              <button
                type="button"
                class="mt-2.5 flex w-full cursor-pointer items-center justify-center gap-1.5 rounded-md bg-primary px-2 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                :disabled="saving"
                :aria-label="`从今天开始：${row.summary.title}，按学习日排定日程（不打卡今天）`"
                @click="submitStart(row.summary)"
              >
                <CalendarCheck class="h-3.5 w-3.5" aria-hidden="true" />
                从今天开始
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 全空兜底（summaries 空时父组件不会渲染面板，此为防御） -->
      <p
        v-if="summaries.length === 0"
        class="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <SquareCheck class="h-4 w-4" aria-hidden="true" />
        暂无计划
      </p>
    </CardContent>
  </Card>
</template>
