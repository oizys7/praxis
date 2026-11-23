<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

/** Card 容器（shadcn-vue 风格：无逻辑，仅样式封装） */
const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <div
    :class="
      cn('rounded-2xl border bg-card text-card-foreground shadow-sm', props.class)
    "
  >
    <slot />
  </div>
</template>
