<script lang="ts">
import type { Component } from "vue";

/** 侧边栏菜单项：icon 为 lucide-vue-next 图标组件（跟随文字颜色，支持树摇） */
export interface MenuItem {
  /** 页面唯一标识 */
  id: string;
  /** 显示文案 */
  label: string;
  /** 菜单图标组件（如 BookOpen / Settings） */
  icon: Component;
}
</script>

<script setup lang="ts">
/**
 * AppSidebar：左侧菜单栏，支持点击折叠（窄条图标模式）。
 * 新增页面时在 App 的 MENU_ITEMS 中追加菜单项，并在内容区按 activePage 渲染。
 */
import { ChevronLeft } from "lucide-vue-next";

const props = defineProps<{
  items: MenuItem[];
  /** 当前激活页面 id（v-model） */
  modelValue: string;
  /** 是否折叠为窄条（v-model:collapsed） */
  collapsed: boolean;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", id: string): void;
  (e: "update:collapsed", collapsed: boolean): void;
}>();

/** 切换折叠状态（底部箭头按钮） */
function toggleCollapsed() {
  emit("update:collapsed", !props.collapsed);
}
</script>

<template>
  <aside
    class="flex shrink-0 flex-col border-r bg-card/40 transition-all duration-200"
    :class="collapsed ? 'w-14' : 'w-56'"
  >
    <nav class="flex-1 space-y-1 overflow-y-auto p-2" aria-label="主导航">
      <button
        v-for="item in items"
        :key="item.id"
        type="button"
        class="flex w-full items-center rounded-lg px-3 py-2.5 text-sm transition-colors"
        :class="[
          collapsed ? 'justify-center' : 'gap-3',
          item.id === modelValue
            ? 'bg-primary/10 font-medium text-primary'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        :title="collapsed ? item.label : undefined"
        :aria-current="item.id === modelValue ? 'page' : undefined"
        @click="emit('update:modelValue', item.id)"
      >
        <!-- 动态组件：lucide 图标 stroke=currentColor，选中高亮时自动跟随文字变色 -->
        <component :is="item.icon" class="h-5 w-5 shrink-0" />
        <span v-if="!collapsed">{{ item.label }}</span>
      </button>
    </nav>

    <!-- 折叠 / 展开切换 -->
    <div class="border-t p-2">
      <button
        type="button"
        class="flex w-full items-center justify-center rounded-lg px-3 py-2 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        :title="collapsed ? '展开菜单' : '折叠菜单'"
        @click="toggleCollapsed"
      >
        <ChevronLeft
          class="h-4 w-4 transition-transform duration-200"
          :class="collapsed ? 'rotate-180' : ''"
        />
      </button>
    </div>
  </aside>
</template>
