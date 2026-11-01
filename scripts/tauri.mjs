#!/usr/bin/env node
/**
 * scripts/tauri.mjs —— 从仓库根转发 tauri-cli 命令
 *
 * 为什么需要它：tauri-cli 只在「当前工作目录及其子目录」中查找 tauri.conf.json。
 * 在 frontend/ 里运行 `npm run tauri dev` 时 cwd 是 frontend/，而 src-tauri/ 是
 * 它的兄弟目录，cli 找不到配置直接 panic。此脚本固定以仓库根为 cwd，调用
 * frontend/node_modules/.bin 下的 tauri-cli。
 *
 * 用法：npm run tauri -- <tauri 子命令>，例如：
 *   npm run tauri -- dev
 *   npm run tauri -- icon path/to/app-icon.png
 *   npm run tauri -- build
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const bin = path.join(
  ROOT,
  "frontend",
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);

// 引号包裹：路径可能含空格；shell:true 让 cmd/sh 负责 .cmd 解析
const result = spawnSync(`"${bin}"`, process.argv.slice(2), {
  cwd: ROOT,
  stdio: "inherit",
  shell: true,
});

process.exit(result.status ?? 1);
