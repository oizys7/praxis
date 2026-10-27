# 学习计划生成链路 —— 头脑风暴与改进清单

> 日期：2026-10-19　基线：dev/v0.1.0 @ ebdda0b
> 方法：4 个子代理并行研究（失败率 / 内容质量 / 结构与交互 / 外部实践），
> 关键结论由控制器逐条回读源码核实（标 ✓ 者为已核实）。

## 一、现状基线

一次成功生成 = `1 outline + Σ章 chapter + Σ阶段 extras + 1 review`，典型 4 阶段 × 4 章 ≈ **22 次 LLM 调用**；
单章输出 2.5k–5k tokens；`_PARSE_ATTEMPTS = 2`；Rust 侧 regen 链路总超时 600s。

---

## 二、方向 1：降低生成失败率

### P0 — 直接导致「整条计划报废」（全是小改动）

| # | 问题 | 位置 | 改法 | 成本 |
|---|---|---|---|---|
| 1 | ✓ 章节扇出**无 try/except**：任一章抛异常 → LangGraph Send 传播 → `astream` 中断 → 已完成的 N-1 章白做 | `nodes.py:expand_chapter_node` / `expand_phase_wrap_node` | 节点内 try/except，失败**不写**自己的 `chapter_drafts` 键（`or_` reducer 缺键不覆盖他人）。assemble 已有「空占位 → 标 failed → 定向重扩」的完整自愈链，只是被首轮异常炸断 | 小 |
| 2 | ✓ `OutputParserException`（JSON 截断/非法）**不在重试链**：只捕 `ValidationError`，截断直接判死 | `llm.py:246` / `llm.py:268` | except 元组补 `OutputParserException`、`openai.APIStatusError`(429/5xx)、`anthropic.APIError`；并显式传 `max_tokens`（chapter 场景 ≥8k），可用 `stop_reason=="max_tokens"` 分类 | 小 |
| 3 | ✓ `review_node` 单点报废：此时计划已完整落库，一次评审超时/解析失败就把全图炸掉（而 clarify / no-key 分支都有降级） | `nodes.py:review_node` | try/except → `passed=True` + review_notes 记「评审未执行：{原因}」；硬校验已在 assemble 由 validators 跑过 | 极小 |

### P1 — 预算错配与假失败

| # | 问题 | 改法 | 成本 |
|---|---|---|---|
| 4 | Rust regen 全阶段路径用 **600s 总超时**，但该路径是 N 章串行（单章 60–180s）→ 慢 provider 必假超时，Python 还在白烧 token | 改「逐行空闲超时」（如 120s/行），Python 侧已有 `chapterStart` 心跳天然适配 | 小 |
| 5 | chapter/phase 入库失败只 warn + StorageIssue，不重试；done 收尾按 `all_chapters_complete` 翻牌 → **内容完整却被标 failed** | done(status=plan) 时用包络里的完整 plan 逐 phase/章幂等 upsert 对账，再 settle | 小–中 |
| 6 | 回炉粒度错位：一个 mermaid 首词大小写不对 → 判定结构性失败 → **整阶段所有章 + extras 全部重写** | `validate_plan` 返回 per-chapter 问题键（`"<p>:<c>"`），章级问题只重扩该章 | 中 |
| 7 | review 输入是**全计划 JSON 含所有正文**（大计划 100k+ tokens）→ 上下文溢出/延迟/费用；注意力也被稀释 | review 只喂骨架 + 每章 summary + 深入节抽样 + validators issues | 中 |

### P2 — 自愈能力

- **断点续跑**：图状态纯内存，失败后已落库的章节无"继续生成"路径 → 新增 resume（从 DB 重建状态只跑 assemble 之后）。
- **失败章自动重生成**：done(error) 时已知失败章集合，但收尾只翻 failed，修复留给用户手工；可自动排队 ≤K 个 `regen_chapter`。
- **WS 重连耗尽即翻 failed**：Python 任务仍存活且事件保留 600s，任务 id 未持久化 → 翻 failed 前存 id，启动时先查 snapshot 再决定是否重新 attach。

---

## 三、方向 2：内容质量（能否系统学到目标）

### P0

| # | 问题 | 改法 |
|---|---|---|
| 8 | ✓ **extras 提示词断链**：`practice/assignments/project` 是决定"学得如何"的部分，但注入变量只有 `plan_context / phase_outline / weekly_hours / weeks` —— **goal / level / interests / background 全部缺席** | `_expand_phase_extras` 注入 4 个字段 + `phase_extras.yaml` 加画像段；`phase.yaml` 同步补 |
| 9 | 章节正文是自由 markdown，教学要素全靠软约束；validators 只查字数 ≥1000 与 mermaid 首词 | **固定小节脚手架**：为什么学这章 / 前置知识 / 学习目标（2-4 条可自查）/ 核心概念 / 示例与动手 / 深入：××（≥800 字）/ 常见误区 / 练一练 / 自检清单 / 延伸；validators 增加「必备小节存在性」+ 深入节字数程序化检查 |

### P1

