#!/usr/bin/env node
/**
 * scripts/build.mjs —— 生产构建
 *
 * 步骤：
 *   1. 前端构建：vue-tsc 类型检查 + vite build（产物 frontend/dist）
 *   2. 桌面打包：tauri build（Rust 编译 + 平台安装包，产物 src-tauri/target/release/bundle）
 *
 * 注意：打包安装包前需要应用图标，见 README「打包发布」一节。
 *
 * 用法：在仓库根目录执行 `npm run build`。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** 运行一个构建步骤，失败即终止 */
function step(title, command, args) {
 console.log(`\n[build] ▶ ${title}`);
 const result = spawnSync(command, args, {
  cwd: ROOT,
  stdio: "inherit",
  shell: true,
 });
 if (result.status !== 0) {
  console.error(`[build] ✗ ${title} 失败`);
  process.exit(1);
 }
 console.log(`[build] ✓ ${title}`);
}

step("构建前端（vue-tsc + vite build）", "npm", [
 "--prefix",
 "frontend",
 "run",
 "build",
]);

// tauri-cli 必须从仓库根运行（原因同 scripts/dev.mjs），
// 直接调用 frontend/node_modules/.bin 下的 cli。
const tauriBin = path.join(
 "frontend",
 "node_modules",
 ".bin",
 process.platform === "win32" ? "tauri.cmd" : "tauri",
);

step("打包桌面应用（tauri build）", `"${tauriBin}"`, ["build"]);

console.log(
 "\n[build] 构建完成 ✓  安装包位于 src-tauri/target/release/bundle/",
);
