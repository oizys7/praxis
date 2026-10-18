<script setup lang="ts">
import { computed, inject, ref, watch, type ShallowRef } from "vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import ChapterSection from "@/components/ChapterSection.vue";
import {
  isRegenInFlight,
  normalizeError,
  regeneratePhase,
} from "@/composables/useAgent";
import { fetchPlanRecord } from "@/composables/usePlanHistory";
import { noteRegenDone } from "@/composables/useGenerationProgress";
import type { StudyPhase, StudyPlan } from "@/types/agent";

/**
 * PhaseCard：单个学习阶段的可折叠卡片。
 * 展开后按章节列表渲染（ChapterSection，含 Mermaid 图示与深入内容），
 * 另有阶段目标 / 收尾项目（含背景、步骤、预计学时）。
 * 头部按 phase.status 渲染两级状态徽标（undefined = 旧数据无徽标）；
 * failed/pending 且 planId 可用时提供「重新生成此阶段」入口。
 */
const props = defineProps<{
  /** 阶段序号（用于展示编号） */
  index: number;
  phase: StudyPhase;
  /** 是否默认展开（首个阶段默认展开） */
  defaultOpen?: boolean;
  /** 历史计划 id：重生成入口依赖，缺省（新生成结果）不显示按钮 */
  planId?: string;
  /** 行级生成状态（generating/failed/complete）：驱动「生成中」徽章与重生成抑制 */
  rowStatus?: string;
}>();

/** 子组件（Section）重生成完成：刷新后的计划整体上抛供调用方替换 */
const emit = defineEmits<{ refreshed: [plan: StudyPlan] }>();

/** 阶段状态徽标（undefined = 旧数据/缺省流式载荷，不展示） */
const statusBadge = computed<{
  label: string;
  variant: "success" | "secondary" | "destructive";
} | null>(() => {
  switch (props.phase.status) {
    case "complete":
      return { label: "✓ 已完成", variant: "success" };
    case "failed":
      return { label: "✗ 失败", variant: "destructive" };
    case "pending":
      // 行还在 generating 时 pending ≠ 未开始，而是「轮到/正在生成」（反馈③区分）
      return props.rowStatus === "generating"
        ? { label: "⟳ 生成中", variant: "secondary" }
        : { label: "◌ 未生成", variant: "secondary" };
    default:
      return null;
  }
});

/** 本阶段重生成按钮：仅历史详情（有 planId）且目标处于 failed/pending 态
 * 时可用；行仍在 generating 时不开放（与后台生成竞争） */
const canRegenerate = computed(
  () =>
    !!props.planId &&
    props.rowStatus !== "generating" &&
    (props.phase.status === "failed" || props.phase.status === "pending"),
);

/** 阶段折叠态受控（目录可驱动正文展开，反馈①）：初值取 default-open，
 * 目录命令信箱（PlanWorkbench 悬浮目录 → provide）下发阶段/章节命令时
 * 展开自身；用户在正文折叠后目录点阶段即重新展开——目录与正文互为镜像。 */
const open = ref(props.defaultOpen ?? true);
const tocCommands = inject<ShallowRef<{
  phase?: number;
  chapter?: { p: number; c: number };
  seq: number;
} | null> | null>("planTocCommands", null);
watch(
  () => tocCommands?.value,
  (cmd) => {
    if (cmd?.phase === props.index) open.value = true;
  },
);

/** 本组件自身是否正在重生成（spinner；在飞锁另有模块级 regenScopes） */
const busy = ref(false);
/** 本计划任意阶段/章节的重生成是否在飞（含非本 scope；按钮禁用与文案判据） */
const regenBusy = computed(
  () => busy.value || (!!props.planId && isRegenInFlight(props.planId)),
);
/** 重生成过程的反馈信息（入库提示 / 失败原因；null = 无） */
const regenError = ref<string | null>(null);

