<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";
import { type ButtonVariants, buttonVariants } from ".";

/**
 * Button 组件（shadcn-vue 风格）
 * 原生 <button> 封装，通过 variant / size 控制样式。
 */
const props = withDefaults(
  defineProps<{
    variant?: ButtonVariants["variant"];
    size?: ButtonVariants["size"];
    class?: HTMLAttributes["class"];
    type?: "button" | "submit" | "reset";
    disabled?: boolean;
  }>(),
  {
    variant: "default",
    size: "default",
    type: "button",
    disabled: false,
  },
);
</script>

<template>
  <button
    :type="props.type"
    :disabled="props.disabled"
    :class="
      cn(
        buttonVariants({ variant: props.variant, size: props.size }),
        props.class,
      )
    "
  >
    <slot />
  </button>
</template>
