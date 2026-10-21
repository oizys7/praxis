<script setup lang="ts">
/**
 * LoadingState：全站统一的加载态（大号墨蓝圆环 + 说明文字）。
 * 打卡日历 / 学习计划列表等页共用，保证加载视觉一致。
 */
withDefaults(
  defineProps<{
    /** 加载说明文字 */
    text?: string;
  }>(),
  { text: "正在加载…" },
);
</script>

<template>
  <div
    role="status"
    class="flex flex-col items-center justify-center gap-3 py-16"
  >
    <span
      class="h-10 w-10 animate-spin rounded-full border-2 border-primary border-t-transparent"
      aria-hidden="true"
    />
    <p class="text-sm text-muted-foreground">{{ text }}</p>
  </div>
</template>
