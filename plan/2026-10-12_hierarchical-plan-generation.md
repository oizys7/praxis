# 分层生成学习计划 + 规范化自动入库 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 subagent-driven-development（推荐）或 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** ① 把「单次 LLM 全量生成 StudyPlan」重构为「outline 骨架 → 按阶段并行扩展（章节化）→ 程序化校验 → LLM 评审 → 定向回炉」，解决生成慢、解析易错、超时、内容敷衍四大问题；② 持久化从「手动保存整串 JSON」改为「生成过程流式增量传输 + 每阶段自动落库 + 规范化多表存储」。

**架构：** LangGraph 拓扑 `clarify → outline → Send×N(expand_phase) → assemble → review →(定向回炉) expand_revision → assemble → END`，阶段草稿用 `Annotated[dict, operator.or_]` reducer 按索引覆盖合并。Python 新增 NDJSON 流式端点（`astream(stream_mode="updates")` 驱动），每完成一个阶段推一条事件；Rust `generate_plan` 命令改为流式消费：请求开始即建 `plans` 行（status=generating），每条 phase 事件事务性 upsert 进多表并通过 Tauri Channel 向前端推进度，done 事件后补全计划元数据。LLM 评审只做语义质量，程序化校验（字数/结构/算术）在 `assemble` 用纯函数完成，两者共用一条定向回炉通道，`revised` 标志保证最多回炉一次。

**技术栈：** Python 3.10+ / FastAPI StreamingResponse / LangGraph（Send、Annotated reducer、astream）/ Pydantic v2 / Rust Tauri2（reqwest 流式、tauri::Channel、sqlx PostgreSQL）/ Vue 3 + TS + mermaid@^11 / pytest + pytest-asyncio（新增）

**规格：** 本文件「规格」一节（需求来自用户 2026-10-12 对话，无外部文档）。

---

## 规格（需求原文归纳）

1. **禁止一次性生成整份计划**：生成慢、解析易错、接口超时、长度限制后内容敷衍 → 必须分层生成。
2. **阶段-章节两级结构**：一个学习计划分多个阶段；一个阶段包含多个章节；**一个阶段只有一个项目**（不按章节硬凑），但每个项目必须认真设计（背景、步骤分解、可测验收标准、学时估算）。
3. **章节内「主要知识 / 深入内容」分区**：
   - 主要知识：核心概念定义（概念名+定义）、核心知识点列表（标题+说明）、Mermaid 图辅助讲解（图题+源码+讲解）；
   - 深入内容：每篇正文 **≥500 字**，主题如某机制的底层原理、内部数据结构详解、历史演进分析；每章 1-2 篇。
4. **持久化改造**：不再保存整串 JSON；**每个阶段生成完成即自动入库**（无需手动点保存）；**不同数据进不同的表**（规范化），方便后续功能开发与迭代。

## 全局约束

- 三端协议字段全部 camelCase，任何字段改动必须同步：`agent-service/app/schemas.py`、`src-tauri/src/protocol.rs`、`frontend/src/types/agent.ts`
- 提示词只写 `agent-service/prompts/*.yaml`，禁止硬编码进 Python；模板变量 `{{ var }}`
- Python 一律用 uv（`cd agent-service && uv sync` / `uv run pytest`），不用 pip
- 云端 PostgreSQL（sqlx）是计划数据唯一真源，无本地库；DDL 只允许 `CREATE TABLE IF NOT EXISTS` / `ADD COLUMN IF NOT EXISTS` 幂等演进；**旧列 `plan_json` 保留只读兼容，新写入一律 NULL**
- 结构体/表演进「只增不删、全部默认值」：Rust 新字段一律 `#[serde(default)]`，旧字段 `theory` 保留，保证旧计划可继续打开
- 新增 HTTP 调用必须带超时（reqwest 按请求设置；流式消费用总超时包裹）
- 注释/文档/commit message 用中文，标识符英文；commit 首行 `类型: 摘要`
- 每任务完成跑 `npm run check`；改 Python 运行时后烟测 `uv run python -c "import app.main"`
- Rust SQL 集成测试用 `#[sqlx::test]`（需本机 PG 与 `DATABASE_URL`）；本机无 PG 时标 `#[ignore]` 并执行任务内的手动验证清单
- 警告清零：防御性保留代码用 `#[allow(dead_code)]` + 中文注释说明

## 审查重点（Review Focus）

