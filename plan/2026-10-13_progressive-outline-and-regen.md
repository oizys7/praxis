# 渐进式大纲展示 + 失败可见 + 阶段/章节级重新生成 —— 实施计划（v2）

> 2026-10-13 起草，同日按用户审阅意见修订。基于分支 dev/v0.0.1（基线 80293c1）。
> 背景为真实 GLM Key 冒烟暴露的产品断层：大纲与已完成阶段在生成期间不可见；失败收尾后用户无法在界面上看到已完成/失败分布，也无法重试失败部分。已直查云端库证实：失败计划的已完成阶段数据完好入库，仅缺少「大纲先建、状态标记、按需重生成」的产品化通道。

## 用户需求（原始 + 审阅修订）

1. **大纲先行**：阶段和章节的大纲生成结束后就应该在界面上显示，章节内容可以慢慢生成（渐进式渲染）。
2. **失败可见 + 可重试**：失败的部分应当在大纲上可见；保留生成时的上下文（提示词），可以由用户重新生成。
3. **（审阅修订）章节级状态与章节级重新生成也要做**——不只是阶段级。
4. **（审阅修订）DDL 分离**：db.rs 中表结构的创建/修改移到新文件 `db-ddl.rs`（Rust 模块名下划线规范），统一管理与逻辑代码分离。

用户已确认的设计决策：

- 重新生成入口 = 历史详情页**阶段级与章节级按钮**（每个失败阶段/章节旁「重新生成」，逐个点；上下文自动从库中回填）。
- 推进方式 = 计划审阅通过后直接开工（SDD 流程）。

## 全球约束（沿用既有约定）

