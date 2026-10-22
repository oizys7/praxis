/// <reference types="vite/client" />

/**
 * Vue SFC 模块声明（2026-10-22 补）。
 *
 * vue-tsc 自带 SFC 解析（项目权威判据，跑 `vue-tsc --noEmit` 恒 0 错误），
 * 但普通 TypeScript 语言服务与裸 `tsc` 不认识 `.vue`，会把
 * `import App from "./App.vue"` 判成「找不到模块」——每次 LSP 扫描都会
 * 重新报一遍，长期被当成「已知误报」绕过。这里补上声明，让两类工具都能
 * 工作；vue-tsc 仍优先走自己的 SFC 解析，类型检查严格度不变。
 *
 * 泛型用宽松形态是刻意的：本声明只为让模块可解析，props/emits 的真实
 * 类型由 vue-tsc 从 SFC 源码推出，不依赖这里。
 */
declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<
    Record<string, unknown>,
    Record<string, unknown>,
    unknown
  >;
  export default component;
}
