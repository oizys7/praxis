<script setup lang="ts">
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * ClarificationPanel：Agent 判定输入信息不完整时，
 * 展示 clarify 节点返回的澄清问题列表，提示用户补充后再试。
 */
defineProps<{ questions: string[] }>();
</script>

<template>
  <!-- 语义归拢：澄清不是警示（amber 双硬编码弃用），而是主流程的补充邀约，用墨蓝 -->
  <Card class="border-primary/30 bg-primary/5">
    <CardHeader>
      <CardTitle class="flex items-center gap-2 text-base">
        <span>🤔</span>
        Agent 需要更多信息
      </CardTitle>
    </CardHeader>
    <CardContent class="space-y-3">
      <p class="text-sm text-muted-foreground">
        请补充以下信息后，修改左侧表单重新生成：
      </p>
      <ol class="space-y-2">
        <li
          v-for="(q, i) in questions"
          :key="i"
          class="flex gap-2.5 rounded-md border bg-background p-3 text-sm"
        >
          <span
            class="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-amber-500/15 text-xs font-bold text-amber-600"
          >
            {{ i + 1 }}
          </span>
          <span class="leading-relaxed">{{ q }}</span>
        </li>
      </ol>
    </CardContent>
  </Card>
</template>
