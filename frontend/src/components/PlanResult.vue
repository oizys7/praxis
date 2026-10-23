<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import PhaseCard from "@/components/PhaseCard.vue";
import { consumeRegenDone } from "@/composables/useGenerationProgress";
import type { StudyPlan } from "@/types/agent";

/**
 * PlanResult：展示完整学习计划。
 * - 顶部：标题 / 概述 / 总学时 + 云端自动保存状态徽标（生成成功即由 Rust 自动入库，
 *   无手动保存；storageIssue 非空时降级提示）
 * - 中部：阶段列表（PhaseCard 可折叠，第一阶段默认展开）
 * - 底部：推荐资源 + Agent 评审备注
 */
/** 响应式 props 引用：planId 变化监听（重生成落袋结果补捞）需要 */
const props = defineProps<{
  plan: StudyPlan;
  /** 生成时的学习目标（历史列表检索字段；历史查看时不传） */
  goal?: string;
  /** 生成时的水平（列表展示用） */
  level?: string;
  /** 生成时的周数 */
  weeks?: number;
  /** 历史查看模式：计划已入云，不显示保存状态徽标 */
  historyMode?: boolean;
  /** 云端自动入库异常提示（任务 23 起源自进度观测 store / App 终态快照；null = 入库正常） */
  storageIssue?: string | null;
  /**
   * 历史计划 id：阶段/章节级重生成入口依赖。新生成结果（Rust 自动入库
   * 的 id 未回传前端）不传，重生成按钮不渲染，失败部分引导走历史详情。
   */
  planId?: string;
  /**
   * 行级生成状态（get_plan 回读的 PlanRecord.status 平铺字段，非
   * record.plan 内容）：generating → 「生成进行中」提示条；failed →
   * 「生成未完成」引导重生成提示条；其余/缺省不加标。
   */
  rowStatus?: string;
}>();

/** 子组件（PhaseCard → ChapterSection）重生成完成：刷新后的计划整体上抛供调用方替换 */
const emit = defineEmits<{ refreshed: [plan: StudyPlan] }>();

/** 章节总数（反馈③：hero 元信息行展示「共 N 阶段 / M 章节」） */
/** 全局重生成互斥（2026-10-16 反馈③）：任一阶段/章节 DB 状态为 generating
 * （含退出重进后从库读回的持久态）→ 全站重生成入口禁用置灰 */
const regenLocked = computed(() =>
  props.plan.phases.some(
    (p) =>
      p.status === "generating" ||
      p.chapters?.some((c) => c.status === "generating"),
  ),
);

const chapterTotal = computed(() =>
  props.plan.phases.reduce((n, p) => n + (p.chapters?.length ?? 0), 0),
);

/**
 * 问题②消费点：PhaseCard/ChapterSection 完成重生成时若本页已切换，emit
 * 静默丢失（目标组件已卸载）；结果已由完成路径落袋到模块级
 * regenResults（useGenerationProgress）。本组件挂载或 planId 变化时
 * 取袋上抛，App.onPlanRefreshed 就地换 result，状态不再停留在「未生成」。
 */
function consumePendingRegen() {
  if (!props.planId) return;
  const plan = consumeRegenDone(props.planId);
  if (plan) emit("refreshed", plan);
}
onMounted(consumePendingRegen);
watch(
  () => props.planId,
  () => consumePendingRegen(),
);

/** 资源类型 -> 展示标签 */
const resourceTypeLabels: Record<string, string> = {
  book: "书籍",
  course: "课程",
  docs: "文档",
  video: "视频",
  other: "其他",
};
</script>

