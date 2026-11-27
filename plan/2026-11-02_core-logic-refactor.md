# 生成主链路逻辑收敛执行计划

> 用户已批准执行；控制器逐步实现，不委派。`commands.rs` 留到其余修复与重构验证完成后处理；最终执行 `npm run format`。

**目标：** 消除已确认的章节状态三值口径分叉、修复重生成输入模型丢字段，并收敛事件处理与 Python 生成模块的重复/职责混杂。

**约束：** 不改 wire 字段、不改变有意区分的质量校验与内容完整性语义；状态唯一算法继续由 `plan_state::plan_of_chapters` 定义；Python 继续使用 `uv`；不触碰用户已有未提交的 `plan/2026-10-14_background-generation-and-progress.md`。

## 阶段顺序

1. **重生成上下文模型（Python）**：为 `PhaseOutline` 补齐 `chapterResearchSnippets` 的图内字段；先写回归测试，证明 `RegenRequest` 验证与 `model_dump(by_alias=True)` 后素材仍在，并且生成节点复用素材、不调用 SearchProvider。
2. **状态终态收敛（Rust，非 commands）**：启动 sweep 复用 `plan_status_of_chapters` 三值判定；重生成收尾把未完成子行收为 failed 后，按章节状态无条件写回计划终态；regen 开始时将计划行置 generating，保证强退后进入现有启动清扫候选。先补真实数据库集成测试，再实现。
3. **事件语义收敛（Rust，非 commands）**：抽取共享的阶段持久化结果→进度事件映射，使生成与重生成都遵循“写入成功才发 PhaseDone，失败只发 StorageIssue”；为成功/失败结果编写纯函数测试。
4. **Python 生成模块清理**：将响应包络组装移入生成层模块，消除 `tasks.py` 对端点模块 `main.py` 的懒导入；在大纲入口拒绝空阶段，返回明确 AgentServiceError；清除由 Pydantic 字段约束保证不可达的重复范围分支及其提示词变量（仅在测试先锁定边界后）。
5. **最后处理 `commands.rs`**：让 regen 所有 done 状态与传输错误统一经过一个计划终态收口路径；收敛阶段事件处理重复；把冗长的 regen 编排拆成命名清楚的小步骤。此阶段不改外部 Tauri/HTTP 事件契约。
6. **最终验证与格式化**：先运行 Python/Rust/前端完整检查；最后执行用户要求的 `npm run format`，检查格式化影响范围，再重跑完整检查与 Vite build。

## 风险与回滚

- 状态转换涉及真实 DB；新测试使用项目现有 `DATABASE_URL` 测试适配，只针对测试计划，断言成功、部分成功、全失败与中断后收尾。
- 事件处理统一不得改变事件顺序或在入库失败时伪报进度；通过 sink 结果映射测试及现有 mock 流测试守住。
- 每阶段保持独立 diff；若新测试暴露既有不一致，先确定它属于本阶段契约再继续，不以放宽断言规避。
- 回滚任一阶段时仅撤销该阶段文件改动；保留用户已有 plan 文档修改。
