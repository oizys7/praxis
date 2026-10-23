<script setup lang="ts">
import { computed } from "vue";
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
 * 图片条款兑底：LLM 产出的内嵌 <img> 统一懒加载 + 异步解码；
 * 真实尺寸未知，溢出/CLS 交给容器 max-w/h-auto 兜底（见模板变体）
 */
function lazyImages(node: Node) {
  const el = node as Element;
  if (el.tagName === "IMG") {
    el.setAttribute("loading", "lazy");
    el.setAttribute("decoding", "async");
  }
}
DOMPurify.addHook("afterSanitizeAttributes", lazyImages);

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
  });
  return doc.body.innerHTML;
}

/** 同步 marked 解析 + 高亮 + DOMPurify 消毒：进入 v-html 的 HTML 必经此函数 */
function renderHtml(markdown: string): string {
  const html = marked.parse(markdown, { async: false }) as string;
  return DOMPurify.sanitize(highlightCodeBlocks(html), {
    USE_PROFILES: { html: true },
  });
}

/** 正文 → 渲染段列表：mermaid 码块抽出为图组件段，其余段依次转消毒 HTML */
function toSegments(contentMd: string): ContentSegment[] {
  const segments: ContentSegment[] = [];
  const fence = /```mermaid[ \t]*\n([\s\S]*?)```/g;
  let cursor = 0;
  let match: RegExpExecArray | null;
  while ((match = fence.exec(contentMd)) !== null) {
    if (match.index > cursor) {
      segments.push({
        kind: "html",
        html: renderHtml(contentMd.slice(cursor, match.index)),
      });
    }
    segments.push({ kind: "mermaid", code: match[1] });
    cursor = match.index + match[0].length;
  }
  if (cursor < contentMd.length) {
    segments.push({
      kind: "html",
      html: renderHtml(contentMd.slice(cursor)),
    });
  }
  return segments;
}

const props = defineProps<{
  /** 章节 markdown 正文全文（空串 = 有意义的空正文，渲染空态提示） */
  contentMd: string;
}>();

const segments = computed<ContentSegment[]>(() =>
  props.contentMd.trim() ? toSegments(props.contentMd) : [],
);
</script>

<template>
  <div v-if="segments.length" class="space-y-3">
    <template v-for="(segment, i) in segments" :key="i">
      <!-- html 段：marked 产出已经 DOMPurify 消毒，才允许落 v-html -->
      <!-- eslint-disable-next-line vue/no-v-html：DOMPurify 消毒红线已过 -->
      <div
        v-if="segment.kind === 'html'"
        class="markdown-code max-w-none text-[15px] leading-[1.9] text-foreground/90 [&_h1]:mb-2 [&_h1]:mt-4 [&_h1]:text-base [&_h1]:font-semibold [&_h1]:text-foreground [&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-foreground [&_h3]:mb-1 [&_h3]:mt-3 [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-foreground [&_img]:h-auto [&_img]:max-w-full [&_img]:rounded [&_li]:my-1 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-2.5 [&_strong]:font-semibold [&_strong]:text-foreground [&_ul]:list-disc [&_ul]:pl-6 [&>p]:indent-[2em]"
        v-html="segment.html"
      />
      <MermaidDiagram v-else :code="segment.code" />
    </template>
  </div>
  <p v-else class="text-sm text-muted-foreground">
    正文尚未生成（可重新生成此章）。
  </p>
</template>
