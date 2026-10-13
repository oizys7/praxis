<script lang="ts">
/**
 * mermaid 动态加载（模块级单例）：import("mermaid") 不进主 bundle，
 * 首次渲染时按需加载；initialize 幂等（合并配置），只在首次加载成功后执行一次。
 */
let mermaidLoader: Promise<typeof import("mermaid")> | null = null;

function loadMermaid(): Promise<typeof import("mermaid")> {
  if (!mermaidLoader) {
    mermaidLoader = import("mermaid").then((mod) => {
      mod.default.initialize({ startOnLoad: false, theme: "default" });
      return mod;
    });
  }
  return mermaidLoader;
}

/** 渲染容器 id 序列（mermaid.render 要求文档内唯一 id） */
let renderSeq = 0;
</script>

<script setup lang="ts">
import { onMounted, ref, watch } from "vue";

/**
 * MermaidDiagram：渲染 markdown 正文内联 ```mermaid 码块（MarkdownContent 切段后传入）。
 * 渲染失败（语法错误 / 动态加载失败）不阻断页面，降级为 <pre> 显示源码。
 */
const props = defineProps<{
  /** Mermaid 源码 */
  code: string;
  /** 图注说明 */
  caption?: string;
}>();

/** 渲染成功的 SVG 字符串（null = 未就绪或失败） */
const svg = ref<string | null>(null);
/** 渲染失败：回退 <pre> 显示源码 */
const failed = ref(false);

async function render(): Promise<void> {
  failed.value = false;
  svg.value = null;
  if (!props.code.trim()) {
    failed.value = true;
    return;
  }
  const id = `praxis-mermaid-${++renderSeq}`;
  try {
    const mod = await loadMermaid();
    const { svg: out } = await mod.default.render(id, props.code);
    svg.value = out;
  } catch {
    // mermaid.render 失败时可能在 body 残留错误占位节点（#d{id}），清理掉
    document.getElementById(`d${id}`)?.remove();
    failed.value = true;
  }
}

onMounted(render);
watch(() => props.code, render);
</script>

<template>
  <figure class="rounded-lg border bg-muted/30 p-3">
    <!-- 渲染成功：注入 SVG（mermaid 样式随 SVG 内嵌，限宽自适应容器） -->
    <!-- eslint-disable-next-line vue/no-v-html：mermaid 产出的受信 SVG -->
    <div
      v-if="svg"
      class="flex justify-center overflow-x-auto [&_svg]:h-auto [&_svg]:max-w-full"
      v-html="svg"
    />
    <!-- 渲染失败 / 动态加载失败：降级显示源码（不阻断页面） -->
    <pre
      v-else-if="failed"
      class="overflow-x-auto rounded bg-muted p-2 text-xs leading-relaxed text-muted-foreground"
    >
图示渲染失败，显示原始源码：
{{ props.code }}</pre>
    <div v-else class="py-6 text-center text-xs text-muted-foreground">
      图示渲染中…
    </div>
    <figcaption
      v-if="props.caption"
      class="mt-2 text-center text-xs text-muted-foreground"
    >
      {{ props.caption }}
    </figcaption>
  </figure>
</template>
