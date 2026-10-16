<script setup lang="ts">
import { computed, reactive } from "vue";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  INTEREST_PRESETS,
  LEVEL_OPTIONS,
  type Level,
  type PlanRequest,
} from "@/types/agent";

/**
 * PlanForm：学习需求输入表单。
 * 字段：学习目标 / 当前水平 / 每周可用小时 / 期望周数 / 偏好方向（多选标签）/ 已有基础。
 * 校验通过后向上抛出 submit 事件，由 App.vue 调用 useAgent 发起请求。
 */
const props = defineProps<{ loading?: boolean }>();
const emit = defineEmits<{ (e: "submit", payload: PlanRequest): void }>();

/** 表单数据（与 PlanRequest 字段一一对应） */
const form = reactive({
  goal: "",
  level: "beginner" as Level,
  weeklyHours: 10,
  weeks: 12,
  interests: [] as string[],
  background: "",
  // 每周学习日（1=周一..7=周日，需求 5）：默认全选
  studyDays: [1, 2, 3, 4, 5, 6, 7] as number[],
});

/** 学习日 chip 标签（列序即 1..7，周一起始） */
const STUDY_DAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;

/** 行内校验错误 */
const errors = reactive({ goal: "", weeklyHours: "", weeks: "" });

/** 偏好方向标签是否可提交（仅用于按钮禁用态展示） */
const canSubmit = computed(() => !props.loading);

/** 切换偏好方向标签的选中状态 */
function toggleInterest(tag: string) {
  const idx = form.interests.indexOf(tag);
  if (idx >= 0) {
    form.interests.splice(idx, 1);
  } else {
    form.interests.push(tag);
  }
}

/** 切换每周学习日（至少保留一天；取消最后一天时静默拒绝） */
function toggleStudyDay(day: number) {
  const idx = form.studyDays.indexOf(day);
  if (idx >= 0) {
    if (form.studyDays.length === 1) return;
    form.studyDays.splice(idx, 1);
  } else {
    form.studyDays.push(day);
    form.studyDays.sort((a, b) => a - b);
  }
}

/** 行内校验：目标长度 / 小时数 / 周数 */
function validate(): boolean {
  errors.goal =
    form.goal.trim().length < 4 ? "请至少用 4 个字描述你的学习目标" : "";
  const hours = Number(form.weeklyHours);
  errors.weeklyHours =
    !Number.isFinite(hours) || hours < 1 || hours > 80
      ? "请填写 1-80 之间的数字"
      : "";
  const weeks = Number(form.weeks);
  errors.weeks =
    !Number.isFinite(weeks) || weeks < 1 || weeks > 52
      ? "请填写 1-52 之间的整数"
      : "";
  return !errors.goal && !errors.weeklyHours && !errors.weeks;
}

/** 提交：校验通过后组装 PlanRequest 抛给父组件 */
function onSubmit() {
  if (props.loading || !validate()) return;
  emit("submit", {
    goal: form.goal.trim(),
    level: form.level,
    weeklyHours: Math.round(Number(form.weeklyHours)),
    weeks: Math.round(Number(form.weeks)),
    interests: [...form.interests],
    background: form.background.trim(),
    studyDays: [...form.studyDays],
  });
}
</script>