1. **phase 事件乱序与覆盖**：fan-out 并行完成、回炉重生成，事件乱序到达——期望：按 `(plan_id, idx)` upsert 幂等，回炉覆盖旧数据且**旧章节/项目子行不残留**（事务内先删后插），读取按 idx 排序。测试：任务 9 的 upsert 覆盖测试。
2. **中断与失败的数据完整性**：生成中断/LLM 失败——期望：已完成阶段已入库、`plans.status='failed'` 可续查；clarification 终态删除 generating 空行。测试：任务 9、10 的状态流转测试。
3. **存储失败不吞生成结果**：云端库写失败/未配置——期望：日志告警 + StorageIssue 进度事件，PlanResponse 照常返回前端（结果不丢）。测试：任务 10 未配置 DB 的透传测试。
4. **深入内容不达 500 字 / Mermaid 非法**：校验器拦截该阶段进回炉；前端 mermaid 渲染失败降级显示源码不白屏。测试：任务 2、11。
5. **回炉死循环**：`revised` 标志双闸门，最多回炉一轮。测试：任务 6 永不通过的桩。
6. **旧数据兼容**：云端旧 `plan_json` 行、前端旧 `theory` 结构——期望：都能打开渲染。测试：任务 9 serde/重组兼容、任务 11 渲染回退。

---

## 新协议结构（三端写死，实现时按此照抄字段名）

JSON 层全部 camelCase，Python 内 snake_case（CamelModel 自动转）。

```text
StudyPlan                          # 顶层不变，phases 内部重构
├─ title / overview / estimatedTotalHours / resources / reviewNotes
└─ phases: StudyPhase[]
   ├─ name / weekRange / objectives[]
   ├─ chapters: Chapter[]          # 新增，替代 theory
   ├─ practice[] / assignments[]   # 保留（阶段级）
   └─ project: PhaseProject        # 一个阶段一个项目
      ├─ name / description / background(新) / estimatedHours(新,int)
      ├─ steps[](新) / acceptanceCriteria[]
Chapter
├─ title / summary
├─ mainKnowledge: MainKnowledge
│  ├─ concepts: {name, definition}[]
│  ├─ knowledgePoints: {title, detail}[]
│  └─ diagrams: {title, mermaid, caption}[]
└─ deepDives: {topic, angle, content}[]   # content ≥500 字
```

内部模型（不入三端响应协议）：`PlanOutline { title, overview, estimatedTotalHours, phases: PhaseOutline[], resources }`、`PhaseOutline { name, weekRange, objectives[], chapterTitles[], projectBrief }`。
`ReviewOutput` 增 `failedPhaseIndexes: list[int]`（0-based）。

## 新图拓扑（写死）

```text
clarify ──(无缺口)──> outline ──Send×N──> expand_phase ──> assemble ──(未修订)──> review ─┬─(passed 且无 pending)──> END
   │                                                                                      └─(pending 非空)──Send──> expand_revision ──> assemble ──(revised)──> END
   └─(有澄清问题)──> END
```

- `expand_phase`（并行 map）：只返回 `{"phase_drafts": {str(index): <Phase camelCase dict>}}`，**不得写其他状态键**（并行写冲突）。模块级 `asyncio.Semaphore(3)` 限并发。
- `AgentState.phase_drafts: Annotated[dict[str, dict], operator.or_]`——回炉重生成按索引覆盖旧草稿。
- `assemble`（单实例）：按 index 排序拼装 StudyPlan；调程序化校验；不合格阶段索引写 `pending_phase_indexes`；返回 `plan`。
- `review`：只做语义评审，把 `failedPhaseIndexes` 并入 `pending_phase_indexes`。
- `expand_revision`（单实例）：`asyncio.gather(return_exceptions=True)` 重生成 pending 阶段（带 review_notes），成功覆盖 drafts；失败保留旧草稿并向 review_notes 追加说明；返回 `revised=True`。
- 回炉后经 assemble 直接 END（最多回炉一次，不二次评审）。
- **校验规则**（`app/validators.py` 纯函数）：阶段 1-6 个；每阶段章节 1-5；每章 deep_dives 非空且每篇 `len(content.strip()) >= 500`；mainKnowledge 的 concepts/knowledge_points 非空，diagrams[].mermaid 非空且首词 ∈ {graph, flowchart, sequenceDiagram, classDiagram, stateDiagram, erDiagram, gantt, mindmap}；project 的 name/description 非空、steps≥2、acceptanceCriteria≥2、estimatedHours>0；objectives 非空；estimatedTotalHours 与 weeklyHours×weeks 偏差 ≤20%。

