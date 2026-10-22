import { createApp } from "vue";
// .vue 模块解析依赖 src/env.d.ts 里的 `declare module "*.vue"`。vue-tsc 自带
// SFC 解析所以不需要它，但普通 tsserver / 裸 tsc 会报 TS2307「找不到模块」——
// 删掉那条声明会让所有非 SFC-aware 工具重新报错。
import App from "./App.vue";
import vTip from "./directives/tip";
import "./assets/index.css";

/** 主题持久化键（'dark' | 'light'；与 App 顶栏切换按钮共用） */
const THEME_STORAGE_KEY = "praxis-theme";

/**
 * 挂载前同步初始化全局主题：读 localStorage（'dark' | 'light' |
 * 缺省跟随系统 prefers-color-scheme），对 documentElement 挂/除
 * dark class（tailwind darkMode: ["class"] + index.css .dark 令牌组）。
 * 必须先于 mount 同步执行，避免首帧亮暗闪烁。
 */
function initTheme(): void {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  const dark =
    stored === "dark" ||
    (stored !== "light" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
}

// Praxis 前端入口
initTheme();
createApp(App).directive("tip", vTip).mount("#app");
