<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import DOMPurify from "dompurify";
import { marked } from "marked";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import MermaidDiagram from "@/components/MermaidDiagram.vue";

/**
 * MarkdownContent：章节 markdown 正文渲染。
 *
 * 段切策略（```mermaid 码块为界）：
 * 1. 正文按 ``` 围栏切分：mermaid 段交给 MermaidDiagram 组件
 *    （渲染失败降级 pre 展示源码，逻辑组件内自含）；
 * 2. 其余文本段经 marked 转 HTML 后再过 DOMPurify 消毒
 *    （v-html 红线：LLM 产出不可信任，消毒后才可注入 DOM）；
 * 3. 空正文是有意义的空态（骨架章 / 生成中）：渲染提示文案。
 */

type ContentSegment =
  { kind: "html"; html: string } | { kind: "mermaid"; code: string };

/**
 * 按需注册语言：只装学习计划里真会出现的几种（python/rust/js/ts/bash/
 * json/sql/yaml/xml/css/markdown），而不是引 highlight.js 全量包。
 * 认不出的语言走 highlightAuto 自动猜测。
 */
const HIGHLIGHT_LANGUAGES = {
  bash,
  css,
  javascript,
  json,
  markdown,
  python,
  rust,
  sql,
  typescript,
  xml,
  yaml,
};
for (const [name, language] of Object.entries(HIGHLIGHT_LANGUAGES)) {
  hljs.registerLanguage(name, language);
}

/**
 * 代码块语法高亮。刻意放在 marked 产出之后、DOMPurify 消毒之前：
 * 先高亮成带 span 的 HTML，再统一过一遍消毒，保证进入 v-html 的仍然是
 * 「消毒后」的内容（顺序反过来就等于把未消毒的 innerHTML 塞进 DOM）。
 * 配色不在这里写死，由 index.css 的 Catppuccin 变量按明暗主题给出。
 */
function highlightCodeBlocks(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("pre code").forEach((node) => {
    const el = node as HTMLElement;
    const code = el.textContent ?? "";
    if (!code.trim()) return;
    const declared = Array.from(el.classList)
      .find((cls) => cls.startsWith("language-"))
      ?.slice("language-".length);
    el.innerHTML =
      declared && hljs.getLanguage(declared)
        ? hljs.highlight(code, { language: declared, ignoreIllegals: true })
            .value
        : hljs.highlightAuto(code).value;
    const pre = el.parentElement!;
    pre.tabIndex = 0;
    pre.setAttribute("aria-label", `${declared || "纯文本"}代码，可横向滚动`);
    const block = doc.createElement("div");
    block.className = "code-block";
    const toolbar = doc.createElement("div");
    toolbar.className = "code-toolbar";
    const label = doc.createElement("span");
    label.textContent = declared || "代码";
    const copy = doc.createElement("button");
    copy.type = "button";
    copy.className = "code-copy";
    copy.dataset.copyCode = "true";
    copy.textContent = "复制代码";
    copy.setAttribute("aria-label", "复制代码");
    toolbar.append(label, copy);
    pre.replaceWith(block);
    block.append(toolbar, pre);
  });
  // 图片：懒加载 + 异步解码；加载失败由 onImageError（capture）替换占位。
  // 外链图片是否放行属运行环境策略：预览入口用 CSP 限制 img-src，并在真实
  // 数据入口改写 Markdown 外链图片；生产组件不含预览专用分支。
  doc.querySelectorAll("img").forEach((image) => {
    image.loading = "lazy";
    image.decoding = "async";
  });
  doc.querySelectorAll("a[href]").forEach((link) => {
    link.setAttribute("target", "_blank");
    link.setAttribute("rel", "noopener noreferrer");
  });
  doc.querySelectorAll("table").forEach((table) => {
    const wrapper = doc.createElement("div");
    wrapper.className = "table-scroll";
    wrapper.tabIndex = 0;
    wrapper.setAttribute("role", "region");
    wrapper.setAttribute("aria-label", "表格，可横向滚动");
    // 溢出提示（.table-scroll.is-overflow 时由 CSS 显示；短表不占空间）。
    // sticky left 让提示在横向滚动时保持可见。
    const hint = doc.createElement("span");
    hint.className = "table-hint";
    hint.setAttribute("aria-hidden", "true");
    hint.textContent = "表格较宽，可左右滑动查看";
    table.replaceWith(wrapper);
    wrapper.append(hint, table);
  });
  return doc.body.innerHTML;
}

/** 同步 marked 解析 + 高亮 + DOMPurify 消毒：进入 v-html 的 HTML 必经此函数 */
function renderHtml(markdown: string): string {
  const html = marked.parse(markdown, { async: false }) as string;
  return DOMPurify.sanitize(highlightCodeBlocks(html), {
    USE_PROFILES: { html: true },
    ADD_ATTR: ["target"],
  });
}

/** 正文 → 渲染段列表：mermaid 码块抽出为图组件段，其余段依次转消毒 HTML */
function toSegments(contentMd: string): ContentSegment[] {
  const segments: ContentSegment[] = [];
  let markdown = "";
  for (const token of marked.lexer(contentMd)) {
    if (token.type === "code" && token.lang?.trim() === "mermaid") {
      if (markdown) segments.push({ kind: "html", html: renderHtml(markdown) });
      markdown = "";
      segments.push({ kind: "mermaid", code: token.text });
    } else {
      markdown += token.raw;
    }
  }
  if (markdown) segments.push({ kind: "html", html: renderHtml(markdown) });
  return segments;
}

