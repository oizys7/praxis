<script setup lang="ts">
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import PhaseCard from "@/components/PhaseCard.vue";
import type { StudyPlan } from "@/types/agent";

/**
 * PlanResult：展示完整学习计划。
 * - 顶部：标题 / 概述 / 总学时 + 云端自动保存状态徽标（生成成功即由 Rust 自动入库，
 *   无手动保存；storageIssue 非空时降级提示）
 * - 中部：阶段列表（PhaseCard 可折叠，第一阶段默认展开）
 * - 底部：推荐资源 + Agent 评审备注
 */
defineProps<{
  plan: StudyPlan;
  /** 生成时的学习目标（历史列表检索字段；历史查看时不传） */
  goal?: string;
  /** 生成时的水平（列表展示用） */
  level?: string;
  /** 生成时的周数 */
  weeks?: number;
  /** 历史查看模式：计划已入云，不显示保存状态徽标 */
  historyMode?: boolean;
  /** 云端自动入库异常提示（useAgent 的 storageIssue；null = 入库正常） */
  storageIssue?: string | null;
}>();

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
        <CardDescription class="leading-relaxed">{{
          plan.overview
        }}</CardDescription>
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
          <li
            v-for="(res, i) in plan.resources"
            :key="i"
            class="flex items-start gap-2.5 text-sm"
          >
            <Badge variant="outline" class="mt-0.5 shrink-0">
              {{ resourceTypeLabels[res.type] ?? res.type }}
            </Badge>
            <div>
              <span class="font-medium">{{ res.title }}</span>
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
