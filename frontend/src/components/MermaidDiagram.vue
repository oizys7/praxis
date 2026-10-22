<script lang="ts">
/**
 * mermaid 动态加载（模块级单例）：import("mermaid") 不进主 bundle，
 * 首次渲染时按需加载；initialize 幂等（合并配置），只在首次加载成功后执行一次。
 */
let mermaidLoader: Promise<typeof import("mermaid")> | null = null;

function loadMermaid(): Promise<typeof import("mermaid")> {
  if (!mermaidLoader) {
    mermaidLoader = import("mermaid")
      .then((mod) => {
        mod.default.initialize({ startOnLoad: false, theme: "default" });
        return mod;
      })
      .catch((err: unknown) => {
        // 失败不缓存：否则一次分包/网络抖动会让本会话后续所有图永久降级为源码
        mermaidLoader = null;
        throw err;
      });
  }
  return mermaidLoader;
}

/** 渲染容器 id 序列（mermaid.render 要求文档内唯一 id） */
let renderSeq = 0;

/** 提示文案 id 序列（同页多个图时 aria-describedby 必须唯一） */
let hintSeq = 0;
</script>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";

/**
 * MermaidDiagram：渲染 markdown 正文内联 ```mermaid 码块（MarkdownContent 切段后传入）。
 * 渲染失败（语法错误 / 动态加载失败）不阻断页面，降级为 <pre> 显示源码。
 *
 * 交互（2026-10-22 增强）：图上可缩放与平移，用于看清复杂流程图的分支细节。
 * - 拖拽平移：仅在放大后启用（缩放 ≤100% 时拖动没有意义，反而容易误操作）；
 * - 缩放：工具条 −/+ 按钮、Ctrl/⌘ + 滚轮、键盘 +/-/0（视口可聚焦，故键盘可达）；
 * - 双击或「重置」回到 100% 居中。
 *
 * 用 transform 做缩放而不是改 SVG 宽高：transform 不参与布局，因此放大后的
 * 内容会被视口裁剪（可拖拽查看），不会撑开文档流导致正文跳动。
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
/** 本实例的键盘/缩放提示 id（aria-describedby 目标） */
const hintId = `mermaid-zoom-hint-${++hintSeq}`;

async function render(): Promise<void> {
  failed.value = false;
  svg.value = null;
  reset();
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

// ---------------- 缩放 / 平移 ----------------

const MIN_SCALE = 0.5;
const MAX_SCALE = 4;
const BUTTON_STEP = 1.25;
/** 按钮/键盘缩放时给 transform 加过渡，拖拽时则直连（避免手感滞后） */
const TRANSITION_MS = 180;

const scale = ref(1);
const offsetX = ref(0);
const offsetY = ref(0);
const dragging = ref(false);
const animated = ref(false);

const zoomPercent = computed(() => `${Math.round(scale.value * 100)}%`);
/** 放大后才允许拖动，避免默认状态下误拖走图 */
const pannable = computed(() => scale.value > 1);

const canvasStyle = computed(() => ({
  transform: `translate(${offsetX.value}px, ${offsetY.value}px) scale(${scale.value})`,
  // 以中心为原点：缩放围绕图心，符合直觉，平移量保持屏幕像素不被缩放放大
  transformOrigin: "center center",
  transition: animated.value
    ? `transform ${TRANSITION_MS}ms cubic-bezier(0.32, 0.72, 0, 1)`
    : "none",
}));

function clampScale(value: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

function withTransition(mutate: () => void): void {
  animated.value = true;
  mutate();
  window.setTimeout(() => {
    animated.value = false;
  }, TRANSITION_MS);
}

function zoomBy(factor: number): void {
  withTransition(() => {
    scale.value = clampScale(scale.value * factor);
    if (!pannable.value) {
      // 回到 100% 及以下时复位平移，避免留下残余偏移
      offsetX.value = 0;
      offsetY.value = 0;
    }
  });
}

function reset(): void {
  withTransition(() => {
    scale.value = 1;
    offsetX.value = 0;
    offsetY.value = 0;
  });
}

let activePointer: number | null = null;
let startClientX = 0;
let startClientY = 0;
let startOffsetX = 0;
let startOffsetY = 0;

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || !pannable.value) return;
  activePointer = event.pointerId;
  dragging.value = true;
  startClientX = event.clientX;
  startClientY = event.clientY;
  startOffsetX = offsetX.value;
  startOffsetY = offsetY.value;
  // 指针捕获：拖出视口边界后仍能收到 move/up，松手不会「粘住」
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  event.preventDefault();
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value || event.pointerId !== activePointer) return;
  offsetX.value = startOffsetX + (event.clientX - startClientX);
  offsetY.value = startOffsetY + (event.clientY - startClientY);
}

