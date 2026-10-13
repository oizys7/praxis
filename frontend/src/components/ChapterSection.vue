<script setup lang="ts">
import { computed, ref } from "vue";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import MermaidDiagram from "@/components/MermaidDiagram.vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  normalizeError,
  regenInFlight,
  regenerateChapter,
} from "@/composables/useAgent";
import { fetchPlanRecord } from "@/composables/usePlanHistory";
import type { Chapter, StudyPlan } from "@/types/agent";

/**
 * ChapterSection：渲染单个学习章节（Chapter）。
 * 三区：标题 / 摘要 → 主要知识（概念定义 / 知识点 / Mermaid 图示）→ 深入内容（默认收起）。
 * 头部按 chapter.status 渲染章节级徽标（undefined = 旧数据无徽标）；
 * failed/pending 且 planId 可用时提供「重新生成此章」入口。
 */
const props = defineProps<{
  chapter: Chapter;
  /** 历史计划 id：重生成入口依赖，缺省（新生成结果）不显示按钮 */
  planId?: string;
  /** 所属阶段索引（0-based，重生成命令参数） */
  phaseIndex?: number;
  /** 章节索引（0-based，重生成命令参数） */
  chapterIndex?: number;
}>();

/** 重生成完成：刷新后的计划整体上抛供调用方替换 */
const emit = defineEmits<{ refreshed: [plan: StudyPlan] }>();

/** 章节级徽标仅 failed/pending：complete 章节照常渲染不额外加标 */
const statusBadge = computed<{
  label: string;
  variant: "secondary" | "destructive";
} | null>(() => {
  switch (props.chapter.status) {
    case "failed":
      return { label: "✗ 失败", variant: "destructive" };
    case "pending":
      return { label: "◌ 未生成", variant: "secondary" };
    default:
      return null;
  }
});

/** 本章节重生成按钮：仅历史详情（有 planId）且目标处于 failed/pending 态时可用 */
const canRegenerate = computed(
  () =>
    !!props.planId &&
    (props.chapter.status === "failed" || props.chapter.status === "pending"),
);

/** 本组件是否正在重生成（按钮 spinner 与禁用；全局在飞锁另有 regenInFlight） */
const busy = ref(false);
/** 重生成过程的反馈信息（入库提示 / 失败原因；null = 无） */
const regenError = ref<string | null>(null);

/** 重生成此章：成败都刷新详情（失败也可见最新 failed 态），刷新后整体上抛 */
async function onRegenerate() {
  if (
    !props.planId ||
    busy.value ||
    regenInFlight.value ||
    props.phaseIndex === undefined ||
    props.chapterIndex === undefined
  ) {
    return;
  }
  busy.value = true;
  regenError.value = null;
  try {
    const envelope = await regenerateChapter(
      props.planId,
      props.phaseIndex,
      props.chapterIndex,
      (event) => {
        // 入库/收尾异常即时展示，不打断流程（PhaseDone{total=1} 不展示 1/1 计数）
        if (event.type === "storageIssue") regenError.value = event.message;
      },
    );
    const record = await fetchPlanRecord(props.planId);
    emit("refreshed", record.plan);
    if (envelope.status !== "plan" && envelope.error) {
      regenError.value = envelope.error.message;
    }
  } catch (e) {
    const err = normalizeError(e);
    try {
      // 传输错误同样刷新：Rust 收尾已把目标行翻 failed，详情可见最新状态
      emit("refreshed", (await fetchPlanRecord(props.planId)).plan);
    } catch {
      // 详情读取失败时仅展示错误，不打断原有渲染
    }
    regenError.value = err.message;
  } finally {
    busy.value = false;
  }
}

/** 是否有可展示的主要知识（三者皆空则整块隐藏） */
const hasMainKnowledge = computed(
  () =>
    props.chapter.mainKnowledge.concepts.length > 0 ||
    props.chapter.mainKnowledge.knowledgePoints.length > 0 ||
    props.chapter.mainKnowledge.diagrams.length > 0,
);
</script>

