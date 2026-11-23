import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * 合并 Tailwind 类名（shadcn/ui 标准工具函数）。
 * clsx 处理条件类名，twMerge 去除冲突类名（后者优先）。
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * 只在页面主滚动容器（main）内定位，不触发根滚动。
 *
 * 原生的 scrollIntoView 会连带滚动 html/body：即使根容器 overflow:hidden，
 * 浏览器仍允许程序化滚动，表现为固定顶栏被顶出 30-40px（阅读器跳目录时
 * 尤其明显）。这里改为按 main 的相对坐标手算 scrollTop。
 */
/** 当前阅读器粘性工具栏的实际高度（字号/窄屏换行会改变它） */
export function readerStickyOffset(extra = 12): number {
  const toolbar = document.querySelector<HTMLElement>(".reader-toolbar");
  return Math.ceil((toolbar?.getBoundingClientRect().height ?? 0) + extra);
}

export function scrollToWithinMain(
  target: HTMLElement | null,
  offset = 0,
): void {
  if (!target) return;
  const host = target.closest<HTMLElement>("main");
  const behavior: ScrollBehavior = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches
    ? "instant"
    : "smooth";
  if (!host) {
    target.scrollIntoView({ behavior, block: "start" });
    return;
  }
  const top =
    target.getBoundingClientRect().top -
    host.getBoundingClientRect().top +
    host.scrollTop -
    offset;
  host.scrollTo({ top: Math.max(0, top), behavior });
}
