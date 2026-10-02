<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'

/**
 * Select 组件：基于原生 <select> 的样式化封装。
 * 相比 reka-ui 的复合 Select 组件更轻量，够用且无焦点管理复杂度。
 */
const props = defineProps<{
  class?: HTMLAttributes['class']
  options: Array<{ value: string; label: string }>
  disabled?: boolean
}>()

const model = defineModel<string>({ required: true })
</script>

<template>
  <div class="relative">
    <select
      v-model="model"
      :disabled="props.disabled"
      :class="
        cn(
          'flex h-9 w-full appearance-none rounded-md border border-input bg-transparent px-3 py-1 pr-9 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50',
          props.class,
        )
      "
    >
      <option v-for="opt in props.options" :key="opt.value" :value="opt.value">
        {{ opt.label }}
      </option>
    </select>
    <!-- 下拉箭头（原生 select 需自行绘制） -->
    <svg
      class="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  </div>
</template>
