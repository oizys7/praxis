#!/usr/bin/env node
/**
 * scripts/build.mjs —— 生产构建（三端覆盖：Python / 前端 / Rust）
 *
 * 步骤：
 *   1. Python 健全性：compileall 语法编译（解释型语言无构建产物，确保可运行）
 *   2. 前端构建：vue-tsc 类型检查 + vite build（产物 frontend/dist）
 *   3. Agent sidecar：PyInstaller 打包内置 Agent 服务（PRAXIS_SKIP_AGENT=1 跳过）
 *   4. Rust 编译 + 桌面打包：tauri build（内含 cargo release 编译，
 *      产物 src-tauri/target/release/bundle）
 *
 * 注意：打包安装包前需要应用图标（仓库已内置，见 README「打包发布」）。
 *
 * 用法：在仓库根目录执行 `npm run build`。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** 运行一个构建步骤，失败即终止 */
function step(title, command, args, opts = {}) {
  console.log(`\n[build] ▶ ${title}`);
  const result = spawnSync(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    shell: true,
    ...opts,
  });
  if (result.status !== 0) {
    console.error(`[build] ✗ ${title} 失败`);
    process.exit(1);
  }
  console.log(`[build] ✓ ${title}`);
}

// 1. Python：构建前语法健全性检查（编译通过 = 无语法错误）
step("Python 语法健全性（compileall）", "python", [
  "-m",
  "compileall",
  "-q",
  "agent-service/app",
]);

// 2. 前端：类型检查 + 打包
step("构建前端（vue-tsc + vite build）", "npm", [
  "--prefix",
  "frontend",
  "run",
  "build",
]);

// 3. Agent sidecar：PyInstaller 把 agent-service 打成独立可执行目录
//    （dist/praxis-agent/，经 tauri resources 进安装包），供 release 版
//    praxis.exe 作为子进程拉起。PRAXIS_SKIP_AGENT=1 可跳过（调试桌面壳时）。
if (process.env.PRAXIS_SKIP_AGENT === "1") {
  console.log("\n[build] ⏭ 跳过 Agent sidecar 打包（PRAXIS_SKIP_AGENT=1）");
} else {
  step(
    "打包 Agent sidecar（PyInstaller）",
    "uv",
    [
      "run",
      "pyinstaller",
      "--name",
      "praxis-agent",
      "--noconfirm",
      "--clean",
      "--onedir",
      "agent_entry.py",
    ],
    { cwd: path.join(ROOT, "agent-service") },
  );
}

// 4. Rust：tauri build 内含 cargo release 编译 + 平台安装包生成。
//    tauri-cli 必须从仓库根运行（原因同 scripts/dev.mjs），
//    直接调用 frontend/node_modules/.bin 下的 cli。
const tauriBin = path.join(
  "frontend",
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);

step("打包桌面应用（tauri build，含 Rust release 编译）", `"${tauriBin}"`, [
  "build",
]);

console.log(
  "\n[build] 三端构建完成 ✓  安装包位于 src-tauri/target/release/bundle/",
);
