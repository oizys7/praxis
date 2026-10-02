<script setup lang="ts">
import { computed } from "vue";
import { Card, CardContent } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import type { StudyPhase } from "@/types/agent";

/**
 * PhaseCard：单个学习阶段的可折叠卡片。
 * 展开后显示本阶段的目标、理论 / 实操 / 作业三块内容，以及收尾项目与验收标准。
 */
const props = defineProps<{
  /** 阶段序号（用于展示编号） */
  index: number;
  phase: StudyPhase;
  /** 是否默认展开（首个阶段默认展开） */
  defaultOpen?: boolean;
}>();

/** 理论 / 实操 / 作业 三个列表区块的元信息 */
const sections = computed(() => [
  { key: "theory", icon: "📚", title: "理论学习", items: props.phase.theory },
  {
    key: "practice",
    icon: "🛠️",
    title: "动手实操",
    items: props.phase.practice,
  },
  {
    key: "assignments",
    icon: "✏️",
    title: "课后作业",
    items: props.phase.assignments,
  },
]);
</script>

<template>
  <Collapsible :default-open="props.defaultOpen">
    <Card class="overflow-hidden">
      <!-- 折叠触发器：整行可点击 -->
      <CollapsibleTrigger>
        <button
          type="button"
          class="flex w-full items-center justify-between gap-3 p-4 text-left transition-colors hover:bg-accent/50"
        >
          <div class="flex items-center gap-3">
            <span
              class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/10 text-sm font-bold text-primary"
            >
              {{ index + 1 }}
            </span>
            <div>
              <div class="font-semibold leading-tight">{{ phase.name }}</div>
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

          <!-- 理论 / 实操 / 作业 -->
          <div
            v-for="section in sections.filter((s) => s.items.length)"
            :key="section.key"
            class="grid grid-cols-[88px,1fr] gap-x-3 gap-y-1 text-sm"
          >
            <span class="font-medium"
              >{{ section.icon }} {{ section.title }}</span
            >
            <ul class="space-y-1 text-muted-foreground">
              <li v-for="(item, i) in section.items" :key="i">· {{ item }}</li>
            </ul>
          </div>

          <!-- 阶段项目 + 验收标准 -->
          <div
            v-if="phase.project.name"
            class="rounded-lg border bg-primary/5 p-3"
          >
            <div class="flex items-center gap-2">
              <span>🚀</span>
              <span class="text-sm font-semibold"
                >阶段项目：{{ phase.project.name }}</span
              >
            </div>
            <p class="mt-1.5 text-sm text-muted-foreground">
              {{ phase.project.description }}
            </p>
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
