#!/usr/bin/env node
/**
 * scripts/format.mjs —— 一键格式化
 *
 * 使用 Prettier 格式化：
 *   - 前端源码（frontend/src 下 ts / vue / css）
 *   - 前端配置（frontend 根下 json / js / ts / html）
 *   - 构建脚本（scripts/*.mjs）
 *   - 根目录 Markdown（README 等）
 *
 * Python 侧如需格式化，建议另行引入 ruff：`ruff format agent-service`
 *
 * 用法：在仓库根目录执行 `npm run format`。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const targets = [
 "frontend/src/**/*.{ts,vue,css}",
 "frontend/*.{json,js,ts,html}",
 "scripts/**/*.mjs",
 "*.md",
 "!frontend/dist/**",
];

console.log("[format] 运行 Prettier…");
const result = spawnSync("npx", ["prettier", "--write", ...targets], {
 cwd: ROOT,
 stdio: "inherit",
 shell: true,
});

if (result.status !== 0) {
 console.error("[format] 格式化失败");
 process.exit(1);
}
console.log("[format] 格式化完成 ✓");