<template>
  <!-- 动效收敛：整根不再逐卡满 fade-in（AI 页面感通病），
       唯一非用户触发动效留给 hero 标题一次性入场 -->
  <div class="space-y-4">
    <!--
      行级状态兜底提示条（任务 25）：数据源是 get_plan 回读的 PlanRecord.status
      （rowStatus，wire 平铺字段与 record.plan 平级行），数据路径可达——
      failed 引导重生成失败部分；generating 说明见到的是当前已生成部分。
    -->
    <div
      v-if="rowStatus === 'generating'"
      class="flex items-center gap-2 rounded-md border bg-secondary/50 px-3 py-2 text-sm text-secondary-foreground"
    >
      <span
        class="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent"
      />
      生成进行中：以下为当前已生成的部分，完成后列表会自动刷新，重新进入可看到全文。
    </div>
    <!--
      审查 Finding-1：failed 行的失败阶段/章节经重生成全部补齐后，行级
      status 仍停留 failed（Rust 侧 regen 收口不强推 complete），提示条
      须叠加内容判据避免「已无可失败标记却仍引导重生成」的自相矛盾；
      onPlanRefreshed 用刷新后的 plan 实时替换 result，补齐后提示条自动消失。
    -->
    <div
      v-else-if="
        rowStatus === 'failed' &&
        plan.phases.some((p) => p.status === 'failed' || p.status === 'pending')
      "
      class="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      ⚠
      该计划生成未完成，可对下方标记为失败的部分点「重新生成」补齐，或稍后在历史记录中重试。
    </div>

    <!-- 计划概览（反馈③整卡一体化：渐变 hero + 元信息行 + 概要同层，
         与下方目录/阶段卡共用一套节奏） -->
    <Card class="overflow-hidden">
      <!-- 音书页头：沙色纸带取代渐变洗染；标题 serif（书页隐喻），
           唯一编排式入场动效挂在这行字上 -->
      <div class="border-b bg-secondary/40 p-6">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <h1
            class="animate-fade-in-up font-display text-3xl font-semibold tracking-tight"
          >
            {{ plan.title }}
          </h1>
          <div class="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">
              共 {{ plan.phases.length }} 阶段 / {{ chapterTotal }} 章节
            </Badge>
            <Badge v-if="plan.estimatedTotalHours" variant="secondary">
              预计总学时 ≈ {{ plan.estimatedTotalHours }}h
            </Badge>
          </div>
        </div>
        <!-- 概要段落：空串（旧数据未写 overview）隐藏整行 -->
        <p
          v-if="plan.overview"
          class="mt-3 leading-relaxed text-muted-foreground"
        >
          {{ plan.overview }}
        </p>
        <!-- 云端自动保存状态（历史查看模式下隐藏：计划本就已入库） -->
        <div
          v-if="!historyMode"
          class="mt-3 flex flex-wrap items-center gap-2 border-t pt-3"
        >
          <template v-if="storageIssue">
            <Badge variant="destructive">⚠ 云端未保存</Badge>
            <span class="text-xs text-muted-foreground">{{
              storageIssue
            }}</span>
          </template>
          <template v-else>
            <Badge variant="success">✓ 已自动保存到云端</Badge>
          </template>
        </div>
      </div>
    </Card>

    <!-- 阶段列表（锚点包装：目录跳定位；scroll-mt 抵销 sticky 双层高度） -->
    <div class="space-y-4">
      <div
        v-for="(phase, i) in plan.phases"
        :id="`plan-phase-${i}`"
        :key="i"
        class="scroll-mt-28"
      >
        <PhaseCard
          :phase="phase"
          :index="i"
          :plan-id="planId"
          :row-status="rowStatus"
          :regen-locked="regenLocked"
          :default-open="i === 0"
          @refreshed="(p) => emit('refreshed', p)"
        />
      </div>
    </div>

    <!-- 术语表（只读）：由各章核心术语去重汇总，随计划重新生成而变 -->
    <Card v-if="plan.glossary?.length">
      <CardHeader>
        <CardTitle class="text-base">🔤 术语表</CardTitle>
      </CardHeader>
      <CardContent>
        <!-- 术语与出处分两行：同一行 + truncate 时，长术语会把出处挤没、
             长出处会被截断，两列布局下几乎必然看不全 -->
        <dl class="grid gap-x-8 gap-y-3 sm:grid-cols-2">
          <div
            v-for="entry in plan.glossary"
            :key="entry.term"
            class="min-w-0 text-sm"
          >
            <dt class="break-words font-medium">{{ entry.term }}</dt>
            <dd class="mt-0.5 break-words text-xs text-muted-foreground">
              —— {{ entry.chapterTitle }}
            </dd>
          </div>
        </dl>
      </CardContent>
    </Card>

    <!-- 推荐资源 -->
    <Card v-if="plan.resources.length">
      <CardHeader>
        <CardTitle class="flex flex-wrap items-baseline gap-x-2 text-base">
          📖 推荐资源
          <span class="text-xs font-normal text-muted-foreground">
            未经核实，版本与适用性请自行判断
          </span>
        </CardTitle>
      </CardHeader>
      <CardContent>
        <ul class="space-y-2">
          <li
            v-for="(res, i) in plan.resources"
            :key="i"
            class="flex items-start gap-2.5 text-sm"
          >
            <Badge variant="outline" class="mt-0.5 shrink-0">
              {{ resourceTypeLabels[res.type] ?? res.type }}
            </Badge>
            <div class="min-w-0">
              <a
                v-if="res.url"
                :href="res.url"
                target="_blank"
                rel="noopener noreferrer"
                class="font-medium underline decoration-border underline-offset-4 transition-colors hover:text-primary"
              >
                {{ res.title }}
              </a>
              <span v-else class="font-medium">{{ res.title }}</span>
              <!-- 两档是客观事实（真探测过 / 没探测过），不是模型自评的可信度 -->
              <span
                v-if="res.verification === 'reachable'"
                class="ml-2 align-middle text-xs text-success"
              >
                ✓ 已核实可达
              </span>
              <span
                v-else
                class="ml-2 align-middle text-xs text-muted-foreground"
              >
                未经核实
              </span>
              <span v-if="res.note" class="text-muted-foreground">
                —— {{ res.note }}</span
              >
            </div>
          </li>
        </ul>
      </CardContent>
    </Card>

    <!-- Agent 评审备注 -->
    <Card v-if="plan.reviewNotes" class="border-dashed">
      <CardContent class="pt-6">
        <div class="text-xs font-semibold text-muted-foreground">
          🔍 Agent 评审备注
        </div>
        <p class="mt-1.5 whitespace-pre-line text-sm text-muted-foreground">
          {{ plan.reviewNotes }}
        </p>
      </CardContent>
    </Card>
  </div>
</template>