## NDJSON 流式事件协议（写死）

端点：`POST /api/generate-plan/stream`，请求体同 `PlanRequest`；响应 `Content-Type: application/x-ndjson`，HTTP 始终 200，每行一个 JSON 对象以 `\n` 结尾：

```jsonc
{"type": "outline", "phaseCount": 3, "title": "…计划标题"}     // outline 完成后推 1 条
{"type": "phase", "index": 2, "phase": {…StudyPhase camelCase…}} // 每阶段完成推 1 条，乱序
{"type": "done", "response": {…PlanResponse 三态包络…}}          // 终态，必为最后一行
```

- `outline` 事件由 `assemble` 之前从图状态取值不可行 → 实现上：流式端点在收到 `updates` 中 outline 节点增量时发送；
- `phase` 事件来自 `expand_phase` / `expand_revision` 增量的 `phase_drafts`（回炉覆盖语义天然成立）；
- 最终状态组装复用 `build_response(final_state) -> PlanResponse`（从现有 `generate_plan` 端点抽出，两端点共用）。

## 数据库 Schema（写死 DDL，首连幂等执行）

```sql
-- plans 表演化（旧列 plan_json 保留只读兼容，新写入一律 NULL）
ALTER TABLE plans ALTER COLUMN plan_json DROP NOT NULL;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'complete';   -- generating|complete|failed
ALTER TABLE plans ADD COLUMN IF NOT EXISTS estimated_total_hours INT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS phases (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
  idx INT NOT NULL, name TEXT NOT NULL, week_range TEXT NOT NULL DEFAULT '',
  objectives JSONB NOT NULL DEFAULT '[]', practice JSONB NOT NULL DEFAULT '[]',
  assignments JSONB NOT NULL DEFAULT '[]',
  UNIQUE (plan_id, idx)
);
CREATE TABLE IF NOT EXISTS projects (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  phase_id UUID NOT NULL UNIQUE REFERENCES phases(id) ON DELETE CASCADE,
  name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', background TEXT NOT NULL DEFAULT '',
  steps JSONB NOT NULL DEFAULT '[]', acceptance_criteria JSONB NOT NULL DEFAULT '[]',
  estimated_hours INT NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS chapters (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  phase_id UUID NOT NULL REFERENCES phases(id) ON DELETE CASCADE,
  idx INT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '',
  UNIQUE (phase_id, idx)
);
CREATE TABLE IF NOT EXISTS concepts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  idx INT NOT NULL, name TEXT NOT NULL, definition TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS knowledge_points (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  idx INT NOT NULL, title TEXT NOT NULL, detail TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS diagrams (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  idx INT NOT NULL, title TEXT NOT NULL DEFAULT '', mermaid TEXT NOT NULL DEFAULT '', caption TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS deep_dives (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  chapter_id UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
  idx INT NOT NULL, topic TEXT NOT NULL, angle TEXT NOT NULL DEFAULT '', content TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS resources (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
  idx INT NOT NULL, title TEXT NOT NULL, type TEXT NOT NULL DEFAULT 'docs', note TEXT NOT NULL DEFAULT ''
);
```

- 字符串数组（objectives/steps/acceptance_criteria）留在 JSONB——实体才拆表；
- **阶段 upsert 事务**（`plan_store.rs::upsert_phase`）：`INSERT … ON CONFLICT (plan_id, idx) DO UPDATE` 拿 phase_id → `DELETE FROM projects WHERE phase_id=$1` → `DELETE FROM chapters WHERE phase_id=$1`（子表级联清 concepts/knowledge_points/diagrams/deep_dives）→ 重插 chapters 及子行、project，全部单事务；
- `get_plan` 重组：`plan_json IS NOT NULL` 的旧行走旧反序列化路径；新行多表查询按 idx 排序组装 StudyPlan（status=generating/failed 的部分数据也允许返回）。

---

### 任务 1：pytest 基础设施 + 新协议模型（schemas.py）

**文件：**
- 修改：`agent-service/pyproject.toml`（dev 组加 `pytest>=8`、`pytest-asyncio>=0.24`、`httpx>=0.27`）
- 修改：`agent-service/app/schemas.py`
- 创建：`agent-service/tests/__init__.py`、`agent-service/tests/test_schemas.py`

