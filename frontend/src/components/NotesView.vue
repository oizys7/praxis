<script setup lang="ts">
/**
 * NotesView：笔记生成页（空占位）。
 * 功能未实现，仅提供导航可达的空态卡——后续在此挂载笔记生成工作流。
 */
import { NotebookPen } from "lucide-vue-next";
</script>

<template>
  <div class="flex h-full items-center justify-center px-6 py-8">
    <div
      class="w-full max-w-sm rounded-xl border bg-card px-10 py-14 text-center shadow-sm"
    >
      <NotebookPen
        class="mx-auto h-12 w-12 text-muted-foreground/60"
        aria-hidden="true"
      />
      <h2 class="font-display mt-5 text-lg font-semibold">笔记生成</h2>
      <p class="mt-2 text-sm text-muted-foreground">功能待实现</p>
    </div>
  </div>
</template>
