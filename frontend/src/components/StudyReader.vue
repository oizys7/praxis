<script lang="ts">
/**
 * 打字机流状态（父组件 StudyView 维护：全局 chapter-delta 事件驱动）。
 * 放在独立 <script> 块以供父组件 import 复用类型。
 */
export interface StudyStreamState {
  /** 生成请求在飞（乐观置位；含打字机与「续看」两种形态） */
  active: boolean;
  /** 打字机形态：从头渐进揭示；false = 中途进入/已滚出视口的兜底形态 */
  typewriter: boolean;
  /** 已累积的原始 markdown（父侧节流刷新，避免逐 token 重渲染） */
  text: string;
  /** 生成失败信息（非空 = 失败态） */
  error: string | null;
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  BookOpenText,
  Highlighter,
  RotateCcw,
  TriangleAlert,
} from "lucide-vue-next";
import MarkdownContent from "@/components/MarkdownContent.vue";
import { newAnnotationId } from "@/composables/useStudy";
import type { Chapter, ChapterAnnotation } from "@/types/agent";

/**
 * StudyReader：研读页中栏——章节内容阅读与按需生成。
 *
 * 四种状态（互斥，优先级自上而下）：
 * - 未选章：居中提示；
 * - 失败态（stream.error 或 DB failed 章）：错误卡 + 重试；
 * - 打字机态（stream.active && typewriter && 未滚出视口）：纯文本渐进揭示
 *   （rAF 步进 + 光标，零 markdown 解析开销；用户上滚即离场 → 转全量渲染）；
 * - 全量渲染：MarkdownContent（marked + DOMPurify 红线不破）。
 * 流式中但非打字机形态（中途进入 / 用户已滚走）同样走 MarkdownContent，
 * 渲染父侧节流后的累积文本，终态以 get_plan 回读内容为准。
 *
 * 选区标注：document 级 mouseup + selectionchange 统一捕获中栏选区 →
 * 浮动「标注」按钮 → 可选评论保存（quote 必填，comment 可选 = 纯划线）。
 */
const props = defineProps<{
  chapter: Chapter | null;
  phaseIndex: number | null;
  chapterIndex: number | null;
  stream: StudyStreamState;
}>();

const emit = defineEmits<{
  (e: "retry"): void;
  (e: "annotate", annotation: ChapterAnnotation): void;
}>();

const KIND_LABELS: Record<string, string> = {
  concept: "概念",
  practice: "实操",
  project: "项目",
  review: "复习",
};

const kindLabel = computed(
  () => KIND_LABELS[props.chapter?.kind ?? ""] ?? null,
);

/** 权威正文：DB 回读优先；流刚结束、记录刷新前的空窗用累积文本兜底 */
const contentMd = computed(
  () => props.chapter?.contentMd || props.stream.text || "",
);

/** 章节级失败（DB 持久 failed 且无正文，且当前没有在飞的生成） */
const chapterFailed = computed(
  () =>
    !props.stream.active &&
    !props.stream.error &&
    !!props.chapter &&
    props.chapter.status === "failed" &&
    !props.chapter.contentMd,
);

// ---------------- 打字机（rAF 步进 + 滚动离场判定） ----------------

const rootEl = ref<HTMLElement | null>(null);
const shownLen = ref(0);
/** 打字机被用户「滚出可见区」后置位：转为全量渲染累积文本 */
const revealFull = ref(false);
let rafId = 0;
let following = false;

const displayedText = computed(() =>
  props.stream.text.slice(0, shownLen.value),
);

/** 打字机形态是否生效（未被离场判定接管） */
const showTypewriter = computed(
  () => props.stream.active && props.stream.typewriter && !revealFull.value,
);

function stopTypewriterLoop() {
  if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
  following = false;
}

/** rAF 主循环：步进揭示 + 跟随滚动（尾点保持可见） */
function typewriterFrame() {
  rafId = 0;
  const target = props.stream.text.length;
  if (shownLen.value < target) {
    const remaining = target - shownLen.value;
    shownLen.value = Math.min(
      target,
      shownLen.value + Math.max(3, Math.ceil(remaining * 0.12)),
    );
    if (following && rootEl.value) {
      rootEl.value.scrollTop = rootEl.value.scrollHeight;
    }
  }
  if (props.stream.active) rafId = requestAnimationFrame(typewriterFrame);
}