- [ ] **步骤 1：编写失败的测试**（`tests/test_schemas.py`）

```python
def test_study_phase_camel_case():
    from app.schemas import StudyPhase
    data = StudyPhase(name="基础").model_dump(by_alias=True)
    assert "weekRange" in data and "chapters" in data and "theory" not in data

def test_chapter_alias():
    from app.schemas import Chapter
    c = Chapter(title="t").model_dump(by_alias=True)
    assert set(c) == {"title", "summary", "mainKnowledge", "deepDives"}
```

- [ ] **步骤 2：运行验证失败**：`cd agent-service && uv run pytest tests/test_schemas.py -v` → FAIL（ImportError）
- [ ] **步骤 3：实现**：按「新协议结构」新增 `ConceptDef`、`KnowledgePoint`、`KnowledgeDiagram`、`MainKnowledge`、`DeepDive`、`Chapter`；`PhaseProject` 增 `background`、`steps`、`estimated_hours`；`StudyPhase` 删 `theory` 增 `chapters`（default 空）、`project` 改 `PhaseProject | None = None`；`ReviewOutput` 增 `failed_phase_indexes: list[int] = []`；新增内部模型 `PhaseOutline`、`PlanOutline`（docstring 注明仅图内流转）
- [ ] **步骤 4：运行验证通过**
- [ ] **步骤 5：Commit**：`feat: 协议引入阶段-章节两级结构与深入内容模型`

### 任务 2：程序化校验器 app/validators.py

**文件：**
- 创建：`agent-service/app/validators.py`
- 测试：`agent-service/tests/test_validators.py`

- [ ] **步骤 1：编写失败的测试**——每条校验规则一正一反，断言 `validate_plan(plan_dict, weekly_hours=10, weeks=12) -> tuple[list[str], list[int]]`：`test_valid_plan_passes`、`test_shallow_deep_dive_rejected`（499 字 → pending 含该阶段，issue 含 "500"）、`test_bad_mermaid_prefix_rejected`、`test_hours_mismatch_rejected`（偏差 >20%）、`test_missing_project_steps_rejected`（steps 仅 1 条）
- [ ] **步骤 2：运行验证失败**
- [ ] **步骤 3：实现**——纯函数，输入 camelCase dict，不 import LangChain；常量 `MIN_DEEP_DIVE_CHARS = 500`、`MAX_HOUR_DRIFT = 0.2` 等放模块顶部
- [ ] **步骤 4：运行验证通过**
- [ ] **步骤 5：Commit**：`feat: 计划程序化校验器（字数/结构/学时/项目完整性）`

### 任务 3：mock 两级数据（mock.py 重构）

**文件：**
- 修改：`agent-service/app/mock.py`
- 测试：`agent-service/tests/test_mock.py`

- [ ] **步骤 1：编写失败的测试**——`test_mock_outline_shape`：`build_mock_outline(request)` 返回 3 个 PhaseOutline、每阶段 `chapterTitles` 2 条；`test_mock_phase_passes_validator`：`build_mock_phase(request, phase_outline, index)` 输出过校验器零 pending（深入内容确定性文本填充 ≥600 字，含一段 `flowchart TD` mermaid 示例）
- [ ] **步骤 2：运行验证失败**
- [ ] **步骤 3：实现** `build_mock_outline(request: dict) -> dict`、`build_mock_phase(request: dict, phase_outline: dict, index: int) -> dict`；项目按 index 取三套确定性模板（入门实战/综合练习/毕业项目），均含 background、≥3 steps、≥2 acceptanceCriteria、estimatedHours；删除旧 `build_mock_plan`
- [ ] **步骤 4：运行验证通过**
- [ ] **步骤 5：Commit**：`refactor: mock 改为 outline/phase 两级确定性数据`

### 任务 4：提示词 outline.yaml + phase.yaml

**文件：**
- 创建：`agent-service/prompts/outline.yaml`、`agent-service/prompts/phase.yaml`
- 删除：`agent-service/prompts/plan.yaml`（若 npm run check 因旧引用失败，本步骤顺延至任务 5 后执行）

