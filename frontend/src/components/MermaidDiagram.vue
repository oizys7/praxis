<script lang="ts">
/**
 * mermaid 动态加载（模块级单例）：import("mermaid") 不进主 bundle，
 * 首次渲染时按需加载；initialize 幂等（合并配置），只在首次加载成功后执行一次。
 */
type MermaidThemeKey = "light" | "dark";

/**
 * base 主题 + 项目色板：mermaid 的 default 主题是淡紫节点/黑盒连线，与全局
 * 设计语言不符；dark 内置主题同理偏灰紫。统一用 base 主题，themeVariables
 * 不写死色值——initialize 时从 index.css 的设计令牌/色阶变量实时读取，
 * 调令牌即全站（含图示）一起换肤。
 */
const MERMAID_FONT =
  "'Segoe UI','PingFang SC','Microsoft YaHei',system-ui,sans-serif";

/** 读取一个 HSL 通道令牌并转为 mermaid 可解析的经典逗号写法 */
function hslVar(name: string): string {
  const [h, s, l] = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim()
    .split(/\s+/);
  return `hsl(${h}, ${s}, ${l})`;
}

/** 亮暗两套 themeVariables 的令牌映射：1/2 融合端 → 5 强调端 */
function buildMermaidThemeVariables(
  theme: MermaidThemeKey,
): Record<string, string> {
  return {
    background: "transparent",
    fontFamily: MERMAID_FONT,
    fontSize: "14px",
    // 节点：淡阶填充 + 基准描边 + 强调端文字
    primaryColor: hslVar("--primary-1"),
    primaryTextColor: hslVar("--primary-5"),
    primaryBorderColor: hslVar("--primary-3"),
    lineColor: hslVar(theme === "dark" ? "--primary-2" : "--primary-3"),
    // 辅助面/簇/便签：沙框族（暗色走次级面深阶）
    secondaryColor: hslVar("--secondary-2"),
    tertiaryColor: hslVar("--secondary-1"),
    clusterBkg: hslVar("--muted-2"),
    clusterBorder: hslVar("--muted-5"),
    edgeLabelBackground: hslVar("--secondary-1"),
    noteBkgColor: hslVar("--secondary-2"),
    noteTextColor: hslVar(theme === "dark" ? "--primary-5" : "--foreground"),
    noteBorderColor: hslVar("--muted-5"),
    titleColor: hslVar(theme === "dark" ? "--primary-5" : "--foreground"),
  };
}

let mermaidLoader: Promise<typeof import("mermaid")> | null = null;
/** 当前已生效的 mermaid 主题键：明暗切换时重新 initialize（render 前调用） */
let initializedTheme: MermaidThemeKey | null = null;

/**
 * 按主题加载 mermaid。初始化可重复调用：只有主题变化时才重新 initialize，
 * 已渲染过的图由组件在主题变化 watch 中重新 render。
 */
function loadMermaid(theme: MermaidThemeKey): Promise<typeof import("mermaid")> {
  if (!mermaidLoader) {
    mermaidLoader = import("mermaid")
      .then((mod) => mod)
      .catch((err: unknown) => {
        // 失败不缓存：否则一次分包/网络抖动会让本会话后续所有图永久降级为源码
        mermaidLoader = null;
        initializedTheme = null;
        throw err;
      });
  }
  return mermaidLoader.then((mod) => {
    if (initializedTheme !== theme) {
      mod.default.initialize({
        startOnLoad: false,
        theme: "base",
        themeVariables: buildMermaidThemeVariables(theme),
      });
      initializedTheme = theme;
    }
    return mod;
  });
}

/** 渲染容器 id 序列（mermaid.render 要求文档内唯一 id） */
let renderSeq = 0;

/** 提示文案 id 序列（同页多个图时 aria-describedby 必须唯一） */
let hintSeq = 0;

/**
 * 引号化修复：mermaid 只认 "..." 包裹的节点文本里的 ( ) 等特殊字符，而
 * LLM 产出偶尔是 `G[t.start\(\)]` 这类转义写法（词法不认，实测解析报错）。
 * 首次渲染失败后把含括号的 [..] / {..} 文本段统一改成 [".."] / {".."}。
 * 圆柱 [( )] / 体育场 ([ ]) 等特殊形状含括号属形状语法，整行跳过；
 * 无法修复（没有可引号化的段）时返回 null，调用方走原样降级。
 */