/**
 * 滚动处理（二合一）：
 * 1) 选区浮动件随滚动收起（评论浮层除外，输入中不骚扰）；
 * 2) 打字机离场判定——跟随滚动始终把内容底沿钉在可视区底，任何滚动后
 *    「底沿离开视口 32px 以上」只可能是用户主动滚走——此刻停掉打字机与
 *    跟随，转为全量 markdown 渲染（生成继续在后台累积，终态以 DB 回读
 *    为准）。不用 IntersectionObserver：流式增长中尾点在帧间频繁出入
 *    视口，IO 抖动会把正常跟随误判为离场。
 */
function onReaderScroll() {
  if (!annotating.value) pick.value = null;
  if (!following || !rootEl.value) return;
  const el = rootEl.value;
  if (el.scrollTop + el.clientHeight < el.scrollHeight - 32) {
    revealFull.value = true;
    stopTypewriterLoop();
  }
}

watch(
  () => props.stream.active,
  (active) => {
    stopTypewriterLoop();
    revealFull.value = false;
    if (active) {
      shownLen.value = 0;
      following = true;
      rafId = requestAnimationFrame(typewriterFrame);
    } else {
      shownLen.value = props.stream.text.length;
    }
  },
);

onMounted(() => {
  rootEl.value?.addEventListener("scroll", onReaderScroll, { passive: true });
  document.addEventListener("mouseup", onDocMouseup);
  document.addEventListener("selectionchange", onDocSelectionChange);
});
onBeforeUnmount(() => {
  stopTypewriterLoop();
  rootEl.value?.removeEventListener("scroll", onReaderScroll);
  if (selectionChangeTimer) {
    clearTimeout(selectionChangeTimer);
    selectionChangeTimer = 0;
  }
  teardownSelectionListeners();
});

// ---------------- 选区标注 ----------------

/** 浮动「标注」按钮位置与引用文本（null = 无有效选区） */
const pick = ref<{ x: number; y: number; quote: string } | null>(null);
/** 评论输入浮层 */
const annotating = ref(false);
const comment = ref("");

/** 选区引用文本上限：后端 quote 列有长度约束，超长截断保序 */
const QUOTE_MAX = 500;

/** 气泡几何估算（按钮 min-h-9 + 余量），用于避障判定与视口钳制 */
const BUBBLE_HALF = 60;
const BUBBLE_HEIGHT = 40;

/**
 * 气泡定位：默认悬在选区上方；若会压住章节头或正文标题（h2–h4），
 * 或顶部放不下，翻到选区下方（评审 P2：气泡曾盖住「本章要点」）。
 */
function bubblePosition(rect: DOMRect): { x: number; y: number } {
  const x = Math.min(
    Math.max(rect.left + rect.width / 2, 72),
    window.innerWidth - 72,
  );
  const collides = (y: number) => {
    const box = {
      left: x - BUBBLE_HALF,
      right: x + BUBBLE_HALF,
      top: y,
      bottom: y + BUBBLE_HEIGHT,
    };
    const overlaps = (r: DOMRect) =>
      !(
        r.right <= box.left ||
        r.left >= box.right ||
        r.bottom <= box.top ||
        r.top >= box.bottom
      );
    return [
      ...(rootEl.value?.querySelectorAll<HTMLElement>("header, h2, h3, h4") ??
        []),
    ].some((el) => overlaps(el.getBoundingClientRect()));
  };
  const above = Math.max(rect.top - 46, 8);
  const below = rect.bottom + 10;
  if (!collides(above)) return { x, y: above };
  if (below + BUBBLE_HEIGHT <= window.innerHeight - 8 && !collides(below)) {
    return { x, y: below };
  }
  return { x, y: above };
}

/**
 * 从当前窗口选区解析「标注」气泡（无有效选区即收起）。
 * 只响应锚点在中栏内部的选区；跨元素拖拽选区同样成立。
 */