- [ ] **步骤 1：写 `outline.yaml`**（骨架规划，无正文）。system 关键约束（写死）：你是学习路径架构师，只输出骨架；阶段 2-5 个、难度递进、周区间连续覆盖全部周数；objectives 2-4 条可衡量；`chapterTitles` 每阶段 2-4 个体现知识递进；`projectBrief` 一句话点出项目主题与产出物；estimatedTotalHours ≈ 每周小时×周数；resources 3-6 条注明类型与理由；全中文。human 变量：`{{goal}} {{level}} {{weekly_hours}} {{weeks}} {{interests}} {{background}}`
- [ ] **步骤 2：写 `phase.yaml`**（单阶段全文扩展）。system 关键约束（写死）：你是课程设计专家，为一个阶段写出全部章节正文；每章 mainKnowledge 含 3-6 个概念定义、4-8 条知识点、1-2 张 Mermaid 图（源码裸文本、首词为合法图类型、**不要** ```mermaid 围栏）；每章 deepDives 1-2 篇，主题从「底层原理/内部数据结构/历史演进/方案权衡」选，**每篇正文不少于 500 字**，深入到机制细节而非罗列名词；project 只有一个且认真设计：background 交代动机场景、steps 4-8 步可执行、acceptanceCriteria ≥2 条可验证（禁"了解/熟悉"）、estimatedHours 与阶段学时匹配；禁止杜撰不存在的书籍/课程。human 变量：`{{plan_context}}`（标题/概述/相邻阶段名）、`{{phase_outline}}`（本阶段骨架 JSON）、`{{review_notes}}`（回炉意见，首生成时为"（首次生成，无评审意见）"）
- [ ] **步骤 3：验证**：`npm run check` 的 YAML 校验通过；`uv run python -c "from app import prompts; prompts.load_prompt('outline'); prompts.load_prompt('phase')"`
- [ ] **步骤 4：Commit**：`feat: 骨架规划与阶段扩展两份提示词`

### 任务 5：nodes.py 重构（outline / expand_phase / expand_revision / assemble / review 改造）

**文件：**
- 修改：`agent-service/app/nodes.py`
- 修改：`agent-service/app/mock.py` 引用清理、`agent-service/app/main.py`（若引用已删符号）
- 测试：`agent-service/tests/test_nodes.py`

- [ ] **步骤 1：编写失败的测试**（monkeypatch `settings.allow_mock_llm = True`）：`test_outline_node_mock`（返回 outline，phases≥2）；`test_expand_phase_parallel_safe`（返回值键集合 == `{"phase_drafts"}`，防并行写冲突回归）；`test_expand_revision_keeps_old_draft_on_failure`（monkeypatch `invoke_structured` 抛 `AgentServiceError`，旧草稿保留、返回 `revised=True`）；`test_review_writes_pending`（stub 返回 `failed_phase_indexes=[1]`，增量含 `pending_phase_indexes=[1]`）
- [ ] **步骤 2：运行验证失败**
- [ ] **步骤 3：实现**（签名写死）：
  - `async def outline_node(state: AgentState) -> dict`：mock/无 Key 走 `build_mock_outline`；否则 `invoke_structured(PlanOutline, "outline", vars)`；返回 `{"outline": <camelCase dict>}`
  - `async def expand_phase_node(payload: dict) -> dict`（Send 目标节点收 payload）：`async with _EXPAND_SEMAPHORE:`；mock 走 `build_mock_phase`；否则 `invoke_structured(StudyPhase, "phase", vars)`；**只返回** `{"phase_drafts": {str(payload["phase_index"]): <phase dict>}}`
  - `async def expand_revision_node(state: AgentState) -> dict`：对 pending 阶段 `asyncio.gather(return_exceptions=True)` 重生成，成功覆盖 drafts，失败保留旧稿并追加备注；返回 `{"phase_drafts": {...}, "revised": True, "review_notes": <合并后>}`
  - `async def assemble_node(state: AgentState) -> dict`：按 index 排序拼 StudyPlan dict；调 `validators.validate_plan`；返回 `{"plan": ..., "validation_issues": ..., "pending_phase_indexes": ...}`
  - `review_node`：改用 review.yaml（plan JSON + validation_issues 注入），mock/无 Key 跳过逻辑保留；`failed_phase_indexes` 并入 `pending_phase_indexes`
  - 删除 `plan_node`
- [ ] **步骤 4：运行验证通过** + 烟测 `uv run python -c "import app.main"`
- [ ] **步骤 5：Commit**：`refactor: 节点重构为骨架规划/阶段扩展/拼装校验/定向回炉`

### 任务 6：graph.py 重装配 + 全链路测试

**文件：**
- 修改：`agent-service/app/graph.py`、`agent-service/app/nodes.py`（AgentState 增 `outline`、`phase_drafts: Annotated[dict[str, dict], operator.or_]`、`validation_issues`、`pending_phase_indexes`）
- 测试：`agent-service/tests/test_graph.py`

- [ ] **步骤 1：编写失败的测试**（mock 模式）：`test_full_flow_mock_returns_plan`（`ainvoke` 后 `plan["phases"][0]["chapters"]` 非空、`passed is True`）；`test_review_rework_route`（`build_graph(review=stub)` 注入 `passed=False, failed_phase_indexes=[1]`，断言 `revised is True` 且 index 1 阶段被重生成数据覆盖）；`test_rework_terminates`（stub 永远 `passed=False`，流程仍终止且 `revised is True`）
- [ ] **步骤 2：运行验证失败**
- [ ] **步骤 3：实现装配**（按「新图拓扑」）：outline 后条件边返回 `list[Send("expand_phase", payload)]`（payload：request、phase_outline、phase_index、plan_context）；`add_edge("expand_phase", "assemble")`；assemble 后条件边 `"review" if not state.get("revised") else END`；review 后条件边返回 `list[Send("expand_revision", ...)]` 或 END；`add_edge("expand_revision", "assemble")`
- [ ] **步骤 4：`uv run pytest tests/ -v` 全绿**
- [ ] **步骤 5：Commit**：`feat: LangGraph 重装配——Send 并行扩展与定向回炉`

### 任务 7：Python NDJSON 流式端点

**文件：**
- 修改：`agent-service/app/main.py`（抽出 `build_response(final_state) -> PlanResponse` 供两端点共用；新增流式端点）
- 测试：`agent-service/tests/test_stream.py`

- [ ] **步骤 1：编写失败的测试**（mock 模式，httpx `AsyncClient(transport=ASGITransport(app))`）：`test_stream_event_sequence`——POST `/api/generate-plan/stream` 逐行读，断言：首条 `type=outline` 且 `phaseCount≥2`；收到 ≥2 条 `type=phase` 且 `phase.chapters` 非空；最后一行 `type=done` 且 `response.status=="plan"`；`test_stream_clarification`（goal 留空的请求）——无 phase 事件，done.response.status=="clarification"
- [ ] **步骤 2：运行验证失败**
- [ ] **步骤 3：实现**：`@app.post("/api/generate-plan/stream")` 返回 `StreamingResponse(gen(), media_type="application/x-ndjson")`；生成器内 `async for mode, chunk in agent_graph.astream(input, stream_mode=["updates", "values"])`：updates 中 `expand_phase`/`expand_revision` 增量转 phase 事件、outline 增量转 outline 事件，并记录最新 values 快照为 final_state；循环结束 `yield done 行`（`build_response(final_state)`）；旧非流式端点保留
- [ ] **步骤 4：运行验证通过** + 烟测 import
- [ ] **步骤 5：Commit**：`feat: NDJSON 流式生成端点（outline/phase/done 事件）`

### 任务 8：Rust 协议同步（protocol.rs）

**文件：**
- 修改：`src-tauri/src/protocol.rs`
- 测试：`src-tauri/src/protocol.rs` `#[cfg(test)]`

