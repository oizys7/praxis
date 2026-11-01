<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { CollapsibleTrigger } from "reka-ui";
import { cn } from "@/lib/utils";

/** Collapsible 触发器：渲染为 <button>，点击切换展开/收起 */
const props = defineProps<{
  class?: HTMLAttributes["class"];
  disabled?: boolean;
}>();
</script>

<template>
  <CollapsibleTrigger
    :disabled="props.disabled"
    :class="cn('', props.class)"
    as-child
  >
    <slot />
  </CollapsibleTrigger>
</template>
