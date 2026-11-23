<script setup lang="ts">
import { computed, nextTick, reactive } from "vue";
import { Check, ChevronDown, LoaderCircle } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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

/** 只在输入有效时展示节奏摘要，避免将无效输入呈现为计划。 */
const rhythmSummary = computed(() => {
  const hours = Number(form.weeklyHours);
  const weeks = Number(form.weeks);
  if (
    !Number.isInteger(hours) ||
    hours < 1 ||
    hours > 80 ||
    !Number.isInteger(weeks) ||
    weeks < 1 ||
    weeks > 52
  )
    return "填写每周时长和周数，安排适合自己的学习节奏。";
  return `每周 ${form.studyDays.length} 天，共 ${weeks} 周，预计投入 ${hours * weeks} 小时。`;
});

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
    !Number.isInteger(hours) || hours < 1 || hours > 80
      ? "请填写 1–80 之间的整数"
      : "";
  const weeks = Number(form.weeks);
  errors.weeks =
    !Number.isInteger(weeks) || weeks < 1 || weeks > 52
      ? "请填写 1-52 之间的整数"
      : "";
  return !errors.goal && !errors.weeklyHours && !errors.weeks;
}

/** 提交：校验通过后组装 PlanRequest 抛给父组件 */
async function onSubmit() {
  if (props.loading) return;
  if (!validate()) {
    await nextTick();
    const field = errors.goal
      ? "goal"
      : errors.weeklyHours
        ? "weekly-hours"
        : "weeks";
    document.getElementById(field)?.focus();
    return;
  }
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
  <form
    class="min-w-0 space-y-6"
    novalidate
    :aria-busy="props.loading"
    @submit.prevent="onSubmit"
  >
    <div class="divide-y divide-border rounded-2xl border bg-card px-5 sm:px-8">
      <section aria-labelledby="goal-heading" class="space-y-6 py-7 sm:py-8">
        <div>
          <h3 id="goal-heading" class="text-base font-semibold">目标与起点</h3>
          <p class="mt-1 text-sm text-muted-foreground">
            想学会什么，以及现在走到了哪里。
          </p>
        </div>
        <div class="space-y-2">
          <Label for="goal"
            >学习目标 <span class="text-muted-foreground">（必填）</span></Label
          >
          <Textarea
            id="goal"
            v-model="form.goal"
            :rows="4"
            class="resize-y bg-background/70 text-base leading-7 shadow-none aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive/30"
            placeholder="例如：三个月内掌握后端开发，独立完成一个带登录和数据库的 Web 应用。"
            required
            :aria-invalid="!!errors.goal"
            :aria-describedby="
              errors.goal ? 'goal-hint goal-error' : 'goal-hint'
            "
            :disabled="props.loading"
            @update:model-value="errors.goal = ''"
          />
          <p id="goal-hint" class="text-xs leading-5 text-muted-foreground">
            写下希望完成的事，比只写一个学科名称更有帮助。
          </p>
          <p
            v-if="errors.goal"
            id="goal-error"
            role="alert"
            class="text-sm text-destructive"
          >
            {{ errors.goal }}
          </p>
        </div>
        <div class="space-y-2 sm:max-w-sm">
          <Label for="level">当前水平</Label>
          <div class="relative">
            <select
              id="level"
              v-model="form.level"
              :disabled="props.loading"
              class="h-11 w-full appearance-none rounded-md border border-input bg-background/70 px-3 pr-10 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
            >
              <option
                v-for="option in LEVEL_OPTIONS"
                :key="option.value"
                :value="option.value"
              >
                {{ option.label }}
              </option>
            </select>
            <ChevronDown
              class="pointer-events-none absolute right-3 top-3.5 h-4 w-4 text-muted-foreground"
              aria-hidden="true"
            />
          </div>
        </div>
      </section>

      <section aria-labelledby="rhythm-heading" class="space-y-6 py-7 sm:py-8">
        <div>
          <h3 id="rhythm-heading" class="text-base font-semibold">学习节奏</h3>
          <p class="mt-1 text-sm text-muted-foreground">
            为学习留出时间，也为日常留出余地。
          </p>
        </div>
        <div class="grid gap-5 sm:grid-cols-2">
          <div class="space-y-2">
            <Label for="weekly-hours"
              >每周投入
              <span class="text-muted-foreground">（小时，必填）</span></Label
            >
            <Input
              id="weekly-hours"
              v-model="form.weeklyHours"
              type="number"
              :min="1"
              :max="80"
              step="1"
              required
              class="h-11 bg-background/70 shadow-none aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive/30"
              :disabled="props.loading"
              :aria-invalid="!!errors.weeklyHours"
              :aria-describedby="
                errors.weeklyHours ? 'hours-hint hours-error' : 'hours-hint'
              "
              @update:model-value="errors.weeklyHours = ''"
            />
            <p id="hours-hint" class="text-xs text-muted-foreground">
              1–80 小时，填写整数
            </p>
            <p
              v-if="errors.weeklyHours"
              id="hours-error"
              role="alert"
              class="text-sm text-destructive"
            >
              {{ errors.weeklyHours }}
            </p>
          </div>
          <div class="space-y-2">
            <Label for="weeks"
              >计划时长
              <span class="text-muted-foreground">（周，必填）</span></Label
            >
            <Input
              id="weeks"
              v-model="form.weeks"
              type="number"
              :min="1"
              :max="52"
              step="1"
              required
              class="h-11 bg-background/70 shadow-none aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive/30"
              :disabled="props.loading"
              :aria-invalid="!!errors.weeks"
              :aria-describedby="
                errors.weeks ? 'weeks-hint weeks-error' : 'weeks-hint'
              "
              @update:model-value="errors.weeks = ''"
            />
            <p id="weeks-hint" class="text-xs text-muted-foreground">
              1–52 周，填写整数
            </p>
            <p
              v-if="errors.weeks"
              id="weeks-error"
              role="alert"
              class="text-sm text-destructive"
            >
              {{ errors.weeks }}
            </p>
          </div>
        </div>
        <fieldset class="min-w-0 space-y-3" aria-describedby="study-days-hint">
          <legend class="text-sm font-medium">每周学习日</legend>
          <div class="flex flex-wrap gap-2">
            <button
              v-for="(label, i) in STUDY_DAY_LABELS"
              :key="label"
              type="button"
              class="flex min-h-11 min-w-11 items-center justify-center gap-1 rounded-lg border px-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              :class="
                form.studyDays.includes(i + 1)
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-input bg-background/70 text-muted-foreground hover:bg-secondary'
              "
              :aria-pressed="form.studyDays.includes(i + 1)"
              :aria-label="`周${label}`"
              :disabled="props.loading"
              @click="toggleStudyDay(i + 1)"
            >
              <Check
                v-if="form.studyDays.includes(i + 1)"
                class="h-3.5 w-3.5"
                aria-hidden="true"
              />
              周{{ label }}
            </button>
          </div>
          <p
            id="study-days-hint"
            class="text-xs leading-5 text-muted-foreground"
          >
            章节将按所选日期安排到日历，至少保留一天。
          </p>
        </fieldset>
        <p
          class="rounded-lg bg-secondary/60 px-4 py-3 text-sm leading-6 text-muted-foreground"
          aria-live="polite"
        >
          {{ rhythmSummary }}
        </p>
      </section>

      <section
        aria-labelledby="preferences-heading"
        class="space-y-6 py-7 sm:py-8"
      >
        <div>
          <h3 id="preferences-heading" class="text-base font-semibold">
            偏好与基础
            <span class="ml-2 text-sm font-normal text-muted-foreground"
              >选填</span
            >
          </h3>
          <p class="mt-1 text-sm text-muted-foreground">
            让练习贴近你的兴趣，让内容从合适的地方开始。
          </p>
        </div>
        <fieldset class="min-w-0 space-y-3">
          <legend class="text-sm font-medium">
            感兴趣的方向
            <span class="font-normal text-muted-foreground">（可多选）</span>
          </legend>
          <div class="flex flex-wrap gap-2">
            <button
              v-for="tag in INTEREST_PRESETS"
              :key="tag"
              type="button"
              class="inline-flex min-h-11 items-center gap-2 rounded-lg border px-4 py-2 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              :class="
                form.interests.includes(tag)
                  ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                  : 'border-input bg-background/70 text-muted-foreground hover:bg-secondary'
              "
              :aria-pressed="form.interests.includes(tag)"
              :disabled="props.loading"
              @click="toggleInterest(tag)"
            >
              <Check
                v-if="form.interests.includes(tag)"
                class="h-3.5 w-3.5"
                aria-hidden="true"
              />
              {{ tag }}
            </button>
          </div>
        </fieldset>
        <div class="space-y-2">
          <Label for="background">已有基础</Label>
          <Textarea
            id="background"
            v-model="form.background"
            :rows="3"
            class="resize-y bg-background/70 leading-7 shadow-none"
            placeholder="已经学过哪些知识，或做过什么项目？"
            :disabled="props.loading"
            aria-describedby="background-hint"
          />
          <p
            id="background-hint"
            class="text-xs leading-5 text-muted-foreground"
          >
            可以留空；需要更多信息时，会进一步向你确认。
          </p>
        </div>
      </section>
    </div>

    <footer
      class="flex flex-col gap-4 pb-2 sm:flex-row sm:items-center sm:justify-between"
    >
      <p class="text-sm leading-6 text-muted-foreground">
        生成的计划会自动保存到列表。
      </p>
      <Button type="submit" class="min-h-11 px-7" :disabled="props.loading">
        <LoaderCircle
          v-if="props.loading"
          class="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none"
          aria-hidden="true"
        />
        {{ props.loading ? "正在规划…" : "生成学习计划" }}
      </Button>
    </footer>
  </form>
</template>