- [ ] **步骤 1：编写失败的测试**——`old_plan_record_still_deserializes`：旧结构 JSON 常量（含 `theory`、无 `chapters`、project 无 `steps`）反序列化为 `StudyPlan` 成功且 `phases[0].chapters` 为空 vec
- [ ] **步骤 2：`cargo test` 验证失败**
- [ ] **步骤 3：实现**：镜像「新协议结构」（`ChapterDef`/`MainKnowledgeDef`/`ConceptDef`/`KnowledgePointDef`/`KnowledgeDiagramDef`/`DeepDiveDef`，命名跟随现有风格）；`StudyPhase` 保留 `#[serde(default)] theory` + 新增 `#[serde(default)] chapters`；`PhaseProject` 新字段全 `#[serde(default)]`；新增 `PlanProgressEvent`（`#[serde(tag = "type", rename_all = "camelCase")]` 枚举：`Outline { phase_count, title }`、`PhaseDone { index, total }`、`StorageIssue { message }`）；`ReviewOutputDef` 增 `#[serde(default)] failed_phase_indexes: Vec<i64>`
- [ ] **步骤 4：`cargo test` + `cargo clippy` 通过零警告**
- [ ] **步骤 5：Commit**：`feat: Rust 协议同步章节化结构与进度事件`

