# 计划 v2：重生成状态链修复 + 章节级完成通知 + UI 细节对齐 + 生成降为章级扇出（2026-10-15）

用户真机反馈五项 + 新增指令：「不要一个阶段一起生成，一个章节一个章节生成，保证
质量」。v1 的 3 任务扩展为 4 个，分段派工：第一波 A/B/C 并行，第二波 D（依赖 B
的 ChapterDone 协议变体先行落地）。

## 根因诊断（已取证）

### ① 返回列表再进：按钮不可点（task-A 核心）
`useAgent.ts` 的 `regenInFlight` 是**模块级全局布尔锁**；点击「重新生成此阶段」
invoke 未 resolve 前（Rust 还在跑）用户返回列表——页面销毁但锁不动；再进同一计划，
PhaseCard `:disabled="busy || regenInFlight"` 被全局锁钉死。「等一段时间后可点」=
invoke 完成终于 resolve 的时刻。

**修法**：锁改 keyed——模块级 `Map<planId, scopeKey>`（同 plan 互斥保留并发保护），
且释放点从「组件 finally」升级为「模块级 promise finally」+ 观察库收章完成事件
（页面切换 promise 仍活，finally 照跑）。PhaseCard disabled 判据改为查 keyed 锁 +
本 scope。卸载组件的 `busy` ref 不再参与跨页判定。

### ② 等待后可点但状态仍「未生成」，重进列表才「已完成」（task-A 核心）
`PhaseCard.onRegenerate` 完成路径是 `fetchPlanRecord → emit("refreshed", record.plan)`，
而 emit 的目标组件**已随页面切换卸载** → Vue emit 静默 no-op → App.onPlanRefreshed
收不到 → 界面停留旧快照；重进时 openHistoryPlan 重读才见新状态（与观察一致）。

**修法**：完成处理改为先落**模块级 store**（复用 useGenerationProgress 观察库新增
`regenResults: Record<planId, StudyPlan>`），再尝试 emit；PlanResult/PlanListView
挂载与激活时消费 store 最新结果（consume-after-read 语义，消费即清）。组件存活时
行为不变（emit 路径保留 + store 同记，双保险）。

### ③ 完成通知只到阶段层（task-B 核心，跨两端）
现状：单章重生成进度事件复用 `PhaseDone{index=章节索引, total=1}`——语义混叠；
且该事件只进 Channel 页内回调，前端没有章完成的一等公民状态。

**修法（三端小扩）**：
- `protocol.rs` `PlanProgressEvent` 增加 `ChapterDone { phaseIndex, chapterIndex }`
  变体（tag=type camelCase，walk 兼容旧 `PhaseDone{total=1}` 保留变体不删）；
- `commands.rs` run_regen 单章路径 upsert 成功后改发 `ChapterDone`（阶段联动徽章
  由前端按章集齐推断）；wire 单测锚定新变体；
- `agent.ts` union 同步 + `useGenerationProgress` 记 `chapterDone` 集合
  (`Map<planId, Set<"{phaseIndex}:{chapterIndex}">>`)；
- PhaseCard/ChapterSection 收到章完成：章徽章转已完成、若该阶段全部章 complete
  则阶段徽章联动。PhaseDone{total=1} 消费路径退役（add-only，旧事件防回退）。

### ④ 大纲后新建页左右两栏顶部不齐（task-C）
左 PlanForm 卡与右侧 Card（过程卡/骨架树）首个内边距不一致（右 `CardContent
py-6` + 内层标题行；左卡结构不同）→ 右侧顶部视觉更高。**修法**：对齐两卡同一基准
（统一 CardContent 顶部 padding 内边距，实施时实测 4/8/16px 对齐后取齐），限
App.vue 281-470区段 + PlanForm.vue 卡头部，不触业务区。

### ⑤ 「← 返回列表」样式与固定顶置（task-C）
现状 App.vue:277 纯文本按钮随内容滚动。**修法**：挪入固定 header（顶部栏
`shrink-0` 已不滚）右侧位（historyMode 显示），样式升级为 ghost Button +
ArrowLeft 形图标（项目图标内联，无新依赖），紧凑 size-sm。App.vue header 区
225-235 行附近。

## 任务分配（第一波：A/B/C 并行）