const props = defineProps<{
  /** 章节 markdown 正文全文（空串 = 有意义的空正文，渲染空态提示） */
  contentMd: string;
  /** 空正文提示文案（研读改版后章节语境可覆盖默认文案） */
  emptyHint?: string;
}>();

const segments = computed<ContentSegment[]>(() =>
  props.contentMd.trim() ? toSegments(props.contentMd) : [],
);
const copyStatus = ref("");
const resetTimers = new Set<ReturnType<typeof setTimeout>>();

// ---------------- 宽表溢出提示 ----------------

/** 正文根节点（各 html 段的容器，用于查找 .table-scroll） */
const rootEl = ref<HTMLElement | null>(null);
let tableObserver: ResizeObserver | null = null;

/** 单个包裹层的滚动边缘状态：驱动两侧渐隐显隐（index.css） */
function syncTableEdges(wrapper: HTMLElement): void {
  const maxScroll = wrapper.scrollWidth - wrapper.clientWidth;
  wrapper.classList.toggle("at-start", wrapper.scrollLeft <= 1);
  wrapper.classList.toggle("at-end", wrapper.scrollLeft >= maxScroll - 1);
}

/** 全量核对：是否溢出（决定提示与渐隐）+ 当前滚动边缘 */
function syncTableOverflow(): void {
  rootEl.value
    ?.querySelectorAll<HTMLElement>(".table-scroll")
    .forEach((wrapper) => {
      wrapper.classList.toggle(
        "is-overflow",
        wrapper.scrollWidth > wrapper.clientWidth + 1,
      );
      syncTableEdges(wrapper);
    });
}

function onTableScroll(event: Event): void {
  syncTableEdges(event.currentTarget as HTMLElement);
}

/** 内容段变化后重绑：v-html 整体替换 DOM，旧的观察目标随之失效 */
async function bindTableObservers(): Promise<void> {
  tableObserver?.disconnect();
  await nextTick();
  const root = rootEl.value;
  if (!root || typeof ResizeObserver === "undefined") return;
  tableObserver = new ResizeObserver(syncTableOverflow);
  root.querySelectorAll<HTMLElement>(".table-scroll").forEach((wrapper) => {
    wrapper.addEventListener("scroll", onTableScroll, { passive: true });
    tableObserver!.observe(wrapper);
    const table = wrapper.querySelector("table");
    if (table) tableObserver!.observe(table);
  });
  syncTableOverflow();
}

onMounted(() => void bindTableObservers());
watch(segments, () => void bindTableObservers());
onUnmounted(() => {
  resetTimers.forEach(clearTimeout);
  tableObserver?.disconnect();
  tableObserver = null;
});

/**
 * 图片加载失败降级（error 不冒泡，用捕获监听）：替换为占位提示，
 * 保留 alt 文本让读者知道缺的是什么，而不是留一个裂图图标。
 */
function onImageError(event: Event) {
  const img = event.target;
  if (!(img instanceof HTMLImageElement) || img.dataset.fallbackApplied) return;
  img.dataset.fallbackApplied = "true";
  const alt = img.getAttribute("alt") ?? "";
  const fallback = document.createElement("span");
  fallback.className = "img-fallback";
  fallback.textContent = alt ? `图片未能加载：${alt}` : "图片未能加载";
  img.replaceWith(fallback);
}

async function onContentClick(event: MouseEvent) {
  const button = (event.target as Element).closest<HTMLButtonElement>(
    "button[data-copy-code]",
  );
  const code = button?.closest(".code-block")?.querySelector("pre code");
  if (!button || !code || button.disabled) return;
  button.disabled = true;
  try {
    await navigator.clipboard.writeText(code.textContent ?? "");
    button.textContent = "已复制";
    copyStatus.value = "代码已复制到剪贴板";
  } catch {
    button.textContent = "复制失败，请重试";
    copyStatus.value = "无法访问剪贴板，请选中代码手动复制";
  }
  const timer = setTimeout(() => {
    button.textContent = "复制代码";
    button.disabled = false;
    resetTimers.delete(timer);
  }, 2000);
  resetTimers.add(timer);
}
</script>

<template>
  <div
    v-if="segments.length"
    ref="rootEl"
    class="reader-body space-y-6"
    @click="onContentClick"
    @error.capture="onImageError"
  >
    <p class="sr-only" role="status">{{ copyStatus }}</p>
    <template v-for="(segment, i) in segments" :key="i">
      <!-- html 段：marked 产出已经 DOMPurify 消毒，才允许落 v-html -->
      <!-- eslint-disable-next-line vue/no-v-html：DOMPurify 消毒红线已过 -->
      <div
        v-if="segment.kind === 'html'"
        class="markdown-code min-w-0"
        v-html="segment.html"
      />
      <MermaidDiagram v-else :code="segment.code" />
    </template>
  </div>
  <p v-else class="text-sm text-muted-foreground">
    {{ props.emptyHint ?? "正文尚未生成（可重新生成此章）。" }}
  </p>
</template>
