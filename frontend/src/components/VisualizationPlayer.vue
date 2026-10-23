<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { Pause, Play, RotateCcw, SkipBack, SkipForward } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import type { Visualization } from "@/types/agent";

/**
 * 过程动画播放器（array 渲染器）。
 *
 * 职责边界：模型只产出「状态序列」，这里负责怎么画。两者分开的好处是——
 * 数据错了看得见（格子里的值不对），渲染错了只在一个组件里，
 * 而不会出现「模型写的动画代码崩了，你分不清是渲染问题还是理解问题」。
 *
 * 只认 kind="array"：一排格子 + 高亮。这个形态能覆盖排序、双指针、
 * 滑动窗口、DP 一维表、栈/队列进出等绝大多数「执行过程不可见」的场景；
 * 树旋转、链表指针这类需要更专门的渲染器，等真有需求再加。
 */
const props = defineProps<{
  visualization: Visualization;
}>();

const STEP_INTERVAL_MS = 900;

const cursor = ref(0);
const playing = ref(false);
let timer: number | null = null;

const steps = computed(() => props.visualization.steps ?? []);
const total = computed(() => steps.value.length);
const step = computed(() => steps.value[cursor.value] ?? null);
const cells = computed(() => step.value?.state ?? []);
const highlighted = computed(() => new Set(step.value?.highlight ?? []));
const atEnd = computed(() => cursor.value >= total.value - 1);

function stopPlayback(): void {
  playing.value = false;
  if (timer !== null) {
    window.clearInterval(timer);
    timer = null;
  }
}

function next(): void {
  if (atEnd.value) {
    stopPlayback();
    return;
  }
  cursor.value += 1;
}

function prev(): void {
  cursor.value = Math.max(0, cursor.value - 1);
}

function reset(): void {
  stopPlayback();
  cursor.value = 0;
}

function toggle(): void {
  if (playing.value) {
    stopPlayback();
    return;
  }
  // 已经播到末尾时再点播放＝从头再看一遍
  if (atEnd.value) cursor.value = 0;
  playing.value = true;
  timer = window.setInterval(() => {
    if (cursor.value >= total.value - 1) stopPlayback();
    else cursor.value += 1;
  }, STEP_INTERVAL_MS);
}

// 换一段动画（重新生成或切换章节）时回到第一帧，不要停在别人的进度上
watch(
  () => props.visualization,
  () => {
    stopPlayback();
    cursor.value = 0;
  },
);
onBeforeUnmount(stopPlayback);
</script>

<template>
  <figure v-if="total" class="rounded-lg border bg-muted/30 p-3">
    <figcaption class="mb-2 flex items-baseline justify-between gap-2">
      <span class="text-xs font-semibold text-foreground">
        {{ props.visualization.title || "过程演示" }}
      </span>
      <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
        {{ cursor + 1 }} / {{ total }}
      </span>
    </figcaption>

    <!-- 帧舞台：fixed min-height 防抖动（格子文本长短不一会让整块跳动） -->
    <div
      class="flex min-h-16 flex-wrap items-center justify-center gap-1.5 py-2"
      role="img"
      :aria-label="`过程动画第 ${cursor + 1} 帧，共 ${total} 帧：${step?.note ?? ''}`"
    >
      <span
        v-for="(cell, i) in cells"
        :key="i"
        class="inline-flex h-9 min-w-9 items-center justify-center rounded border px-1.5 text-xs tabular-nums transition-colors duration-200"
        :class="
          highlighted.has(i)
            ? 'border-primary bg-primary font-semibold text-primary-foreground'
            : 'border-border bg-card text-foreground'
        "
      >
        {{ cell }}
      </span>
    </div>

    <p
      class="min-h-8 text-center text-xs leading-relaxed text-muted-foreground"
    >
      {{ step?.note }}
    </p>

    <div class="mt-1 flex items-center justify-center gap-1">
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7"
        aria-label="上一帧"
        @click="prev"
      >
        <SkipBack class="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7"
        :aria-label="playing ? '暂停' : '播放'"
        @click="toggle"
      >
        <Pause v-if="playing" class="h-3.5 w-3.5" aria-hidden="true" />
        <Play v-else class="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7"
        aria-label="下一帧"
        :disabled="atEnd"
        @click="next"
      >
        <SkipForward class="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7"
        aria-label="回到第一帧"
        @click="reset"
      >
        <RotateCcw class="h-3.5 w-3.5" aria-hidden="true" />
      </Button>
    </div>
  </figure>
</template>