function evaluateSelection() {
  if (annotating.value) return;
  const selection = window.getSelection();
  const quote = (selection?.toString() ?? "").trim();
  if (!selection || selection.isCollapsed || !quote) {
    pick.value = null;
    return;
  }
  const anchor = selection.anchorNode;
  if (!anchor || !rootEl.value?.contains(anchor)) {
    pick.value = null;
    return;
  }
  const rect = selection.getRangeAt(0).getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) {
    pick.value = null;
    return;
  }
  pick.value = {
    ...bubblePosition(rect),
    quote: quote.length > QUOTE_MAX ? `${quote.slice(0, QUOTE_MAX)}…` : quote,
  };
}

/** 浮动按钮/评论浮层自身的 mouseup 不参与选区捕获（否则点「标注」会先清选区） */
function onDocMouseup(event: MouseEvent) {
  const button = event.target as HTMLElement | null;
  if (button?.closest("[data-study-annotate], [data-study-annotate-pop]")) {
    return;
  }
  evaluateSelection();
}

/**
 * 选区监听统一收口在 document：拖拽跨行/跨段选区常在阅读器外松开鼠标
 * （拖出中栏或窗口边缘自动滚动），section 自身的 mouseup 收不到，入口就
 * 丢了。selectionchange 兜底键盘选区（Shift+方向键）；节流避免拖拽中途
 * 高频触发，落点在拖拽结束后一帧。
 */
let selectionChangeTimer = 0;

function onDocSelectionChange() {
  if (annotating.value) return;
  if (selectionChangeTimer) clearTimeout(selectionChangeTimer);
  selectionChangeTimer = window.setTimeout(() => {
    selectionChangeTimer = 0;
    evaluateSelection();
  }, 180);
}

function openAnnotate() {
  if (!pick.value) return;
  annotating.value = true;
  comment.value = "";
}

function cancelAnnotate() {
  annotating.value = false;
  pick.value = null;
}

function saveAnnotate() {
  if (!pick.value || props.phaseIndex === null || props.chapterIndex === null) {
    cancelAnnotate();
    return;
  }
  emit("annotate", {
    id: newAnnotationId(),
    phaseIndex: props.phaseIndex,
    chapterIndex: props.chapterIndex,
    quote: pick.value.quote,
    comment: comment.value.trim(),
    createdAt: new Date().toISOString(),
  });
  annotating.value = false;
  pick.value = null;
  window.getSelection()?.removeAllRanges();
}

/** 浮层打开期间：点击别处即收起（滚动收起走 onReaderScroll 的统一处理） */
function onDocPointerDown(event: PointerEvent) {
  if (!pick.value && !annotating.value) return;
  const target = event.target as HTMLElement | null;
  // 浮层与触发按钮内部的按压不算「点别处」
  if (target?.closest("[data-study-annotate-pop], [data-study-annotate]")) {
    return;
  }
  if (annotating.value) {
    // 评论浮层打开时点外部：关闭浮层（含清选区状态）
    cancelAnnotate();
    return;
  }
  pick.value = null;
}

function teardownSelectionListeners() {
  document.removeEventListener("pointerdown", onDocPointerDown, true);
  document.removeEventListener("mouseup", onDocMouseup);
  document.removeEventListener("selectionchange", onDocSelectionChange);
}

watch(annotating, (open) => {
  if (open) {
    document.addEventListener("pointerdown", onDocPointerDown, true);
  } else {
    document.removeEventListener("pointerdown", onDocPointerDown, true);
  }
});

// ---------------- 标注定位（右栏点击条目 → 滚动到原文并闪烁） ----------------

/**
 * 跨元素选区的引用定位：选区文本会夹入换行，且加粗/行内代码会把一句话
 * 切成多个文本节点，逐字面匹配必然漏配。做法：把根内全部文本节点拼成
 * 去空白长串并记录每个字符的来源节点，再依次尝试「整段规范化 quote →
 * 各行 ≥6 字规范化片段」，任一命中即把命中起点所在块滚入视口并闪烁。
 */
