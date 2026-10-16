# 内容质量与详情修复 —— 实施计划 v3

> 2026-10-13 晚。针对真实 GLM GUI 冒烟暴露的 6 个问题。基线 dev/v0.0.1 @ 4c75da0。

## 问题 → 根因 → 修法

### 问题 1：第一阶段有空章节但已标「已完成」，无法重新生成

**根因**：`insert_chapter_with_children`（plan_store.rs）无条件写 `status='complete'`——
不管章节有没有任何子数据（LLM 偶发返回空 mainKnowledge/空深潜）。前端重生成按钮
只在 failed/pending 出现，空内容 complete 章节成为死角。

**修法（存储层内容完整性判定）**：
- 入库状态由内容决定：`章节 status = (concepts/knowledge_points/diagrams/deep_dives 任一非空) ? 'complete' : 'failed'`
- 阶段联动照旧（全部章节 complete 才置 complete）→ 含空章节的阶段保持 pending →
  阶段级「重新生成此阶段」按钮自动可见 ✓，无函数式改动，只翻 SQL 语义
- 单章重生成空内容 → 该章 failed（按钮恢复可点），阶段不推进
- 既有升级路径兜底 UPDATE 不变（子数据空 → 保持 pending，语义一致）
- 前端无需改（failed 章节徽标 + 按钮已存在）

### 问题 2：章节无法最小化（只有深入内容可折叠）

**修法（前端交互）**：ChapterSection 标题区整行可点击 toggle——整章正文
（主要知识 + 深入内容）折叠/展开，标题与徽标/按钮常显；默认展开；深入内容
面板的独立折叠保持不变。

### 问题 3/4：内容过于结构化、像知识清单（面向初学者但无初学者视角）

**根因**：
a. **phase.yaml / chapter.yaml 完全没有 level 变量**（outline.yaml 有）——受众上下文缺席，
   LLM 只能输出中性知识点罗列
b. **条数偏多且每条太短**：concepts 3-6、knowledgePoints 4-8，detail 字段实际被
   LLM 写成半句短语 → 构建「广而浅」的清单

**修法（少而深 + 受众注入）**：
- level 变量接入 phase.yaml / chapter.yaml（nodes.py 渲染 variables 补 level），
  配受众指令：初学者→从零讲起、先给类比再给术语、常见黑话就地解释、
  每个概念说清「它解决什么问题、不用它会怎样」
- 条数减、单条加厚：concepts 3-6 → **2-4** 且 definition 每条 ≥2 句完整话；
  knowledgePoints 4-8 → **3-5** 且 detail 每条 2-4 句讲解（「是什么/为什么/何时用」，
  明令禁止电报式短语）
- 叙事指令：每章 summary 2-3 句（本章解决什么问题/为何此刻学/学完能做什么），
  章节标题之间保持递进

### 问题 5：深入知识没有深入

**根因**：下限 500 字过松（GLM 实测 673 字即达标）且无深入维度指令。

**修法（阈值 + 深入维度清单）**：
- `MIN_DEEP_DIVE_CHARS` 500 → **800**（validators.py + mock DEEP_DIVE_TARGET_CHARS 600→850
  + 相应测试断言同步）
- prompt 深入四维度（每篇至少覆盖三个）：**机制如何运转 / 为什么这样设计 /
  常见误区与踩坑 / 一个具体例子从头走一遍**
- 数量指引：1-2 篇，若篇幅受限宁可只写 1 篇并写足写深
- 结构化为段落叙事（有观点的连续文本），禁止用小标题把 800 字切碎成清单

**观察点（计划内裁定，非本轮实施）**：800 字下限 + 大嵌套 schema 会加重 GLM
function-calling 输出负担；若真实冒烟出现解析失败率骤升或截断，再评估
「深入内容二次生成」架构变体（章节级第二步：主知识一次请求、深入内容独立一次
请求）——本轮不做，先看质量。

### 问题 6：阶段数量、每阶段章节数量放宽

