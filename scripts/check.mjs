#!/usr/bin/env node
/**
 * scripts/check.mjs —— 提交前的快速自检（三端覆盖：Python / Rust / 前端）
 *
 * 依次执行：
 *   1. Python 语法检查（compileall）
 *   2. Python lint（uvx ruff check）
 *   3. 提示词 YAML 校验（prompts/*.yaml 能否被安全解析）
 *   4. 三端 wire 契约一致性（protocol/protocol.json ↔ TS 接口 / Python 模型）
 *   5. Rust 编译检查（cargo check）
 *   6. Rust lint（cargo clippy）
 *   7. 前端类型检查（vue-tsc --noEmit）
 *
 * 用法：在仓库根目录执行 `npm run check`。
 * 任一步骤失败立即退出（exit code 1），适合挂到 git pre-commit。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_DIR = path.join(ROOT, "src-tauri");

/** 运行一个检查步骤，失败即终止 */
function step(title, command, args, opts = {}) {
  console.log(`\n[check] ▶ ${title}`);
  const result = spawnSync(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    shell: true,
    ...opts,
  });
  if (result.status !== 0) {
    console.error(`[check] ✗ ${title} 失败`);
    process.exit(1);
  }
  console.log(`[check] ✓ ${title}`);
}

// ---------------- Python 端 ----------------

// 1. 语法检查：编译 agent-service/app 下所有源码
step("Python 语法检查（compileall）", "python", [
  "-m",
  "compileall",
  "-q",
  "agent-service/app",
]);

// 2. lint：uvx 临时运行 ruff（uv 钦点工具链，无需预装；@锁定版本保证检查结果可复现）
step("Python lint（ruff check）", "uvx", [
  "ruff@0.16.9",
  "check",
  "agent-service/app",
  "scripts/check_prompts.py",
  "scripts/protocol_check.py",
]);

// 3. 提示词 YAML 校验：确保 prompts/ 下 YAML 均可解析（运行时由 prompts.py 加载）。
//    逻辑在独立脚本 scripts/check_prompts.py 中，避免 python -c 的多行代码
//    在 Windows cmd（shell:true）下因换行被截断的问题。
//    解释器用 uv 环境（--project agent-service）：PyYAML 是 agent-service 的
//    运行依赖，只装在 .venv 里；系统 python 未装 yaml 会让本步骤必然失败。
step(
  "提示词 YAML 校验",
  "uv",
  ["run", "--project", "agent-service", "python", "scripts/check_prompts.py"],
  {
    env: { ...process.env, PYTHONIOENCODING: "utf-8" },
  },
);

// 4. 三端 wire 契约一致性：真源 protocol/protocol.json 与前端 TS 接口、
//    Python Pydantic 模型逐字段比对（缺字段 / 未声明的多余字段都报错）。
//    解决的是「一份契约三处手写镜像」的老问题——一次字段改动曾波及 21 个
//    文件，且仍会漏（GlossaryEntryDef 漏 rename_all 导致字段名不下发）。
//    Rust 侧不在此脚本内：serde 的类型化 + wire 测试比正则解析可靠。
step("三端 wire 契约一致性（protocol.json）", "uv", [
  "run",
  "--project",
  "agent-service",
  "python",
  "scripts/protocol_check.py",
]);

// ---------------- Rust 端 ----------------

// 5. 编译检查（cargo check 只查类型/借用，不产出二进制，远快于 build）
step("Rust 编译检查（cargo check）", "cargo", ["check"], { cwd: TAURI_DIR });

// 6. lint：clippy，零警告标准（与 AGENTS.md「提交前验证」一致）
step("Rust lint（cargo clippy）", "cargo", ["clippy"], { cwd: TAURI_DIR });

// ---------------- 前端 ----------------

// 7. 类型检查：通过 frontend 的 npm script 调用（npm exec + --prefix + --
//    的组合在 Windows 下传参不可靠，npm run 由 npm 自身解析 .bin，最稳定）
step("前端类型检查（vue-tsc --noEmit）", "npm", [
  "--prefix",
  "frontend",
  "run",
  "typecheck",
]);

console.log("\n[check] 三端检查全部通过 ✓");