### 任务 9：Rust 规范化存储（db.rs 拆分 + plan_store.rs）

**文件：**
- 修改：`src-tauri/src/db.rs`（保留 pool 与 ensure_schema，DDL 按「数据库 Schema」节扩展）
- 创建：`src-tauri/src/plan_store.rs`（多表读写）
- 测试：`src-tauri/src/tests_plan_store.rs`（或 `#[cfg(test)]` 模块，`#[sqlx::test]`）

- [ ] **步骤 1：编写失败的测试**（`#[sqlx::test]`）：`test_upsert_phase_idempotent`（同 `(plan_id, idx)` 两次 upsert，chapters 不翻倍）；`test_rework_replaces_children`（upsert 覆盖后旧章节行与子行不残留）；`test_get_plan_reassembles_sorted`（乱序插入 idx 2,0,1 → 重组后 phases 顺序正确、deep_dives/diagrams 完整）；`test_legacy_plan_json_still_reads`（插入带 plan_json 的旧行 → get_plan 走旧路径成功）；`test_delete_plan_cascades`（删 plans 行 → 九表全清）
- [ ] **步骤 2：`cargo test` 验证失败**（无 PG 环境则确认编译失败于缺失 API，运行时标 `#[ignore]` + 执行步骤 7 手动清单）
- [ ] **步骤 3：实现 `plan_store.rs`**：`create_generating_plan(config, id, request) -> Result`（INSERT plans status='generating'，title 暂用 goal）；`update_outline_meta(config, id, title, estimated_total_hours)`；`upsert_phase(config, plan_id, idx, phase) -> Result`（按「阶段 upsert 事务」）；`finalize_plan(config, id, status)`（complete/failed/删除）；`get_plan` 改造（plan_json 旧路径 + 多表重组新路径，组装逻辑纯函数 `reassemble_plan(rows) -> StudyPlan` 便于测试）；`delete_plan` 不变（CASCADE 生效）；所有 SQL 带既有 10s 超时语义
- [ ] **步骤 4：`cargo test` + `cargo clippy` 通过**
- [ ] **步骤 5：手动验证清单（无 PG 环境时执行）**：连接设置页配置的真实库，`psql` 执行 DDL 后跑一次 mock 生成，逐表 `SELECT count(*)` 核对
- [ ] **步骤 6：Commit**：`feat: 计划数据规范化多表存储与阶段级 upsert`

### 任务 10：Rust generate_plan 流式消费 + Channel 进度 + 自动入库编排

**文件：**
- 修改：`src-tauri/src/commands.rs`（generate_plan 命令改造）、`src-tauri/src/agent_client.rs`（新增流式 POST，路径 `/api/generate-plan/stream`，`tokio::time::timeout(PLAN_TIMEOUT)` 包裹整个流消费）
- 测试：`src-tauri/src/commands.rs` `#[cfg(test)]`（NDJSON 行解析纯函数）

- [ ] **步骤 1：编写失败的测试**——`test_ndjson_lines_parse`：构造含粘包/尾行无换行的字节流 `b"{\"type\":\"phase\",...}\n{\"type\":\"done\",...}"`，断言解析出 2 个完整事件且忽略空行
- [ ] **步骤 2：`cargo test` 验证失败**
- [ ] **步骤 3：实现命令编排**（写死流程）：
  1. `ensure_agent_running` 后，若 DB 已配置：预生成 UUID 调 `create_generating_plan`；未配置：跳过入库并 `channel.send(StorageIssue { message: "云端库未配置，本次生成未自动保存" })`
  2. 流式 POST Python stream 端点，`bytes_stream()` 按行缓冲解析 NDJSON（解析逻辑抽纯函数 `parse_ndjson_line(&str) -> Option<StreamEvent>`）
  3. `outline` 事件 → `update_outline_meta` + `channel.send(Outline{..})`
  4. `phase` 事件 → `upsert_phase` + `channel.send(PhaseDone{ index, total: phase_count })`；**upsert 失败只记日志 + StorageIssue，不中断流**
  5. `done` 事件：status=="plan" → `finalize_plan(complete)`；=="clarification" → 删除 generating 行；=="error" → `finalize_plan(failed)`
  6. 传输中断/超时 → `finalize_plan(failed)`（已入库则），返回 Err
  7. 返回 `Ok(done.response)`（前端拿到的最终包络与旧版一致）
  8. 命令签名增加 `channel: tauri::Channel<PlanProgressEvent>`（Tauri 2 标准流式回调）
