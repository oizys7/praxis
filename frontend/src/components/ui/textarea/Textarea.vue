<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

/** Textarea 组件（shadcn-vue 风格） */
const props = defineProps<{
  class?: HTMLAttributes["class"];
  placeholder?: string;
  disabled?: boolean;
  rows?: number;
}>();

const model = defineModel<string>({ default: "" });
</script>

<template>
  <textarea
    v-model="model"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :rows="props.rows ?? 3"
    :class="
      cn(
        'flex min-h-[60px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50',
        props.class,
      )
    "
  />
</template>