<template>
  <section class="rounded-lg border bg-card p-4">
    <!-- 章节标题 + 摘要；右侧章节级徽标与重生成入口（failed/pending 时可见） -->
    <header class="flex items-start justify-between gap-2">
      <div class="min-w-0">
        <h4 class="text-sm font-semibold">{{ props.chapter.title }}</h4>
        <p
          v-if="props.chapter.summary"
          class="mt-1 text-sm leading-relaxed text-muted-foreground"
        >
          {{ props.chapter.summary }}
        </p>
      </div>
      <div
        v-if="statusBadge || canRegenerate"
        class="flex shrink-0 items-center gap-2"
      >
        <Badge v-if="statusBadge" :variant="statusBadge.variant">
          {{ statusBadge.label }}
        </Badge>
        <Button
          v-if="canRegenerate"
          variant="outline"
          size="sm"
          :disabled="busy || regenInFlight"
          @click="onRegenerate"
        >
          <span
            v-if="busy"
            class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent"
          />
          {{ busy ? "重新生成中…" : "重新生成此章" }}
        </Button>
      </div>
    </header>
    <!-- 重生成反馈：入库/收尾异常与失败原因 -->
    <p v-if="regenError" class="mt-2 text-xs text-destructive">
      {{ regenError }}
    </p>

    <!-- 主要知识：概念 / 知识点 / 图示 -->
    <div v-if="hasMainKnowledge" class="mt-3 space-y-3">
      <div
        v-if="props.chapter.mainKnowledge.concepts.length"
        class="grid grid-cols-[88px,1fr] gap-x-3 gap-y-1.5 text-sm"
      >
        <span class="font-medium">📖 核心概念</span>
        <dl class="space-y-1.5">
          <div
            v-for="(c, i) in props.chapter.mainKnowledge.concepts"
            :key="i"
            class="flex gap-2"
          >
            <dt class="shrink-0 font-medium text-primary">{{ c.name }}</dt>
            <dd class="text-muted-foreground">{{ c.definition }}</dd>
          </div>
        </dl>
      </div>

      <div
        v-if="props.chapter.mainKnowledge.knowledgePoints.length"
        class="grid grid-cols-[88px,1fr] gap-x-3 gap-y-1 text-sm"
      >
        <span class="font-medium">💡 知识点</span>
        <ul class="space-y-1 text-muted-foreground">
          <li
            v-for="(k, i) in props.chapter.mainKnowledge.knowledgePoints"
            :key="i"
          >
            <span class="font-medium text-foreground">{{ k.title }}</span>
            <span v-if="k.detail">—— {{ k.detail }}</span>
          </li>
        </ul>
      </div>

      <div v-if="props.chapter.mainKnowledge.diagrams.length" class="space-y-3">
        <div v-for="(d, i) in props.chapter.mainKnowledge.diagrams" :key="i">
          <div
            v-if="d.title"
            class="mb-1 text-xs font-semibold text-muted-foreground"
          >
            📊 {{ d.title }}
          </div>
          <MermaidDiagram :code="d.mermaid" :caption="d.caption" />
        </div>
      </div>
    </div>

    <!-- 深入内容：每篇折叠面板，默认收起，标题 = topic + angle -->
    <div v-if="props.chapter.deepDives.length" class="mt-3 space-y-2">
      <div class="text-xs font-semibold text-muted-foreground">🔍 深入内容</div>
      <Collapsible
        v-for="(dd, i) in props.chapter.deepDives"
        :key="i"
        class="rounded-lg border"
      >
        <CollapsibleTrigger>
          <button
            type="button"
            class="flex w-full items-center justify-between gap-2 px-3 py-2 text-left transition-colors hover:bg-accent/50"
          >
            <span class="text-sm">
              <span class="font-medium">{{ dd.topic }}</span>
              <span v-if="dd.angle" class="text-muted-foreground">
                · {{ dd.angle }}</span
              >
            </span>
            <svg
              class="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 group-data-[state=open]:rotate-180"
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
        <CollapsibleContent>
          <p
            class="whitespace-pre-line border-t px-3 py-2.5 text-sm leading-relaxed text-muted-foreground"
          >
            {{ dd.content }}
          </p>
        </CollapsibleContent>
      </Collapsible>
    </div>
  </section>
</template>
