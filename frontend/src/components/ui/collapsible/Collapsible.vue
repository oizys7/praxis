<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { CollapsibleRoot } from 'reka-ui'
import { cn } from '@/lib/utils'

/**
 * Collapsible 根组件：包装 reka-ui 的 CollapsibleRoot。
 * 根元素会携带 data-state="open|closed"，后代可用 group-data-[state=open] 响应展开状态。
 */
const props = defineProps<{
  class?: HTMLAttributes['class']
  defaultOpen?: boolean
}>()
</script>

<template>
  <CollapsibleRoot
    :default-open="props.defaultOpen"
    :class="cn('group', props.class)"
  >
    <slot />
  </CollapsibleRoot>
</template>
