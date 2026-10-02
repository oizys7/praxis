<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { CollapsibleContent } from 'reka-ui'
import { cn } from '@/lib/utils'

/** Collapsible 内容区：展开时显示 */
const props = defineProps<{
  class?: HTMLAttributes['class']
}>()
</script>

<template>
  <CollapsibleContent :class="cn('', props.class)">
    <slot />
  </CollapsibleContent>
</template>
