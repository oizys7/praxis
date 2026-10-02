#!/usr/bin/env node
/**
 * scripts/format.mjs —— 一键格式化（三端覆盖：Python / Rust / 前端与文档）
 *
 *   1. Python：uvx ruff format（agent-service + scripts/check_prompts.py）
 *   2. Rust：cargo fmt（src-tauri，rustfmt 默认风格）
 *   3. 前端 / 脚本 / Markdown：Prettier
 *
 * 用法：在仓库根目录执行 `npm run format`。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_DIR = path.join(ROOT, "src-tauri");

/** 运行一个格式化步骤，失败即终止 */
function step(title, command, args, opts = {}) {
  console.log(`\n[format] ▶ ${title}`);
  const result = spawnSync(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    shell: true,
    ...opts,
  });
  if (result.status !== 0) {
    console.error(`[format] ✗ ${title} 失败`);
    process.exit(1);
  }
  console.log(`[format] ✓ ${title}`);
}

// 1. Python：uvx 临时运行 ruff（无需预装，uv 自动缓存）
step("Python 格式化（ruff format）", "uvx", [
  "ruff",
  "format",
  "agent-service",
  "scripts/check_prompts.py",
]);

// 2. Rust：rustfmt
step("Rust 格式化（cargo fmt）", "cargo", ["fmt"], { cwd: TAURI_DIR });

// 3. 前端源码 / 前端配置 / 构建脚本 / 根 Markdown
console.log("\n[format] ▶ Prettier（前端 / 脚本 / Markdown）");
const targets = [
  "frontend/src/**/*.{ts,vue,css}",
  "frontend/*.{json,js,ts,html}",
  "scripts/**/*.mjs",
  "*.md",
  "!frontend/dist/**",
];
const result = spawnSync("npx", ["prettier", "--write", ...targets], {
  cwd: ROOT,
  stdio: "inherit",
  shell: true,
});
if (result.status !== 0) {
  console.error("[format] ✗ Prettier 失败");
  process.exit(1);
}
console.log("[format] ✓ Prettier");

console.log("\n[format] 三端格式化完成 ✓");
