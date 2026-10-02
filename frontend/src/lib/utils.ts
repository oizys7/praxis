import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * 合并 Tailwind 类名（shadcn/ui 标准工具函数）。
 * clsx 处理条件类名，twMerge 去除冲突类名（后者优先）。
 */
export function cn(...inputs: ClassValue[]) {
 return twMerge(clsx(inputs));
}
