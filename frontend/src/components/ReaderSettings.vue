<script setup lang="ts">
/**
 * ReaderSettings：研读正文阅读设置弹层（字号 / 行距滑块）。
 *
 * 纯受控组件：滑块范围由宿主以 props 注入（StudyView 持久化恢复时按同一
 * 范围做 clamp），选值经 update 事件回传。实际样式经 CSS 变量
 * --reader-font-size / --reader-line-height 注入正文
 * （见 assets/index.css 的 .reader-body 与 .markdown-code）。
 */
import { computed } from "vue";

const props = defineProps<{
  /** 当前字号（px） */
  fontSize: number;
  /** 字号滑块下限（px） */
  fontMin: number;
  /** 字号滑块上限（px） */
  fontMax: number;
  /** 当前行距（无单位倍数） */
  lineHeight: number;
  /** 行距滑块下限 */
  lineMin: number;
  /** 行距滑块上限 */
  lineMax: number;
}>();

const emit = defineEmits<{
  update: [kind: "fontSize" | "lineHeight", value: number];
}>();

const fontSizeModel = computed({
  get: () => props.fontSize,
  set: (value) => emit("update", "fontSize", Number(value)),
});

const lineHeightModel = computed({
  get: () => props.lineHeight,
  set: (value) => emit("update", "lineHeight", Number(value)),
});
</script>

<template>
  <div
    data-study-reader-settings
    class="w-72 rounded-xl border bg-popover p-3 shadow-xl"
    role="group"
    aria-label="阅读设置"
  >
    <div class="flex items-center gap-3">
      <span class="shrink-0 text-xs text-muted-foreground">字号</span>
      <input
        v-model.number="fontSizeModel"
        type="range"
        data-study-font-slider
        class="h-1.5 w-full cursor-pointer accent-primary"
        :min="fontMin"
        :max="fontMax"
        step="1"
        aria-label="字号"
      />
      <span
        class="w-10 shrink-0 text-right text-xs tabular-nums text-muted-foreground"
        >{{ fontSize }}px</span
      >
    </div>
    <div class="mt-3 flex items-center gap-3">
      <span class="shrink-0 text-xs text-muted-foreground">行距</span>
      <input
        v-model.number="lineHeightModel"
        type="range"
        data-study-line-slider
        class="h-1.5 w-full cursor-pointer accent-primary"
        :min="lineMin"
        :max="lineMax"
        step="0.05"
        aria-label="行距"
      />
      <span
        class="w-10 shrink-0 text-right text-xs tabular-nums text-muted-foreground"
        >{{ lineHeight.toFixed(2) }}</span
      >
    </div>
  </div>
</template>