- **task-A**（frontend 状态链）：useAgent keyed 锁 + dispatch 判据、观察库
  regenResults 落袋/消费、PhaseCard 完成路径改造、（①②）。
- **task-B**（Rust + 三端协议）：protocol.rs ChapterDone 变体 + wire 单测、
  run_regen 改发点、agent.ts union（③；前端消费点在 task-A 落地后由 controller
  合入——避免 PhaseCard/runner 冲突）。
- **task-C**（frontend UI）：④⑤ 纯视觉/布局（与 A 分区：header/卡头 vs
  script/store 区）。

并行关系：A、B、C 三 worker 同轮 dispatch；App.vue 由 A（script 区）、C（header
与卡头区）各占一区，git 可合；B 的 PhaseCard 消费补丁由 controller 在 A 落地后合。

## ⑥ 生成侧降为章级扇出（task-D，依赖 task-B 先落地 ChapterDone）【用户新增，质量主线】

**动机**：阶段级调用一次要吐 2–6 章长 markdown（每章 1500–3000 字），是 GLM 解析
失败/空参数/截断的重灾区（历史多个实机事故都在这一层）。outline 已产出每章标题，
天然支持章级 Send 扇出；单次 payload 小 → 解析可靠、单章失败只回炉单章、进度天然
到章（与 ③ 同构）。

**Python（agent-service，主体改动）**：
- 拓扑改为：outline → Send×章 expand_chapter（每章 1 次调用，chapter 上下文带
  所属阶段目标/兄弟章标题/全局周次）+ 每阶段 1 次**轻量 wrap 调用**生成阶段级
  practice/assignments/project（PhaseProject 不属任何单章）→ 汇合（
  chapter_drafts 按 `(phaseIdx, chapterIdx)` 键合并，Annotated+or_ reducer）
  → assemble（按阶段重组 StudyPhase）→ review → 定向回炉。
- 回炉粒度细化：assemble 时标定 contentMd 缺失/过短/超时长的**失败章集合**
  ；回炉仅重扩展失败章；小时漂移等结构性问题回炉该阶段全部章（语义不变）。
- 提示词：`chapter.yaml` 拆出「生成态」用法（新增可有 sibling/phaseGoal 变量），
  硬底线（1500–3000 字/八型 mermaid/禁电报体）与 phase.yaml 同源不改红。
- `tasks.py` / NDJSON / WS 共享通道不变：章节节点 update74 事件 type=chapter，
  共享生成器按 type 透传，Rust 已支持 StreamEvent::Chapter（任务 15）。
- mock：`build_mock_phase` 重构为 `build_mock_chapter`（章级 mock），旋手
  ；pytest 全绿（现 104 基线）。

**Rust（src-tauri，小扩展）**：
- 生成链 GeneratePlanSink 已处理的 Chapter 事件链路（upsert_chapter、
  全章完成时阶段升促 upsert_chapter 内建）直接复用；进度推进改为：每章完成
  发 `ChapterDone{phaseIndex, chapterIndex}`，阶段全部章完成时发 `PhaseDone`
（阶段计数语义不变，前端 ①② 修复不受影响）。
- run_regen 的整阶段重生成（regenerate_phase）改走同一章扇出机制（RegenSink
  已支持 chapter 事件），regenerate_chapter 不变。

**frontend（小补丁，由 controller 在 A 落地后合）**：
- 大纲骨架树利用 outline 事件已带的 chapterTitles：章级 Carousel_DONE/失败
  标注（PhaseDone 集合计数不动，chapterDone 集合由 task-B 流入）。

## 任务分配（第二波）

- **task-D**（Python 主体 + Rust 小扩 + 前端骨杆小补丁）：上节全部；在 task-B
  的 ChapterDone 变体合入后 dispatch。

## 验收

1. 真机复验五项：①点重生成→返回→再进（按钮应立即可用或明确显示进行中）；
   ②重生成完成无需重进列表状态即完成；③单章重生成完成有章级反馧；④两栏顶齐；
   ⑤返回键固定顶置且样式统一。
2. 生成质量：真机 GLM 复跑一次全量生成——每章独立调用成功率显著高于原阶段级
   失败率；骨架树可见章级逐个点亮；单章失败只回炉单章。
3. vue-tsc / npm run check / cargo test+clippy / pytest 全绿；wire 单测锚定
   ChapterDone 三用例（生成/阶段回炉/单章回炉）。
