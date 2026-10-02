<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'

/** Label 组件：表单控件标签 */
const props = defineProps<{
  class?: HTMLAttributes['class']
  for?: string
}>()
</script>

<template>
  <label
    :for="props.for"
    :class="cn('text-sm font-medium leading-none peer-disabled:opacity-70', props.class)"
  >
    <slot />
  </label>
</template>
