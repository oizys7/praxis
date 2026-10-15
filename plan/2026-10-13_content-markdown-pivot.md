# 章节正文 markdown 化重构 —— 实施计划 v5（无旧数据兼容版）

> 2026-10-13 晚。v4 经用户裁定加严：**直接修改表结构，不考虑旧数据兼容**——
> 双路径读写、legacy 结构化形态、子表全部退役。基线 dev/v0.0.1 @ 4c75da0。

## 核心转向

章节内容不再结构化拆解，正文以一个 markdown 文本字段存储与传输：

```jsonc
// Chapter wire（三端，Chapter 全部形态就这一种）
{ "title": "…", "summary": "…（2-3 句）", "contentMd": "## …\n\n（完整 markdown 正文）",
  "status": "pending|complete|failed" }          // status 仅 Rust/DB 层属性（Python 不产）
```

## 设计定稿

### 1. 协议（彻底简化，非只增）

- **四张内容子表 DELETE**（concepts / knowledge_point_defs / diagrams / deep_dives ——
  DROP TABLE IF EXISTS，连同其中的既有测试数据，用户裁定放弃兼容）
- ChapterDef（Rust）：`{title, summary, content_md: String, status: Option<String>}`——
  main_knowledge/deep_dives 字段**整体删除**；相关类型（MainKnowledgeDef/ConceptDef/
  KnowledgePointDef/KnowledgeDiagramDef/DeepDiveDef）删除
- Python Chapter：`{title, summary, content_md}`（CamelModel 别名 contentMd）；
  MainKnowledge/ConceptDef 等模型**删除**；StudyPhase/RegenRequest 等不动
- 前端 Chapter：`{title, summary, contentMd: string, status?: PhaseStatus}`；
  MainKnowledge 等接口删除；ChapterSection **只保留 markdown 渲染**（结构化渲染删除）

### 2. 存储

- DDL（ensure_schema 内追加三语句，幂等）：
  `ALTER TABLE chapters ADD COLUMN IF NOT EXISTS content_md TEXT NOT NULL DEFAULT ''`
  + `DROP TABLE IF EXISTS deep_dives / concepts / knowledge_points / diagrams`（四条）
- plan_store：写入 = content_md 直写（status：content_md 非空 → complete，空 → failed——
  空内容 failed 可重生成，前轮裁定保留）；读取 = content_md 直读（无双路径）
- 升级兜底 UPDATE（DDL_LEGACY_*_BACKFILL）：**删除**（无 legacy 概念）
- 读回已有行有标题无正文的旧 churn 行 status 语义由行自身 status 列承载
- plan_json legacy 读路径（更早一代的兼容层）**不动**——那是上一轮架构的兼容，
  与本次表结构重构无关

### 3. 提示词（深度与视角核心，条数约束退役，深度从叙事要求来）

- outline.yaml：3-7 阶段、每阶段 2-6 章；level 受众指令
- phase.yaml：markdown 叙事规格——每章 {title, summary, content}；level 注入
  （从零讲起/先类比后术语/黑话就地解释/「解决什么问题/不用会怎样」）；每个知识点
  段落式充分讲解（是什么/为什么/何时用，禁电报式）；每章 ≥1 节「深入剖析」≥800 字
  （机制/设计原因/踩坑/实例走读至少三）；图示 = ```mermaid 码块（8 类白名单）
- chapter.yaml（单章重生成）：同口径单章三字段
- validators.py 章节规则重写：阶段/章节范围（8/7 上限）、周区间、学时漂移照旧；
  章节 contentMd 空 → failed_indexes；`MIN_CHAPTER_CONTENT_CHARS = 1000` 托底；
  mermaid 白名单扫码块首词；结构化字段规则退役
- mock.py：markdown 章节构造（确定性模板，含 mermaid 码块 + 深入剖析节 ≥1000 字）
- mock 条数/字数目标与校验器对齐

### 4. 前端

- MarkdownContent.vue：marked 解析 → 按 ```mermaid 码块切分为
  [DOMPurify 消毒后的 HTML 段(v-html) + MermaidDiagram 组件] 渲染
- ChapterSection.vue：markdown 渲染 + 整章标题行折叠（问题 2）+ 徽标/重生成不变
- 新依赖 marked + dompurify

### 5. 不做（YAGNI）

子表删除的迁移导出；代码高亮；深入内容二次生成变体（看 flat schema 真实质量后裁定）；
plan_json 层的任何改动。

## 任务分解（3 个任务）

### 任务 18：协议精简 + 表结构直改 + Rust 简化 + mock markdown + 前端 markdown 渲染

- scope：§1 协议三端（删型清理）+ §2 存储（DROP 子表/content_md 列/去 backfill）+
  plan_store 写读简化 + mock.py 转 markdown 构造 + 前端 types 清理 +
  MarkdownContent.vue + ChapterSection 重写（含折叠；marked+dompurify）+
  Rust 集成测试重整（删 legacy 用例，新增 markdown roundtrip/空内容 failed/替换无残留）+
  Python 测试同步（schemas/regen/mock/stream）
- 验证：cargo test/clippy/fmt + uv pytest 全量 + npm run check；真库控制者复核
- 提交 `feat: 章节正文 markdown 化——协议精简、表结构直改与前端渲染`

### 任务 19：提示词叙事化 + level 注入 + validators 重写

- scope：§3 全部（outline/phase/chapter 重写、nodes variables 补 level、
  validators 章节规则重写与测试重写）
- 验证：uv pytest 全量回归 + npm run check
- 提交 `feat: 叙事化提示词与 markdown 校验规则，受众 level 注入与范围放宽`

### 任务 20：e2e 收口

- 三端全绿 + 真库集成 + uvicorn mock 真进程（regenerate 流/markdown 正文/
  mermaid 码块/空内容 failed 路径）+ GUI 目验清单（用户手动）+ 真实 GLM 质量观察
- 提交（如有收尾）

## 每任务审查重点

1. **删型彻底性**：四章字段删除后无任何残留引用（grep 级检查；每处编译/live 测试）
2. **DROP TABLE 的依赖安全**：外键/级联不被破坏；ensure_schema 在「已建子表的历史 DB」
   与「全新 DB」两形态下都收敛到同一终态
3. **协议三端一次闭环**：Chapter 三字段键名/Option 一致；status 仅 Rust 层
4. **提示词-校验器-mock 口径对齐**（任务 19）；XSS（DOMPurify 红线，任务 18）
5. **空内容 failed 可重生成的判定链**（写入 status / 前端按钮 / 收尾函数全链）