**修法（提示词与校验器同步）**：
- outline.yaml：2-5 个阶段 → **3-7 个**；每阶段 chapterTitles 2-4 → **2-6 个**
- validators.py：MAX_PHASES 6 → **8**；MAX_CHAPTERS_PER_PHASE 5 → **7**
- 周区间衔接、学时漂移校验照旧不动（同一套公式覆盖更宽的范围）

## 任务分解（4 个任务）

### 任务 18：内容完整性判定入库（问题 1）——Rust

- plan_store.rs：`insert_chapter_with_children` 返回/计算 has_content，
  upsert_phase / upsert_chapter 的章节行 status 按 has_content 置
  complete | failed；阶段联动条件不变
- 集成测试（真库）：①阶段含空内容章节 upsert → 空章节 failed、其余 complete、
  阶段 pending；②单章重生成空内容 → 章节 failed + 阶段不推进；③既有
  upsert_phase_idempotent / 两章渐进例不回归
- 本地 cargo test + clippy + fmt；真库由控制者复核
- 提交 `fix: 章节入库状态按内容完整性判定（空内容 failed 可重生成）`

### 任务 19：提示词受众/深度/范围（问题 3/4/5/6）——Python

- prompts/outline.yaml：3-7 阶段、每阶段 2-6 章、level 受众指令入大纲层级
  （初学者→阶段划分更细、周期更长）
- prompts/phase.yaml + chapter.yaml：level 变量 + 受众指令；concepts 2-4（definition
  ≥2 句）；knowledgePoints 3-5（detail 2-4 句禁电报式）；deepDives ≥800 字 +
  四维度清单 + 宁少而深 + 段落式叙事；summary 叙事指令
- nodes.py：expand/rechagen 的 variables 组装补 level（mock 路径同样）
- validators.py：MAX_PHASES=8、MAX_CHAPTERS_PER_PHASE=7、MIN_DEEP_DIVE_CHARS=800
- mock.py：DEEP_DIVE_TARGET_CHARS=850；mock 章节内容按新条数口径（3-5 知识点）
- tests：validators 断言同步（500→800、阶段/章节上限）；test_regen 深潜字数断言同步
- uv run pytest 全量回归 + npm run check
- 提交 `feat: 提示词受众注入与少而深内容规格，放宽阶段/章节范围与深潜下限`

### 任务 20：章节可折叠（问题 2）——前端

- ChapterSection.vue：标题区整行 toggle（chevron 旋转 + 折叠正文）；默认展开；
  深入内容面板独立折叠维持
- vue-tsc + npm run check + prettier
- 提交 `feat: 章节正文整章可折叠`

### 任务 21：e2e 收口

- npm run check / cargo test / 真库集成全绿
- uvicorn mock 真进程：regenerate 流（阶段级、单章）+ 深潜 ≥800 字断言 +
  outline 新范围（3-7 阶段）冒烟
- GUI 目验清单（用户手动）：整章折叠、空内容章节 failed 徽标 + 重生成按钮、
  真实 GLM 生成新计划观察内容质量（受众口吻、深潜深度、条数/字数统计）
- 若真实 GLM 内容质量仍不足或解析失败率上升 → 记录后提裁定（深入内容二次生成变体）

## 每任务审查重点

1. **（任务 18）状态机回归**：空内容 failed 不被失败收尾二次翻牌；旧数据升级
   兜底不受影响；两章渐进例与全内容例的阶段联动仍正确。
2. **（任务 19）提示词变量链**：level 从 PlanRequest.level（枚举）到 prompt 变量
   的映射（中文受众文案在 nodes 组装层完成，不在 prompt 内比较枚举值）；
   三份 prompt 的条数/字数口径与 validators 常量一致。
3. **（任务 20）无行为回归**：折叠不破坏重生成按钮/徽标/深潜面板。
4. **协议零改动**：本轮无 schema/协议/DDL 变化（全部在语义与内容层）。
