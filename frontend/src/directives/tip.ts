import type { Directive } from "vue";

/**
 * 全局统一 tooltip 指令：v-tip="'文本'"。
 *
 * 替代浏览器原生 title（黑框原生样式丑且延迟不可控）：
 * - 纸面风格深墨蓝气泡（.praxis-tip，见 index.css），跟随明暗主题；
 * - 悬停/聚焦 300ms 后浮出，离开/按下即收，键盘可达；
 * - 优先显示在元素上方，空间不足翻下方，水平钳制在视口内；
 * - 空串/undefined 不启用（如 AppSidebar 展开态不需要提示）。
 *
 * 单例实现：全站共享一个气泡节点，挂载在 body（fixed 定位，
 * 不受宿主 overflow/transform 裁剪影响）。
 */

/** 浮出延迟（ms）：快速滑过不闪框 */
const TIP_DELAY_MS = 300;

/** 气泡与视口边缘的最小间距（px） */
const VIEWPORT_MARGIN = 8;

/** 气泡与宿主元素的间距（px） */
const GAP = 8;

let tipEl: HTMLDivElement | null = null;
let hostEl: HTMLElement | null = null;
let showTimer: number | null = null;

/**
 * 宿主元素 → 当前提示文本。
 *
 * 必须用外部存储而不是闭包里的 `binding`：事件监听器在 mounted 时绑定一次，
 * 而 Vue 每次 updated 传的是**新的 binding 对象**，闭包捕获的旧对象里的
 * value 永远不会更新（展开/收起这类响应式文本会一直读首帧值）。
 * WeakMap 随元素回收，无需在 unmounted 里清理。
 */
const textOf = new WeakMap<HTMLElement, string>();

function ensureEl(): HTMLDivElement {
  if (!tipEl) {
    tipEl = document.createElement("div");
    tipEl.className = "praxis-tip";
    tipEl.setAttribute("role", "tooltip");
    document.body.appendChild(tipEl);
  }
  return tipEl;
}

function hide(): void {
  if (showTimer !== null) {
    window.clearTimeout(showTimer);
    showTimer = null;
  }
  hostEl = null;
  tipEl?.classList.remove("praxis-tip--show");
}

function normalize(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/** 定位并显示气泡（先隐形测量尺寸，再钳制落位，避免首帧闪跳） */
function place(text: string, target: HTMLElement): void {
  const el = ensureEl();
  el.textContent = text;
  el.classList.add("praxis-tip--measure");
  const rect = target.getBoundingClientRect();
  const tw = el.offsetWidth;
  const th = el.offsetHeight;
  // 目标贴近视口左右缘（折叠侧栏/窄屏图标）时水平居中必然被钳到边、
  // 盖住邻近内容：改为在图标侧面垂直居中弹出
  const centeredX = rect.left + rect.width / 2 - tw / 2;
  if (
    centeredX < VIEWPORT_MARGIN ||
    centeredX + tw > window.innerWidth - VIEWPORT_MARGIN
  ) {
    const spaceRight = window.innerWidth - VIEWPORT_MARGIN - (rect.right + GAP);
    const placeRight =
      spaceRight >= tw || spaceRight >= rect.left - VIEWPORT_MARGIN - GAP;
    const x = placeRight ? rect.right + GAP : rect.left - tw - GAP;
    const y = Math.max(
      VIEWPORT_MARGIN,
      Math.min(
        rect.top + rect.height / 2 - th / 2,
        window.innerHeight - th - VIEWPORT_MARGIN,
      ),
    );
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.classList.remove("praxis-tip--measure");
    el.classList.add("praxis-tip--show");
    return;
  }
  let x = centeredX;
  let y = rect.top - th - GAP;
  if (y < VIEWPORT_MARGIN) y = rect.bottom + GAP;
  el.style.left = `${x}px`;
  el.style.top = `${y}px`;
  el.classList.remove("praxis-tip--measure");
  el.classList.add("praxis-tip--show");
}

function schedule(text: string, target: HTMLElement): void {
  hide();
  hostEl = target;
  showTimer = window.setTimeout(() => {
    showTimer = null;
    if (hostEl === target) place(text, target);
  }, TIP_DELAY_MS);
}

const vTip: Directive<HTMLElement, string | undefined | null> = {
  mounted(el, binding) {
    textOf.set(el, normalize(binding.value));
    el.addEventListener("mouseenter", () => {
      const text = textOf.get(el) ?? "";
      if (text) schedule(text, el);
    });
    el.addEventListener("focus", () => {
      const text = textOf.get(el) ?? "";
      if (text) schedule(text, el);
    });
    el.addEventListener("mouseleave", hide);
    el.addEventListener("blur", hide);
    // 点击后立即收起（避免按钮触发的浮层与气泡叠置）
    el.addEventListener("mousedown", hide);
  },
  updated(el, binding) {
    const text = normalize(binding.value);
    textOf.set(el, text);
    if (!text) {
      hide();
    } else if (hostEl === el && tipEl?.classList.contains("praxis-tip--show")) {
      // 显示中文本变化（如状态提示动态更新）：即时重定位
      place(text, el);
    }
  },
  unmounted() {
    // 单例气泡全局仅一份，任一宿主卸载时收起即可（hide 幂等）
    hide();
  },
};

export default vTip;