<template>
  <Card class="h-fit lg:sticky lg:top-24">
    <CardHeader>
      <CardTitle>制定我的学习计划</CardTitle>
      <CardDescription>
        填写学习需求，Agent 将生成"理论 + 实操 + 作业 +
        项目"四位一体的分阶段计划。
      </CardDescription>
    </CardHeader>

    <CardContent class="space-y-5">
      <!-- 组题：目标与起点 -->
      <div class="flex items-center gap-3">
        <span class="text-xs font-medium text-muted-foreground"
          >目标与起点</span
        >
        <div class="h-px flex-1 bg-primary/10" role="presentation" />
      </div>

      <!-- 学习目标 -->
      <div class="space-y-1.5">
        <Label for="goal">学习目标 *</Label>
        <Input
          id="goal"
          v-model="form.goal"
          placeholder="例如：三个月内成为能独立开发 Web 应用的后端工程师"
          :disabled="props.loading"
        />
        <p v-if="errors.goal" class="text-xs text-destructive">
          {{ errors.goal }}
        </p>
      </div>

      <!-- 当前水平 -->
      <div class="space-y-1.5">
        <Label for="level">当前水平</Label>
        <Select
          id="level"
          v-model="form.level"
          :options="LEVEL_OPTIONS"
          :disabled="props.loading"
        />
      </div>

      <!-- 组题：学习节奏 -->
      <div class="flex items-center gap-3 pt-1">
        <span class="text-xs font-medium text-muted-foreground">学习节奏</span>
        <div class="h-px flex-1 bg-primary/10" role="presentation" />
      </div>

      <!-- 每周可用小时 / 期望周数 -->
      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1.5">
          <Label for="weekly-hours">每周可用小时 *</Label>
          <Input
            id="weekly-hours"
            v-model="form.weeklyHours"
            type="number"
            :min="1"
            :max="80"
            :disabled="props.loading"
          />
          <p v-if="errors.weeklyHours" class="text-xs text-destructive">
            {{ errors.weeklyHours }}
          </p>
        </div>
        <div class="space-y-1.5">
          <Label for="weeks">期望周数 *</Label>
          <Input
            id="weeks"
            v-model="form.weeks"
            type="number"
            :min="1"
            :max="52"
            :disabled="props.loading"
          />
          <p v-if="errors.weeks" class="text-xs text-destructive">
            {{ errors.weeks }}
          </p>
        </div>
      </div>

      <!-- 每周学习日（需求 5：默认全选，至少保留一天） -->
      <div class="space-y-1.5">
        <Label>每周学习日</Label>
        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="(label, i) in STUDY_DAY_LABELS"
            :key="label"
            type="button"
            class="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :aria-pressed="form.studyDays.includes(i + 1)"
            :aria-label="`每周${label}${form.studyDays.includes(i + 1) ? '（已选）' : ''}`"
            :disabled="props.loading"
            @click="toggleStudyDay(i + 1)"
          >
            <Badge
              :variant="form.studyDays.includes(i + 1) ? 'default' : 'outline'"
              class="cursor-pointer select-none px-2.5 transition-opacity hover:opacity-80"
            >
              周{{ label }}
            </Badge>
          </button>
        </div>
        <p class="text-xs text-muted-foreground">
          计划章节将按所选日子排布到日历（至少保留一天）
        </p>
      </div>

      <!-- 组题：偏好与基础 -->
      <div class="flex items-center gap-3 pt-1">
        <span class="text-xs font-medium text-muted-foreground"
          >偏好与基础</span
        >
        <div class="h-px flex-1 bg-primary/10" role="presentation" />
      </div>

      <!-- 偏好方向（多选标签） -->
      <div class="space-y-1.5">
        <Label>偏好方向（可多选）</Label>
        <div class="flex flex-wrap gap-2">
          <button
            v-for="tag in INTEREST_PRESETS"
            :key="tag"
            type="button"
            class="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :disabled="props.loading"
            @click="toggleInterest(tag)"
          >
            <Badge
              :variant="form.interests.includes(tag) ? 'default' : 'outline'"
              class="cursor-pointer select-none transition-opacity hover:opacity-80"
            >
              {{ tag }}
            </Badge>
          </button>
        </div>
      </div>

      <!-- 已有基础 -->
      <div class="space-y-1.5">
        <Label for="background">已有基础</Label>
        <Textarea
          id="background"
          v-model="form.background"
          :rows="4"
          placeholder="描述你已经掌握的知识和做过的项目，帮助 Agent 校准起点（可留空，Agent 可能会追问）"
          :disabled="props.loading"
        />
      </div>
    </CardContent>

    <CardFooter>
      <Button class="w-full" :disabled="!canSubmit" @click="onSubmit">
        <svg
          v-if="props.loading"
          class="h-4 w-4 animate-spin"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path d="M21 12a9 9 0 1 1-6.219-8.56" />
        </svg>
        <span v-if="props.loading">Agent 规划中…</span>
        <span v-else>生成学习计划 ✨</span>
      </Button>
    </CardFooter>
  </Card>
</template>