- **outline 缺结构承载**：`PhaseOutline` 只有 name/weekRange/objectives/chapterTitles/projectBrief，没有 **先修关系（dependsOn）** 与 **知识点清单** → 跨阶段重复/断层零防线。加 `dependsOn` + `knowledgePoints`，注入下游提示词「已讲过勿重讲」。
- **plan_context 太薄**：每章的全局视野只有「标题 + 概述 + 相邻阶段名」四行 → 一行改动注入全部阶段 objectives。
- **review 只做定性判断**：改分维度 1-5 打分 + 阈值触发定向回炉，新增「跨阶段重复」「目标关键词覆盖」「练习与章节对齐」三个维度。
- **章内练习缺失**：所有动手都推到阶段收尾，学与练割裂；章末无自检、无延伸、无代码示例要求（提示词只规定 mermaid 围栏，对编程类目标是硬伤）。

### 外部实践对照（与上述判断吻合的部分）

- **评估先行**：先产出「每章学习目标（Bloom 行为动词）+ 每章 3-5 道自测题」，再 fan-out 正文，正文生成时把目标+题目作为上下文强制对齐。
- **先修 DAG 硬校验**：`prerequisites` 进 schema，校验节点做拓扑排序/环检测/难度平滑，不通过带着错误重试。
- **外层 JSON 骨架 + 内层 markdown 正文**混用（机器校验用 schema，正文留 markdown 保证 token 效率与渲染）。
- `finish_reason == "length"` 检测 → 分块续写（与 P0-2 同源）。

---

## 四、方向 3：划分与交互合理性

**结构本身合理**（阶段 → 章 → 练习/作业/项目），问题在「生成 ≠ 学会」：产品重生成、轻学习。

### P0（学习闭环断裂）

| # | 问题 | 改法 | 类型 |
|---|---|---|---|
| 10 | 详情页**没有「今天学什么 / 继续学习」**，hero 只有计数 | hero 加当日日程入口，直达章节锚点并展开 | 前端 |
| 11 | 详情页**看不到打卡态**（打卡只能去日历抽屉勾） | `Chapter` 增只读 `done`（后端 join 日程），章节标题行加打卡徽章 | 协议+前端 |
| 12 | 生成完成后**无法直接开始学习**（入口只在日历页左栏面板） | 详情页加「开始学习」，把 studyDays 选择器抽成共用组件 | 前端 |
| 13 | **澄清流是死路**：只展示问题文本，答案无处提交 | 每个问题配输入框，「回答并重新生成」（协议加 `clarifications[]` 或并入 background） | 协议+前端 |
| 14 | `practice/assignments` 是纯文本、项目验收只展示不可勾 | 列表项加 done 布尔（DB 列 + 协议），验收标准 checklist 可勾 | 协议+DB+前端 |

### P1

- **重生成粒度**：`complete` 章不能重生成（质量不满意无路可走）；不能单独重做 practice/assignments/project（只能整阶段重做，连带全部章节）。
- **内容呈现**：无**代码高亮**（编程学习硬伤，marked 无 highlighter）、长文无章节内 mini-TOC/锚点、mermaid 主题硬编码不跟暗色、正文无自测题/术语卡。
- **节奏管理半成品**：无每章时长（日历只能按章均分）、落后无感知无顺延、漏卡无提醒。

### P2

- 输入侧缺 `purpose（求职/转行/兴趣）/ expectedOutcome（作品集/能独立做 X）/ resourcePrefs（付费/中文/视频）`；
- `studyDays` 明确「不走 LLM」，LLM 自报 `weekRange` 与实际学习日分布可能脱节；
- 收藏 IPC 已有但详情页无入口；导出/打印/分享均无。

---

## 五、建议的落地顺序

**第 1 批（1 天内，全部为小改动，收益最大）**
① 章节扇出 try/except（#1）
② llm.py 异常网 + max_tokens（#2）
③ review 降级（#3）
④ extras 提示词补 goal/level/interests/background（#8）

**第 2 批（失败率工程化收尾）**
⑤ 回炉粒度到章级（#6）　⑥ Rust regen 空闲超时（#4）　⑦ done 对账落库（#5）

**第 3 批（质量杠杆）**
⑧ 章节固定脚手架 + validators 小节校验（#9）　⑨ outline 增 dependsOn/knowledgePoints　⑩ review 分维度打分

**第 4 批（学习闭环）**
⑪ 详情页今天学什么 / 开始学习 / 打卡态（#10-12）　⑫ 澄清流对话化（#13）　⑬ 代码高亮 + 章内 TOC

**可作为 v1 之后的差异化**：断点续跑、失败章自动重生成、评估先行（自测题驱动）、FSRS 间隔复习、导出/笔记。

---

## 六、子代理提出的、需要人判断的点

1. **是否引入第二个异构评审模型**：性价比排序建议先做「单章轻量评审」（每章一次并行调用，天然规避全计划超长），再考虑异构模型。
2. **mock 是否作为失败兜底**：建议不自动兜底（会让用户拿到演示内容却以为是真结果），但可作为 error 包络里 retryHint 的素材。
3. **结构化 vs markdown 的边界**：正文继续 markdown（已定），但「学习目标 / 自测题 / 术语」这类需要校验与交互的内容，值得从正文里抽成结构化字段（三端协议改动）。
