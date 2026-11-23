<script lang="ts">
import type { Component } from "vue";
export interface MenuItem {
  id: string;
  label: string;
  icon: Component;
}
</script>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { ChevronLeft, Compass } from "lucide-vue-next";
const props = defineProps<{
  items: MenuItem[];
  modelValue: string;
  collapsed: boolean;
}>();
const emit = defineEmits<{
  (e: "update:modelValue", id: string): void;
  (e: "update:collapsed", collapsed: boolean): void;
}>();

/**
 * tooltip 只在标签不可见时才有意义：手动折叠（collapsed）或 CSS 窄屏
 * 自动隐藏文字（<768px，与 index.css 断点一致）。展开态标签可读，
 * 再弹 tooltip 是复述 + 盖住上一项。
 */
const compact = ref(false);
let mediaQuery: MediaQueryList | null = null;

function syncCompact() {
  compact.value = mediaQuery?.matches ?? false;
}

onMounted(() => {
  mediaQuery = window.matchMedia("(max-width: 767px)");
  syncCompact();
  mediaQuery.addEventListener("change", syncCompact);
});

onBeforeUnmount(() => {
  mediaQuery?.removeEventListener("change", syncCompact);
  mediaQuery = null;
});
</script>

<template>
  <aside class="app-sidebar flex shrink-0 flex-col border-r bg-card" :class="{ 'is-collapsed': collapsed }">
    <div class="sidebar-caption px-6 pb-3 pt-8 text-xs text-muted-foreground">学习空间</div>
    <nav class="flex-1 space-y-2 overflow-x-hidden overflow-y-auto px-2 py-4 sm:px-3 sm:py-2" aria-label="主导航">
      <button
        v-for="item in items" :key="item.id" type="button"
        class="nav-item relative flex min-h-12 w-full items-center gap-3 rounded-xl px-3 text-sm transition-colors"
        :class="item.id === modelValue ? 'bg-accent font-semibold text-primary' : 'text-muted-foreground hover:bg-muted hover:text-foreground'"
        :aria-label="item.label"
        :aria-current="item.id === modelValue ? 'page' : undefined"
        v-tip="collapsed || compact ? item.label : ''"
        @click="emit('update:modelValue', item.id)"
      >
        <component :is="item.icon" class="h-[19px] w-[19px] shrink-0" :stroke-width="1.7" aria-hidden="true" />
        <span class="nav-label whitespace-nowrap">{{ item.label }}</span>
      </button>
    </nav>
    <div class="sidebar-caption mx-5 mb-6 border-t pt-5">
      <Compass class="mb-3 h-5 w-5 text-primary" :stroke-width="1.5" aria-hidden="true" />
      <p class="text-sm leading-6 text-muted-foreground">循序渐进，<br />让知识成为自己的。</p>
    </div>
    <div class="sidebar-collapse border-t p-3">
      <button type="button" class="flex min-h-11 w-full items-center justify-center gap-2 rounded-lg text-xs text-muted-foreground hover:bg-muted"
        :aria-label="collapsed ? '展开菜单' : '折叠菜单'" :aria-expanded="!collapsed"
        @click="emit('update:collapsed', !props.collapsed)">
        <ChevronLeft class="h-4 w-4" :class="{ 'rotate-180': collapsed }" aria-hidden="true" />
        <span class="nav-label">收起侧栏</span>
      </button>
    </div>
  </aside>
</template>
