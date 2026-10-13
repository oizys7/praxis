<script setup lang="ts">
import { computed } from "vue";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import MermaidDiagram from "@/components/MermaidDiagram.vue";
import type { Chapter } from "@/types/agent";

/**
 * ChapterSection：渲染单个学习章节（Chapter）。
 * 三区：标题 / 摘要 → 主要知识（概念定义 / 知识点 / Mermaid 图示）→ 深入内容（默认收起）。
 */
const props = defineProps<{ chapter: Chapter }>();

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
    <!-- 章节标题 + 摘要 -->
    <header>
      <h4 class="text-sm font-semibold">{{ props.chapter.title }}</h4>
      <p
        v-if="props.chapter.summary"
        class="mt-1 text-sm leading-relaxed text-muted-foreground"
      >
        {{ props.chapter.summary }}
      </p>
    </header>

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