/** 重生成此阶段：成败都刷新详情（失败也可见最新 failed 态），刷新后整体上抛 */
async function onRegenerate() {
  if (!props.planId || regenBusy.value) return;
  busy.value = true;
  regenError.value = null;
  try {
    // 海量细节由后端编排；前端只透传进度事件中的入库/收尾异常（storageIssue）
    const envelope = await regeneratePhase(
      props.planId,
      props.index,
      (event) => {
        if (event.type === "storageIssue") regenError.value = event.message;
      },
    );
    const record = await fetchPlanRecord(props.planId);
    // 落袋无论组件死活照走：页面已切换时 emit no-op，重进后 PlanResult 从袋里补捞
    noteRegenDone(props.planId, record.plan);
    emit("refreshed", record.plan);
    if (envelope.status !== "plan" && envelope.error) {
      regenError.value = envelope.error.message;
    }
  } catch (e) {
    const err = normalizeError(e);
    try {
      // 传输错误同样刷新：Rust 收尾已把目标行翻 failed，详情可见最新状态
      const refreshed = (await fetchPlanRecord(props.planId)).plan;
      noteRegenDone(props.planId, refreshed);
      emit("refreshed", refreshed);
    } catch {
      // 详情读取失败时仅展示错误，不打断原有渲染
    }
    regenError.value = err.message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <!-- 折叠受控开态（目录命令可驱动展开，反馈①） -->
  <Collapsible :open="open" @update:open="(v) => (open = v)">
    <Card class="overflow-hidden">
      <!-- 头部：折叠触发器（标题/箭头，主体行可点击）+ 右侧状态徽标与重生成按钮 -->
      <div class="flex items-center gap-2">
        <CollapsibleTrigger class="min-w-0 flex-1">
          <button
            type="button"
            class="flex w-full items-center justify-between gap-3 p-4 text-left transition-colors hover:bg-accent/50"
          >
            <div class="flex min-w-0 items-center gap-3">
              <span
                class="font-display flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-sm font-semibold text-primary"
              >
                {{ index + 1 }}
              </span>
              <div class="min-w-0">
                <div
                  class="font-display truncate text-[15px] font-semibold leading-tight"
                >
                  {{ phase.name }}
                </div>
                <div
                  v-if="phase.weekRange"
                  class="mt-0.5 text-xs text-muted-foreground"
                >
                  {{ phase.weekRange }}
                </div>
              </div>
            </div>
            <!-- 展开状态指示箭头：group-data-[state=open] 由 Collapsible 根元素驱动 -->
            <svg
              class="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200 group-data-[state=open]:rotate-180"
              aria-hidden="true"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="m6 9 6 6 6-6" />
            </svg>
          </button>
        </CollapsibleTrigger>
        <!-- 状态徽标（旧数据无 status 时不展示）+ 重生成入口（独立于触发器，不套套） -->
        <div
          v-if="statusBadge || canRegenerate"
          class="flex shrink-0 items-center gap-2 pr-4"
        >
          <Badge v-if="statusBadge" :variant="statusBadge.variant">
            {{ statusBadge.label }}
          </Badge>
          <Button
            v-if="canRegenerate"
            variant="outline"
            size="sm"
            :disabled="!props.planId || regenBusy"
            @click="onRegenerate"
          >
            <span
              v-if="busy"
              class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent"
            />
            {{ busy ? "重新生成中…" : "重新生成此阶段" }}
          </Button>
        </div>
      </div>
      <!-- 重生成反馈：入库/收尾异常与失败原因，头部下方可见（不展开也能看到） -->
      <p v-if="regenError" class="px-4 pb-2 text-xs text-destructive">
        {{ regenError }}
      </p>

      <CollapsibleContent>
        <CardContent class="space-y-4 border-t pt-4">
          <!-- 阶段目标 -->
          <div v-if="phase.objectives.length">
            <h4 class="mb-1.5 text-sm font-semibold">🎯 阶段目标</h4>
            <ul class="space-y-1 text-sm text-muted-foreground">
              <li
                v-for="(item, i) in phase.objectives"
                :key="i"
                class="flex gap-2"
              >
                <span class="text-primary">·</span>{{ item }}
              </li>
            </ul>
          </div>

          <!-- 章节化内容（新数据）：每章一个 ChapterSection（携带重生成入口参数） -->
          <div v-if="phase.chapters?.length" class="space-y-3">
            <!-- 目录锚点包装层：悬浮两级目录（PlanResult）滚动定位至此 -->
            <div
              v-for="(chapter, ci) in phase.chapters"
              :id="`plan-phase-${index}-chapter-${ci}`"
              :key="ci"
              class="scroll-mt-24"
            >
              <ChapterSection
                :chapter="chapter"
                :plan-id="planId"
                :phase-index="index"
                :chapter-index="ci"
                :row-status="props.rowStatus"
                @refreshed="(p) => emit('refreshed', p)"
              />
            </div>
          </div>

          <!-- 阶段项目 + 验收标准 -->
          <div
            v-if="phase.project.name"
            class="rounded-lg border bg-primary/5 p-3"
          >
            <div class="flex flex-wrap items-center gap-2">
              <span>🚀</span>
              <span class="text-sm font-semibold"
                >阶段项目：{{ phase.project.name }}</span
              >
              <Badge
                v-if="phase.project.estimatedHours"
                variant="secondary"
                class="ml-auto"
              >
                预计投入 ≈ {{ phase.project.estimatedHours }}h
              </Badge>
            </div>
            <p class="mt-1.5 text-sm text-muted-foreground">
              {{ phase.project.description }}
            </p>
            <!-- 项目背景与动机（旧数据无此字段） -->
            <p
              v-if="phase.project.background"
              class="mt-1.5 text-sm text-muted-foreground"
            >
              <span class="font-medium text-foreground">背景：</span
              >{{ phase.project.background }}
            </p>
            <!-- 建议实施步骤（有序，旧数据无此字段） -->
            <div v-if="phase.project.steps?.length" class="mt-2">
              <div class="text-xs font-semibold text-muted-foreground">
                建议实施步骤
              </div>
              <ol class="mt-1 space-y-1 text-sm">
                <li
                  v-for="(step, si) in phase.project.steps"
                  :key="si"
                  class="flex gap-2 text-muted-foreground"
                >
                  <span class="shrink-0 font-medium text-primary"
                    >{{ si + 1 }}.</span
                  >{{ step }}
                </li>
              </ol>
            </div>
            <div v-if="phase.project.acceptanceCriteria.length" class="mt-2">
              <div class="text-xs font-semibold text-muted-foreground">
                验收标准
              </div>
              <ul class="mt-1 space-y-1 text-sm">
                <li
                  v-for="(c, i) in phase.project.acceptanceCriteria"
                  :key="i"
                  class="flex gap-2 text-muted-foreground"
                >
                  <span class="text-emerald-500">✓</span>{{ c }}
                </li>
              </ul>
            </div>
          </div>
        </CardContent>
      </CollapsibleContent>
    </Card>
  </Collapsible>
</template>
