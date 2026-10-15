#!/usr/bin/env node
/**
 * scripts/dev.mjs —— 一键启动 Praxis 开发环境
 *
 * 同时拉起两个进程：
 *   1. Python Agent 服务：agent_entry.py（内部 uvicorn，--reload 热重载，
 *      目录 agent-service/，优先使用 .venv 虚拟环境解释器，依赖都装在里面；
 *      经 --config 传入仓库根 config.json，与 Rust sidecar 的 CLI 引导同构）
 *   2. Tauri 桌面端：frontend/node_modules/.bin/tauri dev（必须从仓库根运行，
 *      tauri-cli 只在 cwd 子树内查找 tauri.conf.json；其 beforeDevCommand
 *      会自动启动 Vite dev server）
 *
 * 启动前配置预检：仓库根无 config.json 时从 config.example.json 复制模板
 * 并提示按需填写（LLM Key 在应用「设置」页配置，不入 config.json）。
 *
 * 用法：在仓库根目录执行 `npm run dev`，Ctrl+C 一次性停止全部子进程。
 */
import { spawn } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** 记录所有子进程，退出时统一清理 */
const children = [];

/**
 * 跨平台杀掉整个进程树。
 * Windows：taskkill /T 连同子进程一起强制结束；
 * Unix：以 detached 进程组方式启动，向进程组发送 SIGTERM。
 */
function killTree(child) {
  if (!child.pid || child.exitCode !== null) return;
  if (process.platform === "win32") {
    spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      shell: true,
    });
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      child.kill("SIGTERM");
    }
  }
}

/** 启动一个子进程并登记（shell:true 保证 Windows 下能解析 npm/python 等命令） */
function launch(name, command, args, opts = {}) {
  console.log(`[dev] 启动 ${name} → ${command} ${args.join(" ")}`);
  const child = spawn(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    shell: true,
    detached: process.platform !== "win32",
    ...opts,
  });
  child.on("exit", (code) =>
    console.log(`[dev] ${name} 已退出（code=${code}）`),
  );
  children.push(child);
  return child;
}

// ---------- 启动前配置预检 ----------
// config.json（gitignore 覆盖，不入库）是 Python/Rust 共用的冷启动配置；
// 缺失时从模板复制一份（全默认值即可启动），LLM 各家 Key 在应用「设置」页配置。
const configPath = path.join(ROOT, "config.json");
const configExample = path.join(ROOT, "config.example.json");
if (!existsSync(configPath)) {
  if (!existsSync(configExample)) {
    console.error(
      "[dev] 未找到 config.json 与 config.example.json（配置模板），无法启动 Agent 服务。\n" +
        "      请确认仓库版本包含 config.example.json（配置文件化改造交付物）。",
    );
    process.exit(1);
  }
  copyFileSync(configExample, configPath);
  console.warn(
    "[dev] 已从 config.example.json 复制生成 config.json；按需编辑该文件" +
      "（service.host/port、logging.level/dir、agent.mockMode）。\n" +
      "      LLM API Key 不在 config.json：请在应用「设置」页配置（保存即热加载）。",
  );
}

// ---------- 1) Python Agent 服务 ----------
// 依赖（uvicorn/fastapi/langchain）都装在 agent-service/.venv 里，必须用该解释器；
// 全局 python 没有这些包（No module named uvicorn）。
const venvPython =
  process.platform === "win32"
    ? path.join(ROOT, "agent-service", ".venv", "Scripts", "python.exe")
    : path.join(ROOT, "agent-service", ".venv", "bin", "python");
if (!existsSync(venvPython)) {
  console.error(
    "[dev] 未找到 agent-service/.venv 虚拟环境，无法启动 Agent 服务。\n" +
      "      请先执行：cd agent-service && uv sync",
  );
  process.exit(1);
}
// 引号包裹：绝对路径可能含空格（shell:true 时由 cmd/sh 保留引号）
launch(
  "agent-service",
  `"${venvPython}"`,
  [
    "agent_entry.py",
    "--port",
    "8000",
    "--config",
    `"${configPath}"`,
    "--reload",
  ],
  { cwd: path.join(ROOT, "agent-service") },
);

// ---------- 2) Tauri 桌面端（自动带起 Vite） ----------
// `npm --prefix frontend run tauri` 会把脚本 cwd 切到 frontend/，而 tauri-cli
// 只在 cwd 子树内查找配置，兄弟目录 src-tauri/tauri.conf.json 找不到 → panic。
// 改为在仓库根直接调 .bin 下的 cli（Windows 为 tauri.cmd），launch 默认 cwd=ROOT。
const tauriBin = path.join(
  "frontend",
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);
launch("tauri", `"${tauriBin}"`, ["dev"]);

// ---------- 信号处理：Ctrl+C 清理全部子进程 ----------
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    console.log("\n[dev] 正在停止所有子进程…");
    children.forEach(killTree);
    process.exit(0);
  });
}