function locateQuote(quote: string) {
  const root = rootEl.value;
  if (!root || !quote.trim()) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const owners: Text[] = [];
  let haystack = "";
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? "";
    // 按码元遍历（不用 for-of）：owners 与 haystack 必须逐字符对齐
    for (let i = 0; i < text.length; i++) {
      if (/\s/.test(text[i]!)) continue;
      owners.push(node as Text);
      haystack += text[i];
    }
  }
  const segments = [quote.replace(/\s+/g, "")];
  for (const line of quote.split(/\r?\n/)) {
    const norm = line.replace(/\s+/g, "");
    if (norm.length >= 6) segments.push(norm);
  }
  let at = -1;
  for (const segment of segments) {
    at = haystack.indexOf(segment);
    if (at >= 0) break;
  }
  if (at < 0) return;
  const el = owners[at]?.parentElement;
  if (!el) return;
  const block =
    el.closest<HTMLElement>("p, h2, h3, h4, li, pre, blockquote, td") ?? el;
  block.scrollIntoView({ block: "center", behavior: "instant" });
  block.classList.remove("annotation-flash");
  // 强制重排重启 CSS 动画（同一元素连续定位时闪烁可复现）
  void block.offsetWidth;
  block.classList.add("annotation-flash");
  window.setTimeout(() => block.classList.remove("annotation-flash"), 1800);
}

/** 评论浮层定位（模板不可直接访问 window，视口钳制在脚本内完成） */
const popStyle = computed(() => {
  const x = pick.value ? pick.value.x : 0;
  const y = pick.value ? pick.value.y : 0;
  return {
    left: `${Math.min(Math.max(x - 160, 8), window.innerWidth - 328)}px`,
    top: `${Math.max(y - 8, 8)}px`,
  };
});

defineExpose({ locateQuote });
</script>