function repairMermaidSource(source: string): string | null {
  let changed = false;
  const repaired = source
    .split("\n")
    .map((line) => {
      // 圆柱 [( )] / 体育场 ([ ]) 的形状开合序列同时出现才整行跳过——
      // 单独的 ")]"（如 t.start\(\)] 行尾）不能误判为形状语法
      if (/\[\(.*\)\]|(\(\[.*\]\))/.test(line)) return line;
      return line.replace(
        /(\[|\{)([^"\][{}]*)(\]|\})/g,
        (whole, open, text, close) => {
          if (!/[()]/.test(text)) return whole;
          changed = true;
          const cleaned = text
            .replace(/\\([()])/g, "$1")
            .replace(/"/g, "'");
          return `${open}"${cleaned}"${close}`;
        },
      );
    })
    .join("\n");
  return changed ? repaired : null;
}
</script>

<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";

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

/** 观测容器（根 figure） */
const rootEl = ref<HTMLElement | null>(null);
/** 图视口（固定高度的画布窗口，缩放/平移都在它里面发生） */
const viewportEl = ref<HTMLElement | null>(null);
/**
 * 是否已进入视口 —— 懒渲染开关。
 *
 * 一份 27 章的计划里有 40+ 个 mermaid 图。挂载即渲染时，mermaid 的布局计算
 * 与 SVG 生成会把主线程占满数秒（单图几十到几百毫秒，逐图串行），而用户
 * 首屏只看得到 1-3 个。改为进入视口才渲染：首屏只付首屏的钱，滚动到哪渲染
 * 到哪；mermaid 那个 684KB 的分包也随之延后到真正需要时。
 */
const visible = ref(false);
let observer: IntersectionObserver | null = null;

/** 当前应用是否暗色（跟随 documentElement.dark，主题切换时重渲染） */
const isDark = ref(document.documentElement.classList.contains("dark"));
let themeObserver: MutationObserver | null = null;

/** 渲染代次：主题/源码快速切换时，旧异步任务不得覆盖新结果 */
let renderToken = 0;

/** 渲染入口：只有进入视口后才真正开始（脚本/schema 变化时也走这里） */
function renderIfVisible() {
  if (visible.value) void render();
}

async function render(): Promise<void> {
  const token = ++renderToken;
  failed.value = false;
  svg.value = null;
  canvasHeight.value = null;
  if (!props.code.trim()) {
    failed.value = true;
    return;
  }
  const id = `praxis-mermaid-${++renderSeq}`;
  try {
    const theme: MermaidThemeKey = isDark.value ? "dark" : "light";
    const mod = await loadMermaid(theme);
    if (token !== renderToken) return;
    let out: string;
    let retryId: string | null = null;
    try {
      ({ svg: out } = await mod.default.render(id, props.code));
    } catch {
      // 先清理首渲染残留占位，再对引号化修复版做一次重试；无可修复段
      // 或修复仍失败则抛给外层降级为源码。
      document.getElementById(`d${id}`)?.remove();
      const repaired = repairMermaidSource(props.code);
      if (!repaired) throw new Error("mermaid render failed, no repair");
      retryId = `praxis-mermaid-fix-${++renderSeq}`;
      ({ svg: out } = await mod.default.render(retryId, repaired));
    }
    if (token !== renderToken) return;
    svg.value = out;
    // 测量前先回到 100% 且关闭过渡：带着上一次 transform 量 rect 会把
    // 倍率二次计入，导致主题切换后图突然缩小/放大。
    animated.value = false;
    scale.value = 1;
    offsetX.value = 0;
    offsetY.value = 0;
    await nextTick();
    if (token !== renderToken) return;
    applyInitialFit();
  } catch {
    // mermaid.render 失败时可能在 body 残留错误占位节点（#d{id}），清理掉
    document.getElementById(`d${id}`)?.remove();
    document
      .querySelectorAll('[id^="dpraxis-mermaid-"]')
      .forEach((node) => node.remove());
    if (token === renderToken) failed.value = true;
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
/**
 * 初始适配倍率（把图放大到高度填满视口）。重置按钮回到这个倍率而不是
 * 死板的 100%——100% 恰恰是用户抱怨「太小看不清」的那个状态。
 */
const fitScale = ref(1);
/** 图在 100% 下就已经超出视口（纵向长图）：此时即使没放大也要允许拖 */
const overflows = ref(false);
/**
 * 画布高度随渲染结果自适应（评审 P2：固定 22rem 视口装 100px 短图，
 * 上下大片留白）。上下限夹在 11rem–22rem：下限保证版式稳定，上限对齐
 * 原固定高度。null = 未渲染（模板回退默认高度）。
 */
const canvasHeight = ref<number | null>(null);
/** 能拖动 = 放大过 或 内容本身就装不下 */
const pannable = computed(() => scale.value > 1 || overflows.value);

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
    scale.value = fitScale.value;
    offsetX.value = 0;
    offsetY.value = 0;
  });
}

/**
 * 初始适配：「等比 contain、不小于 100%、封顶 2.2」。
 *
 * mermaid 的横图（flowchart LR）在图宽被正文列限制住之后高度只有两百上下
 * 像素，字挤成一团——只按高度放大又会把宽度撑出视口、不同图之间倍率忽大
 * 忽小。改为同时受高度与宽度约束：取两轴适配倍率的较小者，封顶 2.2、不缩
 * 小（<1 时保持 100% 并标记 overflows，让用户直接拖）。
 */
const AUTO_FIT_MAX = 2.2;

function applyInitialFit(): void {
  const viewport = viewportEl.value;
  const svgNode = viewport?.querySelector("svg");
  if (!viewport || !svgNode) {
    fitScale.value = 1;
    overflows.value = false;
    return;
  }
  // render 开头已 reset 到 100%，此处量到的是自然布局尺寸
  const rect = svgNode.getBoundingClientRect();
  const naturalHeight = rect.height;
  const naturalWidth = rect.width;
  if (naturalHeight <= 0 || naturalWidth <= 0) {
    fitScale.value = 1;
    overflows.value = false;
    return;
  }
  const axisFit = Math.min(
    viewport.clientHeight / naturalHeight,
    viewport.clientWidth / naturalWidth,
  );
  const next = clampScale(Math.min(AUTO_FIT_MAX, Math.max(1, axisFit)));
  fitScale.value = next;
  scale.value = next;
  offsetX.value = 0;
  offsetY.value = 0;
  overflows.value =
    naturalHeight * next > viewport.clientHeight + 4 ||
    naturalWidth * next > viewport.clientWidth + 4;
  // 画布高度贴合渲染结果（含上下呼吸边距），避免短图居中后大片留白
  canvasHeight.value = Math.round(
    Math.min(352, Math.max(176, naturalHeight * next + 28)),
  );
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

onMounted(() => {
  // 明暗主题切换：SVG 内的文字/描边颜色由 mermaid 主题决定，必须重渲染
  themeObserver = new MutationObserver(() => {
    const next = document.documentElement.classList.contains("dark");
    if (next !== isDark.value) isDark.value = next;
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
  // 没有 IntersectionObserver（旧 webview）或拿不到容器：降级为立即渲染，
  // 宁可慢也不能丢内容
  if (typeof IntersectionObserver === "undefined" || !rootEl.value) {
    visible.value = true;
    renderIfVisible();
    return;
  }
  observer = new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      visible.value = true;
      // 一次性：渲染过后不再需要观察，断开避免滚动时反复回调
      observer?.disconnect();
      observer = null;
      renderIfVisible();
    },
    // 提前 200px 触发：滚到位时图基本已渲染完，不会看到空洞
    { rootMargin: "200px 0px" },
  );
  observer.observe(rootEl.value);
});

onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
  themeObserver?.disconnect();
  themeObserver = null;
});

watch([() => props.code, isDark], renderIfVisible);
</script>

<template>
  <figure ref="rootEl" class="group relative rounded-lg border bg-muted/30 p-3">
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
        aria-label="重置为适配视图并居中"
        @click="reset"
      >
        适配
      </button>
    </div>

    <!-- 渲染成功：注入 SVG（mermaid 样式随 SVG 内嵌） -->
    <div
      v-if="svg"
      ref="viewportEl"
      class="flex touch-none items-center justify-center overflow-hidden rounded select-none"
      :class="dragging ? 'cursor-grabbing' : pannable ? 'cursor-grab' : ''"
      :style="{ height: `${canvasHeight ?? 288}px` }"
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
    <!-- 占位：未进视口（懒渲染）或渲染中。min-h 固定住版式高度，
         避免滚动到位、图渲染出来时把下方内容顶下去 -->
    <div
      v-else
      class="flex min-h-40 items-center justify-center py-6 text-center text-xs text-muted-foreground"
    >
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
