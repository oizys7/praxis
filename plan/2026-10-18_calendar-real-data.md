# 打卡日历页接入真实数据（学习状态 + 持久化日程）

> 2026-10-18 · 用户 7 条需求 + 3 个确认决策（每章独立勾选打卡 / 全章节完成自动流转已完成 / study_day_marks 直接退役不迁移）
> 架构约束：Python 零改动（分配与查询全在 Rust，前端只展示）；遵守「未显式指示不提交不推送」。

## 现状 → 目标差异

| 现状 | 目标 |
| --- | --- |
| 前端 useStudyAllocation 纯函数即时重算（零存储），created_at 起排 | Rust 持久化日程表 study_schedule，start_learning / update_study_days 时落表 |
| 「开始学习」= 保存学习日 + **自动打卡今天** | chips 仅本地选择，点【从今天开始】一次性提交，**不打卡今天** |
| 学习日 chips 单击即调接口 | 本地暂存，统一提交 |
| list_plans + 逐计划 get_plan（N+1） | 仅 list_plans（summary 增强到自足） |
| 打卡表 study_day_marks（天级，存在即已学） | 新表 study_schedule：哪天学 + 学哪章 + 该行是否打卡（行级勾选） |
| 无学习状态 / 无月年接口 | plans.learning_status 三分类 + query_month / query_year |

## 数据模型（db_ddl.rs 幂等演进）

- **plans 新列**：
  - `learning_started_at DATE`（NULL = 未开始）
  - `learning_status TEXT NOT NULL DEFAULT 'not_started'`（`not_started` | `active` | `completed`）
- **新表 study_schedule**（day = 哪天学，done = 该行是否打卡）：
  `id UUID PK / plan_id FK CASCADE / date DATE / phase_idx INT / chapter_idx INT / chapter_title TEXT / phase_name TEXT / done BOOLEAN DEFAULT FALSE / created_at / updated_at`
  `UNIQUE (plan_id, date, phase_idx, chapter_idx)`；`idx_study_schedule_date ON (date)`（月/年查询）
  chapter_title / phase_name 冗余存展示名：月接口免回连 chapters JSON，重排时刷新。
- **DROP study_day_marks**（用户裁定放弃旧数据），Task E 相关 Rust/前端代码一并删除。
- 新表进时间戳触发器体系（set_updated_at）+ 中文注释；plans 新列注释补齐。

## 分配算法（Rust schedule.rs，移植前端 useStudyAllocation）

1. 输入：plan JSON（phases[].weekRange/chapters）、weeks、study_days、start_date（今天）。
2. 阶段 `weekRange`（"第 X-Y 周"）→ 周区间 [w1,w2] → 该区间内的学习日 slots；解析失败归兜底组铺全期。
3. 章节 `floor(i*m/n)` 均匀铺到 slots；`done` 一律 FALSE（打卡口径独立于章节状态）。
4. Rust 单元测试覆盖：区间分配、兜底组、slots 少于章节数、study_days 边界。

## Rust 接口（commands + protocol）

| 命令 | 语义 |
| --- | --- |
| `start_learning(planId, studyDays)` | 仅 not_started 可调；写 learning_started_at=今天、status=active、study_days；生成日程落表；**不打卡今天** |
| `update_study_days(planId, studyDays)` | 仅 active 可调；保留 date<今天 + (date=今天 AND done) 的行；删除其余；未完成章节从今天起按新学习日重排；打卡仅允许 date≤今天，故未来无 done 行 |
| `toggle_schedule_item(planId, date, phaseIdx, chapterIdx, done)` | 行级打卡；校验 date≤今天（不能打卡未来） |
| `query_month(year, month)` | 按天组织：`{days: [{date, items:[{planId, planTitle, phaseIdx, phaseName, chapterIdx, chapterTitle, done}], total, doneCount}]}`，只含有数据的天 |
| `query_year(year)` | 热力图：`{days: [{date, total, done}]}` |
| `list_plans` 增强 | PlanSummary 增 `learningStatus / learningStartedAt / studyDays(已有) / chapterTotal / chapterDone`——日历页零 get_plan |

**已完成自动流转**：章节完成写入点（章节状态置 complete 的 UPDATE 处）追加检查——该计划全部章节 complete → `UPDATE plans SET learning_status='completed'`。

## 前端

- `types/agent.ts`：PlanSummary 扩展字段 + MonthCalendar / YearHeatmap / ScheduleItem 类型。
- `usePlanHistory.ts`：startLearning / updateStudyDays / toggleScheduleItem / fetchMonthCalendar / fetchYearHeatmap；删 saveStudyDayMark / savePlanStudyDays。
- `CalendarView.vue`：load 仅 list_plans；月/年视图切换与翻页时调 query_month / query_year（按视图日期缓存）；删 marksByPlan / studyDaysByPlan 本地分配态与 computeDayAssignments 引用。
- `PlanProgressPanel.vue`：三分类分组（未开始 / 学习中 / 已完成）；「开始学习」展开区 chips 全本地，【从今天开始】一次性提交；学习中行「修改学习日」同模式提交 update_study_days。
- `DayDrawer.vue`：每章独立勾选（toggle_schedule_item），组头显示当天完成 n/m。
- `MonthGrid / YearHeatmap`：数据源改为后端结构映射（CalendarEvent 形状不变）。
- `useStudyAllocation.ts`：删分配/聚合/planStartDate/parseWeekRange/studySlots，保留 dayKey / normalizeStudyDays 等展示工具。

## 批次

1. **Rust 数据层**：db_ddl → schedule.rs（分配 + 六个函数）→ protocol → db.rs(list_plans 增强 + 章节完成流转) → commands → main.rs；cargo test + clippy。
2. **前端**：types → usePlanHistory → CalendarView 重写 → PlanProgressPanel / DayDrawer / MonthGrid / YearHeatmap → 删旧工具；vue-tsc。
3. **收口**：npm run format + 三端 check + 单测全绿。不提交不推送。

## 风险与边界

- weeks 内 slots 少于章节数：floor(i·m/n) 尾部收敛到末个学习日（与现前端行为一致）。
- update_study_days 后已完成章节（chapter.status=complete）不参与重排；重排集合 = schedule 中 done=false 的 (phase_idx, chapter_idx)。
- 旧打卡数据随 DROP 丢弃（用户裁定）；日历历史从启用日起算。
- 前端月网格固定 6×7 补空天，query_month 只回有数据的天即可。