<template>
  <section
    ref="rootEl"
    data-study-reader
    class="scroll-stable min-h-0 min-w-0 flex-1 overflow-y-auto"
  >
    <div class="mx-auto w-full max-w-[760px] px-5 py-8 sm:px-8">
      <!-- 未选章空态 -->
      <div
        v-if="!chapter"
        class="flex flex-col items-center gap-3 py-24 text-center"
      >
        <BookOpenText class="h-10 w-10 text-primary/40" aria-hidden="true" />
        <p class="font-medium">从左侧选择一章开始研读</p>
        <p class="max-w-sm text-sm text-muted-foreground">
          空章节会在选中后自动按需生成正文；已生成章节直接进入阅读。
        </p>
      </div>

      <template v-else>
        <!-- 章节头：标题 + 元信息（quiet，不抢正文） -->
        <header class="border-b pb-5">
          <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <h2 class="font-display text-2xl font-semibold leading-snug">
              {{ chapter.title }}
            </h2>
            <span v-if="kindLabel" class="text-xs text-muted-foreground">
              {{ kindLabel }}
            </span>
          </div>
          <div
            v-if="chapter.week != null || chapter.estimatedHours != null"
            class="mt-2 flex items-center gap-2 text-xs text-muted-foreground"
          >
            <span
              v-if="chapter.week != null"
              class="rounded-md bg-secondary-2 px-1.5 py-0.5"
              >第 {{ chapter.week }} 周</span
            >
            <span v-if="chapter.estimatedHours != null"
              >预计 ≈ {{ chapter.estimatedHours }}h</span
            >
          </div>
          <p
            v-if="chapter.summary"
            class="mt-2 text-sm leading-relaxed text-muted-foreground"
          >
            {{ chapter.summary }}
          </p>
        </header>

        <!-- 失败态：本次生成失败或章节持久 failed，可重试 -->
        <div
          v-if="stream.error || chapterFailed"
          class="mt-8 rounded-lg border border-destructive/40 bg-destructive-1 px-4 py-5 text-center"
          role="alert"
        >
          <TriangleAlert
            class="mx-auto h-6 w-6 text-destructive"
            aria-hidden="true"
          />
          <p class="mt-2 text-sm font-medium text-destructive">
            本章内容生成失败
          </p>
          <p
            v-if="stream.error"
            class="mt-1 break-all text-xs text-muted-foreground"
          >
            {{ stream.error }}
          </p>
          <p v-else class="mt-1 text-xs text-muted-foreground">
            上一次生成未完成，可重试。
          </p>
          <button
            type="button"
            data-study-retry
            class="mt-3 inline-flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-3 text-sm transition-colors hover:border-primary/40 hover:text-primary"
            @click="emit('retry')"
          >
            <RotateCcw class="h-3.5 w-3.5" aria-hidden="true" />
            重试生成
          </button>
        </div>

        <!-- 打字机态：纯文本渐进揭示（rAF），零 markdown 解析开销 -->
        <div v-else-if="showTypewriter" class="mt-7" data-study-typewriter>
          <p
            v-if="stream.text"
            class="whitespace-pre-wrap break-words text-foreground/90"
            :style="{
              fontSize: 'var(--reader-font-size, 17px)',
              lineHeight: 'var(--reader-line-height, 1.95)',
            }"
          >
            {{ displayedText }}<span class="study-caret" aria-hidden="true" />
          </p>
          <p
            v-else
            class="flex items-center gap-2 text-sm text-muted-foreground"
          >
            <span
              class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary border-t-transparent"
            />
            Agent 正在生成本章内容…
          </p>
        </div>

        <!-- 生成中（非打字机：中途进入 / 已滚出视口 / 尾段空窗） -->
        <div v-else-if="stream.active" class="mt-7">
          <p class="flex items-center gap-2 text-sm text-muted-foreground">
            <span
              class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary border-t-transparent"
            />
            正在生成本章内容…
          </p>
          <div v-if="stream.text" data-study-content class="mt-5 opacity-80">
            <MarkdownContent :content-md="stream.text" />
          </div>
        </div>

        <!-- 全量渲染：marked + DOMPurify（权威正文） -->
        <div v-else-if="contentMd" data-study-content class="reader-body mt-7">
          <MarkdownContent :content-md="contentMd" />
        </div>

        <!-- 空章节 & 无在飞请求：极短窗口（选中即自动发起） -->
        <p v-else class="mt-8 text-sm text-muted-foreground">
          本章尚未生成内容，正在准备…
        </p>
      </template>
    </div>

    <!-- 选区浮动「标注」按钮 -->
    <div
      v-if="pick && !annotating"
      class="fixed z-50 -translate-x-1/2"
      :style="{ left: `${pick.x}px`, top: `${pick.y}px` }"
    >
      <button
        type="button"
        data-study-annotate
        class="flex min-h-9 items-center gap-1.5 rounded-full border bg-popover px-3 text-xs font-medium shadow-lg transition-colors hover:border-primary/40 hover:text-primary"
        @click="openAnnotate"
      >
        <Highlighter class="h-3.5 w-3.5" aria-hidden="true" />
        标注
      </button>
    </div>

    <!-- 标注评论浮层：quote 必填已带，comment 可选（纯划线直接保存） -->
    <div
      v-if="annotating && pick"
      data-study-annotate-pop
      class="fixed z-50 w-80 max-w-[calc(100vw-2rem)] rounded-xl border bg-popover p-3 shadow-xl"
      :style="popStyle"
    >
      <p
        class="max-h-24 overflow-y-auto break-words rounded-lg bg-muted px-2.5 py-2 text-xs leading-relaxed text-muted-foreground"
      >
        {{ pick.quote }}
      </p>
      <textarea
        v-model="comment"
        rows="3"
        aria-label="标注评论（可选）"
        placeholder="写点想法（可选，直接保存即为纯划线）"
        class="mt-2 w-full resize-none rounded-lg border bg-card px-2.5 py-2 text-sm outline-none focus:ring-2 focus:ring-ring"
        @keydown.esc.stop.prevent="cancelAnnotate"
      />
      <div class="mt-2 flex justify-end gap-2">
        <button
          type="button"
          class="min-h-8 rounded-lg px-3 text-xs text-muted-foreground hover:bg-muted"
          @click="cancelAnnotate"
        >
          取消
        </button>
        <button
          type="button"
          data-study-annotate-save
          class="min-h-8 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground hover:bg-primary-4"
          @click="saveAnnotate"
        >
          保存标注
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* 打字机光标：随正文行高呼吸，不出现在读屏输出（aria-hidden） */
.study-caret {
  display: inline-block;
  width: 2px;
  height: 1.05em;
  margin-left: 2px;
  vertical-align: -0.15em;
  background: hsl(var(--primary));
  animation: praxis-caret-blink 0.9s steps(1) infinite;
}
@keyframes praxis-caret-blink {
  50% {
    opacity: 0;
  }
}
</style>