function endDrag(event: PointerEvent): void {
  if (event.pointerId !== activePointer) return;
  activePointer = null;
  dragging.value = false;
}

function onWheel(event: WheelEvent): void {
  // 普通滚轮留给页面滚动；只有按住 Ctrl/⌘ 才缩放（与地图、图片查看器一致）
  if (!event.ctrlKey && !event.metaKey) return;
  event.preventDefault();
  animated.value = false;
  const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
  scale.value = clampScale(scale.value * factor);
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "+" || event.key === "=") {
    event.preventDefault();
    zoomBy(BUTTON_STEP);
  } else if (event.key === "-") {
    event.preventDefault();
    zoomBy(1 / BUTTON_STEP);
  } else if (event.key === "0") {
    event.preventDefault();
    reset();
  }
}

onMounted(render);
watch(() => props.code, render);
</script>

<template>
  <figure class="group relative rounded-lg border bg-muted/30 p-3">
    <!-- 工具条：悬停或键盘聚焦时出现（focus-within 保证键盘用户也能到达） -->
    <div
      v-if="svg"
      class="absolute right-2 top-2 z-10 flex items-center gap-0.5 rounded-md border bg-popover/95 p-1 opacity-0 shadow-sm backdrop-blur transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100"
    >
      <button
        type="button"
        class="flex h-6 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        aria-label="缩小"
        @click="zoomBy(1 / BUTTON_STEP)"
      >
        <span aria-hidden="true" class="text-sm leading-none">−</span>
      </button>
      <span
        class="w-10 text-center text-[11px] tabular-nums text-muted-foreground"
        aria-hidden="true"
        >{{ zoomPercent }}</span
      >
      <button
        type="button"
        class="flex h-6 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        aria-label="放大"
        @click="zoomBy(BUTTON_STEP)"
      >
        <span aria-hidden="true" class="text-sm leading-none">+</span>
      </button>
      <span aria-hidden="true" class="mx-0.5 h-4 w-px bg-border" />
      <button
        type="button"
        class="flex h-6 items-center rounded px-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        aria-label="重置为 100% 并居中"
        @click="reset"
      >
        重置
      </button>
    </div>

    <!-- 渲染成功：注入 SVG（mermaid 样式随 SVG 内嵌） -->
    <div
      v-if="svg"
      class="flex touch-none items-center justify-center overflow-hidden rounded select-none"
      :class="dragging ? 'cursor-grabbing' : pannable ? 'cursor-grab' : ''"
      tabindex="0"
      role="img"
      :aria-label="props.caption || 'Mermaid 图示'"
      aria-roledescription="可缩放图示"
      :aria-describedby="hintId"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="endDrag"
      @pointercancel="endDrag"
      @dblclick="reset"
      @wheel="onWheel"
      @keydown="onKeydown"
    >
      <!-- eslint-disable-next-line vue/no-v-html：mermaid 产出的受信 SVG -->
      <div
        :style="canvasStyle"
        class="will-change-transform [&_svg]:h-auto [&_svg]:max-w-full"
        v-html="svg"
      />
    </div>

    <!-- 渲染失败 / 动态加载失败：降级显示源码（不阻断页面） -->
    <pre
      v-else-if="failed"
      class="overflow-x-auto rounded bg-muted p-2 text-xs leading-relaxed text-muted-foreground"
    >
图示渲染失败，显示原始源码：
{{ props.code }}</pre>
    <div v-else class="min-h-40 py-6 text-center text-xs text-muted-foreground">
      图示渲染中…
    </div>
    <figcaption
      v-if="props.caption"
      class="mt-2 text-center text-xs text-muted-foreground"
    >
      {{ props.caption }}
    </figcaption>
    <!-- 键盘/缩放说明：放在 v-if 链之外，避免打断 v-else-if 的相邻性 -->
    <p :id="hintId" class="sr-only">
      图示可缩放：按住 Ctrl 加滚轮，或聚焦后按加号、减号缩放，按 0
      重置，放大后可拖拽查看。
    </p>
  </figure>
</template>
