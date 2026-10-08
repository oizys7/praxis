<script setup lang="ts">
import { ref } from "vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import PhaseCard from "@/components/PhaseCard.vue";
import { savePlan } from "@/composables/usePlanHistory";
import type { CommandError, StudyPlan } from "@/types/agent";

/**
 * PlanResult：展示完整学习计划。
 * - 顶部：标题 / 概述 / 总学时 + 保存到云端
 * - 中部：阶段列表（PhaseCard 可折叠，第一阶段默认展开）
 * - 底部：推荐资源 + Agent 评审备注
 */
const props = defineProps<{
  plan: StudyPlan;
  /** 生成时的学习目标（保存到云端的检索字段；历史查看时不传） */
  goal?: string;
  /** 生成时的水平（列表展示用） */
  level?: string;
  /** 生成时的周数 */
  weeks?: number;
  /** 历史查看模式：计划已入云，不显示保存按钮 */
  historyMode?: boolean;
}>();

/** 资源类型 -> 展示标签 */
const resourceTypeLabels: Record<string, string> = {
  book: "书籍",
  course: "课程",
  docs: "文档",
  video: "视频",
  other: "其他",
};

/** 保存到云端的状态 */
const saving = ref(false);
const savedAt = ref<string | null>(null);
const saveError = ref<CommandError | null>(null);

async function onSave() {
  saving.value = true;
  saveError.value = null;
  try {
    await savePlan({
      title: props.plan.title || props.goal || "未命名计划",
      goal: props.goal ?? "",
      overview: props.plan.overview,
      level: props.level ?? "",
      weeks: props.weeks ?? 0,
      plan: props.plan,
    });
    savedAt.value = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  } catch (e) {
    saveError.value = e as CommandError; // usePlanHistory 已归一
  } finally {
    saving.value = false;
  }
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
        <CardDescription class="leading-relaxed">{{
          plan.overview
        }}</CardDescription>
        <!-- 保存到云端（历史查看模式下隐藏：计划已入库） -->
        <div
          v-if="!historyMode"
          class="mt-3 flex flex-wrap items-center gap-3 border-t pt-3"
        >
          <template v-if="!savedAt">
            <Button
              size="sm"
              variant="outline"
              :disabled="saving"
              @click="onSave"
            >
              <span
                v-if="saving"
                class="mr-2 h-3 w-3 animate-spin rounded-full border-2 border-primary border-t-transparent"
              />
              💾 保存到云端
            </Button>
            <span v-if="saveError" class="text-xs text-destructive">
              {{ saveError.message }}
            </span>
          </template>
          <template v-else>
            <Badge variant="success">✓ 已保存到云端（{{ savedAt }}）</Badge>
            <Button size="sm" variant="ghost" @click="savedAt = null">
              再存一份
            </Button>
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
