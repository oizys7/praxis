<script setup lang="ts">
/**
 * NotesView：笔记生成页（空占位）。
 * 功能未实现，仅提供导航可达的空态卡——后续在此挂载笔记生成工作流。
 */
import { NotebookPen } from "lucide-vue-next";
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-8 2xl:max-w-7xl">
    <!-- 页头：与其他页同构的衬线大标题 + hairline -->
    <div class="mb-6">
      <h2 class="font-display text-3xl font-semibold tracking-tight">
        笔记生成
      </h2>
      <p class="mt-1 text-sm text-muted-foreground">
        把学过的章节整理成可复习的笔记
      </p>
      <div class="mt-4 h-px bg-primary/15" role="presentation" />
    </div>
    <!-- 空态：方向性邀请（空屏是行动邀请，不是气氛渲染） -->
    <div class="flex justify-center py-16">
      <div
        class="w-full max-w-sm rounded-xl border border-dashed bg-card/60 px-10 py-14 text-center"
      >
        <NotebookPen
          class="mx-auto h-10 w-10 text-primary/40"
          aria-hidden="true"
        />
        <p class="mt-4 font-medium">笔记生成功能尚未开放</p>
        <p class="mt-1.5 text-sm text-muted-foreground">
          先在「学习计划」里积累章节内容，笔记整理将在后续版本上线。
        </p>
      </div>
    </div>
  </div>
</template>