- 三端协议 camelCase 对齐：frontend/src/types/agent.ts ↔ src-tauri/src/protocol.rs ↔ agent-service/app/schemas.py；协议**只增不改**，新字段一律可选/带默认值（旧数据兼容）。
- 业务错误走 HTTP 200 + `status=error`；流式通道 done 必为最后一行（三态包络）。
- Python 一律 uv；tauri-cli 从仓库根 `npm run tauri -- <cmd>`；JSON 配置不写注释。
- 提示词只写在 agent-service/prompts/*.yaml；LLM 配置经设置页热载。
- DDL 只用 CREATE/ADD COLUMN IF NOT EXISTS（首连幂等）；旧数据「只增不删」。
- Python 用 uv（pytest/pytest-asyncio/httpx 已在 dev 组）；Rust 集成测试 #[sqlx::test] 标 #[ignore]（远程测试库 DATABASE_URL postgresql://praxis:…@47.93.243.146:5432/praxis，凭据仅作临时环境变量）。
- 注释/文档/commit 中文；禁止 git add -A。

## 设计定稿

### 1. 两级状态机（PhaseStatus / ChapterStatus）

```text
pending（骨架已入库，内容未生成）
   │ 对应内容生成成功（upsert_phase 全阶段 / upsert_chapter 单章）
   ▼
complete ── regenerate 前先重置回 pending
   ↑
   │ 流失败收尾 / regenerate 失败（仅 pending 参与流转）
   ▼
failed
```

- DDL 演进（首连幂等）：
  - `phases ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'`（pending|complete|failed）
  - `chapters ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'`
  - `plans ADD COLUMN weekly_hours INT NOT NULL DEFAULT 0` / `interests JSONB NOT NULL DEFAULT '[]'` / `background TEXT NOT NULL DEFAULT ''`
- **rules**：complete 永不被 failed 覆盖（失败收尾只动 pending）；upsert 后必 complete；重生成开始时目标阶段/章节置 pending（前端即时转「生成中」）。

### 2. 大纲先建骨架（seed_outline）

outline 事件（已带阶段大纲）到达时：

1. `update_outline_meta`（现状不变：title + estimated_total_hours）
2. **seed_outline**（新函数）：按大纲载荷建立骨架行——
   - phases 行：name/week_range/status='pending'（objectives 不进大纲事件也不播种，保持事件轻——与 §3 口径一致）
   - chapters 行：title/summary=''/status='pending'（**章节骨架含标题**，题目是 outline 大纲的一部分 chapterTitles）
   - resources 行：老 resources 表写入
   - 幂等保护：仅当该计划 phases 为空时播种（防 regenerate 与竞态覆盖已有完成数据）
3. channel 推 Outline 进度事件（带 phases 骨架载荷）

说明：projects 骨架行**不播种**（大纲阶段只有 project_brief 一句话，无法填充 project 完整结构；project 留待 upsert_phase 整体写入，前端 project 区按 status 显示）。

### 3. 大纲事件载荷（协议扩展，只增）

```jsonc
{ "type": "outline", "phaseCount": 3, "title": "…",
  "overview": "……",                  // 新增，可缺省
  "phases": [                        // 新增，可缺省（向后兼容）
    { "name": "阶段一：…", "weekRange": "第 1-4 周",
      "chapterTitles": ["线程池深度解析", "虚拟线程与结构化并发"] }
  ] }
```

数据源：Python outline 节点产出的 PlanOutline（字段现成 name/week_range/chapter_titles），直接 by_alias 序列化；objectives/project_brief 不进事件（保持事件轻，objectives 入库时从大纲取）。

### 4. 保留生成上下文（重新生成的回填源）

plans 三新列（goal/level/weeks 已存）+ create_generating_plan 写入 weeklyHours/interests/background。regenerate 时 Rust 读 plans 七字段重建 PlanRequest。提示词模板不入库（上下文 = 请求参数 + 大纲，可完整重建生成条件）。

### 5. 双级重新生成链路（合并端点）

```
前端（失败阶段/章节旁「重新生成」）
  → invoke("regenerate_phase", { planId, phaseIndex, channel })
  → invoke("regenerate_chapter", { planId, phaseIndex, chapterIndex, channel })
      │ Rust：plans 七字段 + phases/chapters 骨架重建 PlanRequest + PlanOutline
      │       目标阶段/章节置 pending（前端即时可见）
      → POST /api/regenerate/stream
           body: { request, outline, phaseIndex, chapterIndex? }   // chapterIndex 有=单章，无=全阶段
      │ Python：目标为全阶段 → expand_phase 全阶段（StudyPhase 输出）
      │         目标为单章   → chapter 级生成（新 prompt chapter.yaml，输出单个 Chapter）
      │         流式 NDJSON：{type:"phase",...}（全阶段）或 {type:"chapter",...}（单章）→ {type:"done",response}
      │ Rust：入库复用 upsert_phase（全阶段）/ 新 upsert_chapter（单章，事务：chapters ON CONFLICT (phase_id,idx) + 子表替换，置 complete）
      │        章节重生成后若该阶段全部 chapters complete → 阶段同步置 complete
      └ 前端：进度 → 完成后 refresh fetchPlanRecord
```

- **单章 upsert**：chapters 表已有 UNIQUE(phase_id, idx)，ON CONFLICT DO UPDATE title/summary/status + 删四子表重插——天然支持大纲骨架行的「内容填充」，不残留旧数据。
- 阶段状态联动：单章 upsert 成功后检查：该阶段 chapters 的 pending/failed 全部为 complete → 阶段置 complete。
- Python 端点输出 schema：全阶段复用现（StudyPhase），单章输出 Chapter 模型（schemas 现成）。
- 新 prompt：prompts/chapter.yaml（单章生成：入变量 goal/background/interests/weeklyHours/weeks/phaseName/phaseWeekRange/phaseObjectives/chapterTitle/siblingChapterTitles/projectBrief，约束与 phase.yaml 的章节规格一致——主知识三件套 + 深潜 ≥500 字 + mermaid 白名单）。

### 6. 前端改动

- **生成期间（App.vue loading 卡片）**：outline 事件带阶段大纲 → 立即渲染骨架树（阶段名 + 章节标题，各标「待生成」）；phaseDone → 对应阶段整块填完成；phaseDone（单章场景）不用于初次生成。
- **历史详情页（PlanResult/PhaseCard/ChapterSection）**：
  - PhaseCard 头部阶段徽标（✓ 已完成 / ✗ 失败 / ◌ 未生成；旧数据 status 缺省 → 无徽标）
  - failed 阶段旁「重新生成此阶段」；章节行按 chapter.status 徽标 + failed 章节旁「重新生成此章」
  - useAgent:regeneratePhase / regenerateChapter（invoke + Channel 进度）→ 完成后 fetchPlanRecord 刷新
- 生成失败的 error 结果页追加引导文案：「已完成的阶段已保存，可到历史记录中查看并重新生成失败部分」。

### 7. 明确不做（YAGNI）

- prompts 持久化；自动重试/定时重试；同计划多阶段并发重生成（按钮请求期间禁用，后端 upsert 幂等安全）；学习计划以外的计划类型扩展。

## 任务分解（6 个任务）

### 任务 13：DDL 抽离 + Rust 存储层（两级状态机 + 大纲骨架入库 + 上下文列）

- **新文件 src-tauri/src/db_ddl.rs**：把 db.rs 全部建表/演进 DDL（plans 建表 + plan_json 可空化 + 三新列 + phases/chapters status 列 + 其余八表）迁入统一管理（常量化或函数化，首连幂等语义不变）；db.rs 只保留调用与连接池逻辑；main.rs mod 声明；确保 `cargo test` 全绿、`#[sqlx::test]` 集成测试用例全部照跑（远程测试库已可用）。
- **DDL 演进**：plans 三列；phases ADD status；chapters ADD status。
- **protocol.rs**：StudyPhase.status: Option<String>、ChapterDef.status: Option<String>（wire 可选，Anchor：serde camelCase，全 default）；PhaseStatus 常量或直接字符串（不建枚举，避免多一处转换）。
- **plan_store.rs**：
  - `seed_outline`（幂等：phases 空才播种；写 phases/status='pending'、chapters/status='pending'、resources；不写 project）
  - upsert_phase 置 status='complete'；新增 `upsert_chapter`（单章事务：chapters ON CONFLICT (phase_id, idx) + 四子表替换 + complete；随后若该阶段 chapters 无 pending/failed 则阶段置 complete——SQL 单事务内原子完成）
  - `mark_phase_pending` / `mark_chapter_pending`（重生成开始重置）＋ 失败收尾扩展 finalize_plan(failed, pending_to_failed)：phases/chapters 的 pending→failed（complete 不动）
  - create_generating_plan 写 weekly_hours/interests/background 三列
  - load_study_plan 读两级 status 装入 StudyPhase/Chapter
  - 新增 `load_plan_request_meta` / `load_plan_outline`（重建 PlanRequest 所需七字段 + PlanOutline 骨架），供任务 15 复用
- 集成测试（#[sqlx::text + #[ignore]]，真库）：seed→upsert 状态流转 pending→complete；失败收尾 pending→failed 且 complete 不动；seed 幂等不重复播种；单章 upsert 后子表替换 + 阶段 all-complete 联动；上下文读回（PlanRequest 七字段 + PlanOutline 完整）。
- 提交：`feat: DDL 抽离 db_ddl 模块与两级状态机、大纲骨架入库`

### 任务 14：Python regenerate 端点 + outline 载荷扩展 + chapter prompt

- prompts/chapter.yaml：单章生成提示词（风格/规格与 phase.yaml 章节规格一致：概念/知识点/图示 mermaid 白名单/深潜 ≥500 字；互链 title 引用问题按 spec：禁止引导语「了解/熟悉」出现在 acceptanceCriteria 之外）；模板变量：goal/background/interests/weeklyHours/weeks/phaseName/phaseWeekRange/phaseObjectives/chapterTitle/siblingChapterTitles/projectBrief
- main.py：/api/generate-plan/stream 的 outline 事件载荷扩展（overview + phases[name/weekRange/chapterTitles] 直出自 PlanOutline）
- 新端点 `POST /api/regenerate/stream`：RegenRequest(request: PlanRequest, outline: PlanOutline, phaseIndex: int, chapterIndex: int | None)：
  - chapterIndex=None：全阶段 → astream expand_phase 封装（build_plan_context 复用 + Semaphore + 解析重试已有错误反馈）→ {type:"phase"} + done
  - chapterIndex 指定：单章 → build_plan_context + chapter 上下文（同阶段兄弟章标题占位参考）→ invoke_structured(Chapter, "chapter") → {type:"chapter", index, phaseIndex, chapter} + done
  - done 三态包络语义同任务 7（error 也走 done；验证失败 → AgentServiceError → error done）
- mock：build_mock_outline 已有，加 build_mock_chapter(outline, phase_index, chapter_index)（章节级单点）+ mock regen 响应
- tests：test_outline_event_carries_phases（outline 事件 chapterTitles 非空）；test_regen_phase_stream（[phase, done(plan)] 且 chapters 非空）；test_regen_chapter_stream（[chapter, done(plan)] 单章结构含 mainKnowledge/deepDives）；超界 phaseIndex/chapterIndex → done(error)
- 提交：`feat: 阶段/章节级重生成流式端点与单章提示词`

### 任务 15：Rust regenerate 编排（两命令 + 共享流消费）

- commands.rs：
  - `regenerate_phase(plan_id, phase_index, channel)` ①读 plans 七字段 + phases 骨架（load_plan_request_meta/load_plan_outline）②mark_phase_pending ③consume regenerate 流（复用任务 10 分行/解析/入库基础设施，只换 URL 与事件处理：phase 事件→upsert_phase→_phase_done；chapter 事件忽略/防错） ④done 分派：plan→ 整阶段 status complete 后 finalize（计划位 complex 收尾）… ⑤error/Err → mark_phase_failed + Err/Ok 透传
  - `regenerate_chapter(plan_id, phase_index, chapter_index, channel)`：同构造；chapter 事件→upsert_chapter→PhaseDone{index, total:1} 或新 ChapterDone 事件（前端粒度：用现有 PhaseDone 即可，index 传 chapter 显示不受影响）
  - 提炼共享 `consume_ndjson_stream(response, on_event)`（与任务 10 的 generate_plan 流消费重构为同一函数，防双份漂移；分 retreat 改动最小化优先）
- StreamEvent 加 Chapter { index, phaseIndex, chapter } 变体（Rust 入站）
- 单测：上下文重建纯函数（PlanRequest 七字段回填断言）；cargo test + clippy 零警告 + fmt
- 提交：`feat: regenerate_phase/regenerate_chapter 命令与共享流消费`

### 任务 16：前端两级状态渲染 + 重新生成

- types：OutlinePhasePayload、PlanProgressEvent.phases/overview、StudyPhase.status: string | undefined、Chapter.status: string | undefined
- App.vue：outline 骨架树（阶段名+章节标题，状态点）；失败 error → 未完成部分「失败」标
- PlanResult/PhaseCard/ChapterSection：两级徽标 + 失败旁「重新生成」按钮（re generating 期间禁用 + spinner）
- useAgent:regeneratePhase / regenerateChapter（invoke + Channel）→ 完成后 fetchPlanRecord 刷新详情
- error 结果页引导文案
- vue-tsc / npm run check 全绿
- 提交：`feat: 两级状态徽标与阶段/章节级重新生成入口`

### 任务 17：e2e 验证收口

- npm run check / cargo test（+ 真库 -- --ignored）/ vite build 全绿
- mock 模式 uvicorn 真进程：/api/regenerate/stream 双级验证（全阶段/单章，事件序列与错误路径）；outline 事件载荷断言
- GUI 目验清单更新与人工目验（骨架渐进渲染、两级徽标、两级按钮、完成后刷新、上下文回填正确——重生成阶段与原始请求参数一致）
- 提交（如有收尾杂项）；汇总 deferred minor

## 每任务审查重点

1. **seed_outline 幂等与竞态**：仅 phases 空时播种；regenerate 绝不触发 seed。
2. **状态机完整性**：complete 永不被 failed 覆盖；upsert 后必 complete；单章 upsert 的阶段联动在事务中原子。
3. **上下文重建正确性**：regenerate 的 PlanRequest 与原请求逐字段一致（interests JSONB 读回）+ PlanOutline 骨架对得上（chapterTitles 顺序）。
4. **协议兼容**：outline 新载荷 Option 缺省零影响；单章 NDJSON 新 type=chapter 的 Rust 解析不影响原初生链路事件循环。
5. **重生成失败体验**：error done → 目标行置 failed → 前端按钮恢复可点，且详情刷新显示最新状态。
