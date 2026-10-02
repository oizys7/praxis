#!/usr/bin/env node
/**
 * scripts/check.mjs —— 提交前的快速自检
 *
 * 依次执行：
 *   1. Python 语法检查（compileall）
 *   2. 提示词 YAML 校验（prompts/*.yaml 能否被安全解析）
 *   3. 前端类型检查（vue-tsc --noEmit）
 *
 * 用法：在仓库根目录执行 `npm run check`。
 * 任一步骤失败立即退出（exit code 1），适合挂到 git pre-commit。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

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

// 1. Python 语法检查：编译 agent-service/app 下所有源码
step("Python 语法检查（compileall）", "python", [
 "-m",
 "compileall",
 "-q",
 "agent-service/app",
]);

// 2. 提示词 YAML 校验：确保 prompts/ 下 YAML 均可解析（运行时由 prompts.py 加载）。
//    逻辑在独立脚本 scripts/check_prompts.py 中，避免 python -c 的多行代码
//    在 Windows cmd（shell:true）下因换行被截断的问题。
step("提示词 YAML 校验", "python", ["scripts/check_prompts.py"], {
 env: { ...process.env, PYTHONIOENCODING: "utf-8" },
});

// 3. 前端类型检查：通过 frontend 的 npm script 调用（npm exec + --prefix + --
//    的组合在 Windows 下传参不可靠，npm run 由 npm 自身解析 .bin，最稳定）
step("前端类型检查（vue-tsc --noEmit）", "npm", [
 "--prefix",
 "frontend",
 "run",
 "typecheck",
]);

console.log("\n[check] 全部检查通过 ✓");
