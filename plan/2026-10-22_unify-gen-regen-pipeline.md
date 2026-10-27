# 生成 / 重生成统一为一条流水线

基线：`dev/v0.1.1 @ ef76d8f`。用户裁定（2026-10-22）：三步（素材复用、程序化校验、LLM 评审）都要，
且「信息准备完毕之后走同一套逻辑，复用同一套代码，逻辑统一、维护统一」。

## 现状：叶子共用，编排两套

| 层 | 首次生成 | 重新生成 |
| --- | --- | --- |
| 编排驱动 | LangGraph `astream`（Send 扇出） | `_regen_event_dicts` 手写 `for` 循环 |
| 并发 | `Send` 并行，`_EXPAND_SEMAPHORE=3` | 串行 `await` |
| 章节生产 | `_expand_chapter_draft` | **同一个函数** ✅ |
| 阶段轻装 | `_expand_phase_extras` | **同一个函数** ✅ |
| 程序化校验 | `assemble_node` → `validators` | 无 |
| LLM 评审 | `review_node` | 无 |
| 失败自愈 | `expand_revision_node` 定向重扩一次 | 无 |
| 搜索素材 | 现场搜 Tavily | 现场搜 Tavily（不复用） |

结论：**编排重复是唯一的结构性问题**，它顺带导致闸门缺失。

## 方案：regen 成为图的第二个入口

```
_entry_router(state)
  ├─ state.regen_scope 存在 → [Send expand_chapter…, Send expand_phase_wrap]   ← 按 scope 过滤
  └─ 否则                   → clarify → outline → 同一条扇出
        ↓
    assemble（scope 化校验）→ review（scope 化摘要）→ 有 pending → expand_revision → assemble → END
```

两条路径此后共用 `assemble / review / expand_revision / 全部叶子函数 / 事件翻译`，
`tasks._regen_event_dicts` 删除，regen 端点直接调 `_plan_event_dicts(request, outline, scope)`。

**为什么不做成两个共享函数而是并进图**：闸门本身就是图里的节点。抽函数只能复用「章生产」，
校验/评审/回炉的拓扑顺序、回炉上限、事件时序仍要各写一遍，正是当前不一致的来源。

## 改动清单

### A. 搜索素材持久化（三步之一）

1. `chapters.research_snippets JSONB NOT NULL DEFAULT '[]'`（db_ddl 独立 ALTER 常量）。
2. `_expand_chapter_draft` 返回 `(chapter, snippets)`；`snippets` 是本次实际使用的素材
   （复用命中则为旧素材），供上游落库。
3. `AgentState.chapter_snippets`（`or_` reducer，键同 `chapter_drafts`）；`expand_chapter`
   与 `expand_revision` 都写它；`assemble` 不需要它。
4. 事件层：`chapter` 事件加兄弟字段 `researchSnippets`（**不进 `ChapterDef`**，前端 wire 不变，
   与 `outline` 事件加 `outline` 字段同一手法）。
5. Rust：`StreamEvent::Chapter` 加 `research_snippets: Vec<Value>`（serde default）；
   `upsert_chapter` 写列；`load_plan_outline` 顺带读回每章素材，塞进
   `OutlinePhasePayload.chapter_research_snippets`（并行动数组，与 `chapter_kinds` 同模式）。
6. Python `_chapter_research_snippets(phase_outline, ci)` → 有值时跳过 Tavily，直接用。

### B. regen 并入图（三步之二、三 + 编排统一）

7. `AgentState.regen_scope: {phase: int, chapters: list[int] | None}`。
8. `graph.py`：`set_conditional_entry_point(_entry_router)`；`_fan_out_chapters` 按 scope 过滤
   （单章 → 只发一个 `expand_chapter`，不发 wrap）。
9. `nodes.outline_node`：`state` 已有 `outline` 时直接返回 `{}`（0 次 LLM，regen 用库里的）。
10. `nodes.assemble_node`：scope 化——只拼装/校验 scope 内的阶段与章节，scope 外**不补空占位、
    不判失败**（否则单章 regen 会把整阶段的缺失章判成失败并连带回炉）。
11. `validators.validate_plan(..., scope_phases=...)`：跳过非 scope 阶段（学时算术等计划级规则
    在 scope 下只对 scope 内阶段负责）。
12. `review_node` / `_plan_summary`：摘要只覆盖 scope；`review.yaml` 措辞从「整计划」改为
    「给定范围」，单章 regen 时范围即该章。
13. `expand_chapter_node`：用 `get_stream_writer()` 发 `chapterStart`（LangGraph custom stream
    mode），保住逐章转圈动画；`tasks` 侧 astream 增 `"custom"` mode 并翻译。
14. `_plan_event_dicts(request, *, outline=None, regen_scope=None)`：outline 已在 state 时
    不发 outline 事件（regen 前端不需要骨架重建）。
15. 删除 `tasks._regen_event_dicts`；`/api/regenerate/stream` 改调统一生成器。

## 风险与对策

- **首次生成零回归**：改动集中在 scope 分支，`regen_scope` 为空时行为与现在逐字节一致；
  193 个 pytest + mock e2e 兜底。
- **chapterStart 依赖 custom stream**：langgraph 1.2 支持 `get_stream_writer()`；若实测不可用，
  退化为「Rust 收到 chapter 事件时补推 ChapterDone」——动画粒度从章开始变成章完成，可接受。
- **单章 regen 的 review**：范围只有一章，评审维度退化为「结构/深度/知识点落地」，提示词已注明
  看不到全局，不会误判。

## 验证

`pytest`（含新增：regen 走图后的校验/回炉/素材复用用例、scope 化 validators）
→ `cargo test` + 真实 DB → mock e2e（regen 全阶段/单章事件序列）
→ `npm run check` 七步 → 真 GLM 跑一次单章 regen（可选，需 key）。
