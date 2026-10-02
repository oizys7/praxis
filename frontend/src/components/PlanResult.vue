<script setup lang="ts">
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import PhaseCard from '@/components/PhaseCard.vue'
import type { StudyPlan } from '@/types/agent'

/**
 * PlanResult：展示完整学习计划。
 * - 顶部：标题 / 概述 / 总学时
 * - 中部：阶段列表（PhaseCard 可折叠，第一阶段默认展开）
 * - 底部：推荐资源 + Agent 评审备注
 */
defineProps<{ plan: StudyPlan }>()

/** 资源类型 -> 展示标签 */
const resourceTypeLabels: Record<string, string> = {
  book: '书籍',
  course: '课程',
  docs: '文档',
  video: '视频',
  other: '其他',
}
</script>

<template>
  <div class="animate-fade-in-up space-y-4">
    <!-- 计划概览 -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-start justify-between gap-2">
          <CardTitle class="text-lg">{{ plan.title }}</CardTitle>
          <Badge v-if="plan.estimatedTotalHours" variant="secondary">
            预计总学时 ≈ {{ plan.estimatedTotalHours }}h
          </Badge>
        </div>
        <CardDescription class="leading-relaxed">{{ plan.overview }}</CardDescription>
      </CardHeader>
    </Card>

    <!-- 阶段列表 -->
    <div class="space-y-3">
      <PhaseCard
        v-for="(phase, i) in plan.phases"
        :key="i"
        :phase="phase"
        :index="i"
        :default-open="i === 0"
      />
    </div>

    <!-- 推荐资源 -->
    <Card v-if="plan.resources.length">
      <CardHeader>
        <CardTitle class="text-base">📖 推荐资源</CardTitle>
      </CardHeader>
      <CardContent>
        <ul class="space-y-2">
          <li v-for="(res, i) in plan.resources" :key="i" class="flex items-start gap-2.5 text-sm">
            <Badge variant="outline" class="mt-0.5 shrink-0">
              {{ resourceTypeLabels[res.type] ?? res.type }}
            </Badge>
            <div>
              <span class="font-medium">{{ res.title }}</span>
              <span v-if="res.note" class="text-muted-foreground"> —— {{ res.note }}</span>
            </div>
          </li>
        </ul>
      </CardContent>
    </Card>

    <!-- Agent 评审备注 -->
    <Card v-if="plan.reviewNotes" class="border-dashed">
      <CardContent class="pt-6">
        <div class="text-xs font-semibold text-muted-foreground">🔍 Agent 评审备注</div>
        <p class="mt-1.5 whitespace-pre-line text-sm text-muted-foreground">
          {{ plan.reviewNotes }}
        </p>
      </CardContent>
    </Card>
  </div>
</template>
