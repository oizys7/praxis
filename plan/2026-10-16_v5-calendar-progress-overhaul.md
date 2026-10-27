# 打卡日历页重构计划（续 v5，2026-10-16 晚）

需求原文五点 → 架构与任务拆解。基线 `dev/v0.1.0` @ `2899579`（五项需求全绿）。

## 需求映射

| # | 需求 | 层 |
|---|---|---|
| 1 | 年月切换：年=热力图、月=日历 | 视觉 |
| 2 | 页面重心=计划学习进度（非打卡纪事） | 信息架构 |
| 3 | 任意一天可点击 → 当天规划 + 完成情况 | 交互 |
| 4 | 联动学习计划数据、按预期时间自动分配每日章节 | 功能（数据层） |
| 5 | 新增计划时可选一周学习日 | 功能（三端协议） |

## Task D — 页面重构与双视图 + 日钻取（视觉优先，mock 先行）

1. **信息架构倒转（需求 2）**：页面第一屏 = 「进行中的学习计划」进度卡列表
   （每计划：标题、总章/已完成章、进度条、剩余天数、阶段分布摘要）；日历下移为辅助视图
2. **年月切换（需求 1）**：
   - 年视图：GitHub 风格 12 月热力图（行=周一~周日，按日学习量/完成度五档着色）
   - 月视图：现日历升级版（三态打卡点保留，重点着色为「当日分配章节完成度」）
   - 顶部切换控件：年/月 chip + ⇦⇨ 翻页 + 「今天」回跳
3. **日钻取（需求 3）**：点击任一天 → 右侧抽屉/dialog：
   - 该日各进行中计划的分配章节清单（阶段+章节名+时段估算）
   - 完成情况徽章（章节 status：完成/未完成）+ 「当日已学」打卡标记（Rust mark 接口先接、mock 数据阶段预占 props）
4. 组件拆分：CalendarView 拆 年热力图 YearHeatmap / 月日历 MonthGrid / 日抽屉 DayDrawer 三子组件；
   mock 数据集中 `calendarMocks.ts` 便于 Task E 断点接真
5. a11y/QA：格子/day button aria-label、抽屉 Esc 关闭+focus 管理、装饰 svg aria-hidden、热力图色阶语义令牌（不 sbí hardcode）

## Task E — 真数据联动（协议 + DB + 算法，D 落盘后接）

1. **三端协议**：`PlanRequest` 增 `studyDays`（number[]，默认全周 [0..6]），
   schemas.py（camelCase）/ commands.rs PlanRequest（serde default）/ agent.ts 三处同步；
   PlanForm 生成表单加「每周学习日」多选（默认全 7 天可改）
2. **DB**：`plans` 增 `study_days` JSONB（默认 [0,1,2,3,4,5,6]）；新表
   `study_day_marks(plan_id FK, day DATE, PK(plan_id, day))`（当日已学打卡，幂等 toggle）
3. **Rust IPC**：
   - `get_plan` / `list_plans` wire 增 `studyDays`
   - 新命令 `set_plan_study_days`（历史计划在日历页可改学习日 → 触发前端视图重算）、
     `set_study_day_mark`（日抽屉打卡，写/删 marks 行）
4. **分配算法（前端纯函数，确定性、无分配存储）**：
   输入 plan（weeks/weeklyHours/studyDays/chapters 按 phase 归组），输出每日章节分配：
   - 章 φ 个按 phase 的 weekRange 均匀铺到该区间内的学习日（轮转填充，章非满时压缩到章少周）
   - 依据 weekly_hours 与章内容长度微调权重（1.x 权重制即可，不过度工程）
   - 因为确定性，前端重算即视图，无需持久化分配表
5. **联动完成情况**：日抽屉的「完成情况」直接读 chapters.status（get_plan），
   打卡与集团合并口径 =「当日全部分配章节 status=complete 或 已标记当日已学」
6. **验收**：pytest（schemas.py 动了）/ cargo test+clippy / vue-tsc / npm check 全绿；
   真库 --ignored 跑 db 迁移与 mark 命令测试；live uvicorn 冒烟（mock 链）

## 顺序

D（视觉壳）→ 复核 → E（真数据）→ 复核 → 两笔独立 commit。
沿用子代理三纪律（$TMPDIR/不 add -A/不动 plan 与 AGENTS.md）。
Todo：分配算法边界欄（学习日全剔/章数>学习日数）在 E 简报中给铁则。