- [ ] **步骤 4：`cargo test` + `cargo clippy` 通过**
- [ ] **步骤 5：Commit**：`feat: 生成命令流式消费——每阶段自动入库与进度事件`

### 任务 11：前端类型 + 章节化渲染 + Mermaid + 进度

**文件：**
- 修改：`frontend/src/types/agent.ts`（镜像新协议；`theory?: string[]` 标注 deprecated）、`frontend/src/composables/useAgent.ts`（generatePlan 传 `Channel<PlanProgressEvent>`，暴露 onProgress）、`frontend/src/composables/usePlanHistory.ts`（删 savePlan）、`frontend/src/components/PlanResult.vue`（保存按钮 → "已自动保存"徽章 + 生成中进度显示）、`frontend/src/components/PhaseCard.vue`
- 创建：`frontend/src/components/ChapterSection.vue`、`frontend/src/components/MermaidDiagram.vue`
- 依赖：`frontend/package.json` 增 `mermaid@^11`

- [ ] **步骤 1：安装依赖**：`cd frontend && npm i mermaid`
- [ ] **步骤 2：实现 MermaidDiagram.vue**——props `{ code: string; caption?: string }`；`onMounted` 动态 `import("mermaid")`，`mermaid.initialize({ startOnLoad: false, theme: "default" })`，`mermaid.render(唯一id, code)` 注入 SVG；**try/catch 失败降级 `<pre>` 显示源码**
- [ ] **步骤 3：实现 ChapterSection.vue**——props `{ chapter: Chapter }`；三区：主要知识（概念表、知识点列表、diagrams 逐个 MermaidDiagram）、深入内容（每篇折叠面板：topic 标题、angle 副标、content 正文）
- [ ] **步骤 4：改造 PhaseCard.vue**——objectives / chapters（ChapterSection 循环）/ practice / assignments / 项目详情（name、background、steps 有序表、acceptanceCriteria、estimatedHours）；**兼容分支**：`phase.chapters?.length ? 章节渲染 : 旧 theory 列表`
- [ ] **步骤 5：接线进度**——useAgent 暴露进度回调；PlanResult 生成中显示「第 N/M 阶段完成」，完成后显示「已自动保存到云端」或 StorageIssue 文案
- [ ] **步骤 6：`npx vue-tsc --noEmit` 零错误**（`.vue` LSP 误报以 vue-tsc 为权威）
- [ ] **步骤 7：Commit**：`feat: 前端章节化渲染、Mermaid 图示与生成进度`

### 任务 12：端到端验证与收尾

- [ ] **步骤 1**：`cd agent-service && uv run python -c "import app.main"` 烟测
- [ ] **步骤 2**：`npm run check` 全绿（Python 语法+ruff+YAML、cargo check/clippy、vue-tsc），警告清零
- [ ] **步骤 3**：mock 流式冒烟：启动 uvicorn，`curl -N POST /api/generate-plan/stream`（合法请求体），断言逐行输出 outline → ≥2 条 phase → done，且 `phase.chapters[0].deepDives[0].content` ≥500 字、diagrams 非空
- [ ] **步骤 4**：真库冒烟（配置云端 PG 后走应用生成一次）：核对 `plans.status='complete'`、九表行数与阶段/章节数一致；中断一次生成（杀掉 Python 进程）核对 status='failed' 且已完成阶段在库
- [ ] **步骤 5**：真实 LLM 冒烟（如已配 Key）：生成 12 周计划，抽查第二阶段深入内容 ≥500 字、mermaid 可渲染；API 不可用则记录跳过原因
- [ ] **步骤 6**：Commit 收尾：`chore: 分层生成与自动入库联调收尾`

---

## 排除项（YAGNI，明确不做）

- checkpointer / 澄清问题续答（表单式交互下无必要）
- 生成中断后续传（已完成阶段在库可查即可，不做断点续跑）
- 资源链接真实性校验（提示词禁杜撰 + UI 免责足够）
- review 二次评审（回炉后直接出结果，与「最多回炉一次」对齐）
- SSE 格式（用更简单的 NDJSON 行协议）
