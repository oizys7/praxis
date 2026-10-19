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

/**
 * 菜单项按钮基类：对齐布局恒量化——justify/gap 恒定，唯一变化的 px
 * 走 transition。折叠态另课 px-2.5：nav p-2 除外后按钮净宽 40px，
 * 图标 20px 居中即 (40-20)/2=10px（早前按 aside 全宽 56px 误算 18px
 * 造成图标右偏 8px）。单提常量避免模板行超宽
 */
const ITEM_BTN_CLASS =
  "flex w-full items-center justify-start gap-3 rounded-lg py-2.5 text-sm transition-[padding,background-color,color] duration-300 ease-in-out";
</script>

<template>
  <!-- 纸面侧栏：宽度过渡 300ms；overflow-hidden 裁剪过渡期内容
       （文字重排逐帧抖闪 + overflow-x 隐式抬升横向滚动条的根治位） -->
  <aside
    class="flex shrink-0 flex-col overflow-hidden border-r bg-card/40 transition-[width] duration-300 ease-in-out"
    :class="collapsed ? 'w-14' : 'w-56'"
  >
    <!-- overflow-y-auto 会把 overflow-x 隐式抬为 auto（过渡中变宽文字溢出
         即出现横向滚动条），x 向显式压住 -->
    <nav
      class="flex-1 space-y-1 overflow-x-hidden overflow-y-auto p-2"
      aria-label="主导航"
    >
      <!-- 对齐布局恒量化：justify/gap 恒定，唯一变化的 px 走 transition
           （折叠 18px 使图标在 56px 窄条内精确居中，不再瞬跳） -->
      <button
        v-for="item in items"
        :key="item.id"
        type="button"
        :class="[
          ITEM_BTN_CLASS,
          collapsed ? 'px-2.5' : 'px-3',
          item.id === modelValue
            ? 'relative bg-secondary/60 font-semibold text-primary'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        v-tip="collapsed ? item.label : undefined"
        :aria-current="item.id === modelValue ? 'page' : undefined"
        @click="emit('update:modelValue', item.id)"
      >
        <!-- 左缘墨点：活跃态指示条（span 方案兑现，免伪元素 content 引号嵌套） -->
        <span
          v-if="item.id === modelValue"
          class="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-primary"
          aria-hidden="true"
        />
        <!-- 动态组件：lucide 图标 stroke=currentColor，选中高亮时自动跟随文字变色 -->
        <component
          :is="item.icon"
          class="h-5 w-5 shrink-0"
          aria-hidden="true"
        />
        <!-- 标签 v-if 保留原被验证过的双态布局（常驻挂载 + min-w-0 在
             实测中致 active 项竖排，回退）；折叠后的文字瞬隐被 aside
             的 overflow-hidden 裁剪 + 300ms 宽度过渡掩盖，图标走位由
             恒定 justify-start + padding 渐变保证平滑 -->
        <span v-if="!collapsed" class="whitespace-nowrap">{{
          item.label
        }}</span>
      </button>
    </nav>

    <!-- 折叠 / 展开切换 -->
    <div class="border-t p-2">
      <button
        type="button"
        class="flex w-full items-center justify-center rounded-lg px-3 py-2 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        v-tip="collapsed ? '展开菜单' : '折叠菜单'"
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
