#!/usr/bin/env node
/**
 * scripts/init.mjs —— 一次性初始化三端依赖
 *
 * 为什么要有这个脚本：仓库的依赖分散在四个地方（根目录的 Prettier、
 * frontend/ 的 Vue 工具链、agent-service/ 的 uv 虚拟环境、src-tauri/ 的
 * crate 依赖），README 里是四段手敲的命令。少敲一段的后果不是报错得早，
 * 而是报错得晚——典型是 `npm run dev` 跑到一半才发现 .venv 不存在。
 * 这里把它们收成一条命令，并且**先检查工具有没有装**，否则每一步都会以
 * 一段难以理解的原生报错收场。
 *
 * 幂等：可重复执行，已装好的依赖走缓存（npm/uv 都不会重复下载）。
 *
 * 用法：在仓库根目录执行 `npm run init`。
 *
 * 注意区分：`npm init`（不带 run）是 npm 内置命令，会去创建 package.json，
 * 与本脚本无关。
 */
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function log(message) {
  console.log(`[init] ${message}`);
}

/**
 * 命令统一以「整串」形式交给 shell。
 *
 * 两个理由：
 * 1. Windows 上 npm / uv / cargo 是 .cmd 包装，不经过 shell 会 ENOENT；
 * 2. Node 22+ 对「shell:true + args 数组」会刷 DEP0190（参数只做拼接不转义）。
 *    本脚本的命令与参数全是硬编码常量、不含任何外部输入，拼成整串既无注入面，
 *    也不会每次运行都吐一条弃用警告。
 */
function shellLine(command, args = []) {
  return [command, ...args].join(" ");
}

/** 探测命令是否可用。 */
function hasCommand(command) {
  const result = spawnSync(shellLine(command, ["--version"]), {
    shell: true,
    stdio: "ignore",
  });
  return result.status === 0;
}

/** 跑一条可能耗时的安装命令；失败即停，避免后面的步骤建立在坏基础上。 */
function runStep(name, command, args, cwd) {
  log(`→ ${name}`);
  const result = spawnSync(shellLine(command, args), {
    cwd,
    stdio: "inherit",
    shell: true,
  });
  if (result.status !== 0) {
    console.error(`\n[init] ✗ ${name} 失败（退出码 ${result.status}）`);
    process.exit(result.status ?? 1);
  }
}

// ---------------------------------------------------------------------------
// 1. 前置工具：缺一个就停下来把话说清楚
// ---------------------------------------------------------------------------
const REQUIRED_TOOLS = [
  ["node", "Node.js ≥ 18：https://nodejs.org/"],
  ["npm", "npm（随 Node.js 一起安装）"],
  ["uv", "uv（Python 环境与依赖管理）：https://docs.astral.sh/uv/"],
];

const missing = REQUIRED_TOOLS.filter(([command]) => !hasCommand(command));
if (missing.length > 0) {
  console.error("[init] ✗ 缺少前置工具，请先安装：\n");
  for (const [command, hint] of missing) {
    console.error(`  · ${command} —— ${hint}`);
  }
  console.error("");
  process.exit(1);
}

// Rust 单独处理：只有桌面端需要它，缺了也不该挡住前端/Python 的依赖安装，
// 但必须说清楚「现在能用什么、以后会卡在哪」。
const hasCargo = hasCommand("cargo");
if (!hasCargo) {
  console.warn(
    "[init] ! 未检测到 cargo（Rust）。前端与 Python 部分照常安装，但\n" +
      "         `npm run dev` / `npm run build` 会在编译桌面端时失败。\n" +
      "         安装：https://rustup.rs/",
  );
}

// ---------------------------------------------------------------------------
// 2. 三端依赖
// ---------------------------------------------------------------------------
runStep("仓库根依赖（Prettier，供 npm run format）", "npm", ["install"], ROOT);
runStep(
  "前端依赖（Vue / Vite / tauri-cli）",
  "npm",
  ["install"],
  path.join(ROOT, "frontend"),
);
runStep(
  "Python Agent 依赖（uv sync：创建 .venv 并按 pyproject.toml 安装）",
  "uv",
  ["sync"],
  path.join(ROOT, "agent-service"),
);

// Rust 依赖预取：只是把 crate 提前拉下来，省掉第一次 dev/build 的等待。
// 失败不算错——真正的编译会在 npm run dev / build 时重试并给出完整报错。
if (hasCargo) {
  log("→ Rust 依赖预取（cargo fetch，仅加速首次编译）");
  const fetched = spawnSync(shellLine("cargo", ["fetch"]), {
    cwd: path.join(ROOT, "src-tauri"),
    stdio: "inherit",
    shell: true,
  });
  if (fetched.status !== 0) {
    console.warn(
      "[init] ! cargo fetch 未成功，跳过（不影响后续编译，只是首次会慢）",
    );
  }
}

// ---------------------------------------------------------------------------
// 3. 配置模板：与 dev.mjs 同一套预检，避免用户第一次 npm run dev 才发现
// ---------------------------------------------------------------------------
const configPath = path.join(ROOT, "config.json");
const templatePath = path.join(ROOT, "config.example.json");
if (!existsSync(configPath) && existsSync(templatePath)) {
  copyFileSync(templatePath, configPath);
  log("已从 config.example.json 生成 config.json（默认值即可直接运行）");
  log("  · LLM 的 Key 在应用内置「设置」页配置，不写进这个文件");
}

// ---------------------------------------------------------------------------
// 4. 收尾
// ---------------------------------------------------------------------------
log("");
log("✓ 初始化完成（脚本可重复执行：依赖已就绪时第二次运行不会有任何改变）");
log("  下一步：npm run dev");
if (!hasCargo) {
  log(
    "  注意：桌面端需要 Rust，装好 cargo 后再跑一次 npm run init（或直接 npm run dev）。",
  );
}
