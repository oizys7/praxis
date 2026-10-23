<script setup lang="ts">
import { computed } from "vue";
import { Badge } from "@/components/ui/badge";
import type { Exercise } from "@/types/agent";

/**
 * 结构化自测题面板：把「知识点 → 题目」的对应关系显式摆出来。
 *
 * 与正文里的「自检清单」小节是同一批题的两种形态——正文给人读，这里给程序
 * 和读者双向核对：每道题考的是哪个知识点、有没有知识点一道题都没考到。
 * 刻意不做「勾选完成」的状态：那属于学习闭环，需要落库（打卡记录），
 * 没有存储的勾选只会制造「我学完了」的假象。
 */
const props = defineProps<{
  exercises: Exercise[];
}>();

const KIND_LABELS: Record<string, string> = {
  recall: "回忆",
  scenario: "情景判断",
};

const items = computed(() => props.exercises ?? []);
</script>

<template>
  <section v-if="items.length" class="rounded-lg border bg-card p-3">
    <h5
      class="mb-2 flex items-baseline gap-2 text-xs font-semibold text-foreground"
    >
      自测
      <span class="text-[11px] font-normal text-muted-foreground">
        共 {{ items.length }} 题，先自己答一遍再往下读
      </span>
    </h5>
    <ol class="space-y-2">
      <li
        v-for="(item, i) in items"
        :key="i"
        class="flex gap-2 text-sm leading-relaxed"
      >
        <span class="shrink-0 tabular-nums text-muted-foreground">
          {{ i + 1 }}.
        </span>
        <div class="min-w-0">
          <p class="text-foreground/90">{{ item.question }}</p>
          <div class="mt-1 flex flex-wrap items-center gap-1">
            <Badge
              :variant="item.kind === 'scenario' ? 'default' : 'secondary'"
              class="text-[10px]"
            >
              {{ KIND_LABELS[item.kind] ?? item.kind }}
            </Badge>
            <Badge
              v-for="target in item.targets"
              :key="target"
              variant="outline"
              class="text-[10px] font-normal"
            >
              {{ target }}
            </Badge>
          </div>
        </div>
      </li>
    </ol>
  </section>
</template>
