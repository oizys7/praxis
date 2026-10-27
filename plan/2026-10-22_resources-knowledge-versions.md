# 执行计划：资源可信化 · 知识点覆盖 · 版本事实注入

基线：`dev/v0.1.1` @ `976aea2`（章型分叉 + 术语表 + 评审缩输入 + 资源如实标注）
日期：2026-10-22

## 背景与判断

现在的三个真实缺陷，按"地板 vs 天花板"排序：

1. **资源不可核查**：`Resource = {title, type, note}` 没有 URL。用户拿到"《Rust 程序设计语言》/ docs / 官方教程"点不开、不知版本、无法验证是否真实存在。模型凭记忆列书目必然产出一本听起来很真的书。
2. **正文浅而啰嗦**：每章生成时模型手里只有章标题 + 阶段目标 + 兄弟标题，没有新信息，只能重组训练集通用表述。同时 `MIN_CHAPTER_CONTENT_CHARS=1000` 与"1500–3000 字"在**奖励注水**，而各章重复解释同一概念是最大的重复来源。
3. **内容陈旧**：没有任何机制知道"这个库现在是什么版本"，模型只能用训练截止时的 API。

三件事都在"地板"层：不引入 RAG 基建、不引入向量、不引入新服务，纯程序化事实源 + prompt 约束 + Python 校验。

## 边界

| 归属 | 内容 |
|---|---|
| **本项目（学习器/生成器）** | 外部权威 API 探测（crates.io/npm/PyPI/GitHub）、资源可达性检查、知识点覆盖校验、章级素材注入**调用点** |
| **新项目（知识库）** | 爬虫 + 清洗 + 索引 + 向量 + 跨语言召回，通过 HTTP API 供本项目调用 |
| **后续排期** | 动画、RAG 库、向量接入 —— 见文末备注，本轮不实现 |

本项目只负责"消费"，不负责"维护语料"。理由：技术内容时效性极强，自建全网库会变成过时知识的放大器，而覆盖度永远追不上任意学习目标。

---

## 任务 A：资源可信化（url + 可达性 + 两档标注）

### 数据模型

`Resource` 增两字段：

| 字段 | 类型 | 语义 |
|---|---|---|
| `url` | str，默认 `""` | 资源真实地址；空串 = 模型没给（不编造） |
| `verification` | str，默认 `"unverified"` | `reachable` / `unverified` |

两档而不是"可信度分数"：**"有没有真的可达"是客观事实**，模型自评 confidence 只会全给 high，那是假精度。

### 可达性检查

`app/resource_check.py` 新增：

```
async def verify_resources(resources: list[dict]) -> list[dict]
```

- 并发 `httpx.HEAD`，单请求超时 3s，整体上限 8s
- `2xx/3xx` → `reachable`；空 url / 失败 / 超时 → `unverified`
- 任何异常都不得中断生成（fail-open，保持 `unverified`）
- 挂载点：`outline_node` 拿到 `PlanOutline` 后立即检查并回写 —— **不新增图节点**（沿用"能在已有调用里多要一个字段"的原则）

### 三端与 DB

- `schemas.py`：`Resource` 加两字段
- `protocol.rs`：`StudyResource` 加 `url` / `verification`（`#[serde(default)]`）
- `db_ddl.rs`：`resources` 表 CREATE 与 `ALTER TABLE ADD COLUMN IF NOT EXISTS` 双写两列，DEFAULT 分别为 `''` / `'unverified'`
- `plan_store.rs`：`upsert_resources` 写两列、读取两列、reassemble 带上
- `agent.ts`：`StudyResource` 加两字段
- `PlanResult.vue`：url 非空渲染为可点击链接（`target="_blank" rel="noopener noreferrer"`），徽标两档（已核实可达 / 未经核实）

### 提示词

`outline.yaml`：资源必须给真实 URL（官方文档 / 官方仓库 / 课程主页）；**拿不准就给空串，不许编造路径**；note 继续要求写明"用在哪一阶段/哪一章、补足什么"。

### 验收

- 单测：可达 → `reachable`；连接失败 → `unverified`；空 url → `unverified` 且不发请求
- mock 数据带真实 URL（如 `https://doc.rust-lang.org/book/`）
- 前端链接可点、两档徽标显示

---

## 任务 B：知识点驱动的双向覆盖校验

### 为什么先改主从关系

若"题目决定正文范围"，未出题的知识点就会被抛弃。所以：**知识点是主，题目是小节的校验器**。

知识点走**大纲并行数组**（与 `chapter_titles` / `chapter_kinds` 同一模式）：

```
PhaseOutline.chapter_knowledge_points: list[list[str]]   # 与 chapter_titles 等长同序
```

不进 `Chapter` wire → 前端零改动、DB 零改动。

### 双向覆盖校验（`validators._check_coverage`）

1. **正向**：每个知识点必须能映射到一个 `## ` 小节（宽松子串匹配，知识点里 ≥2 字的词出现在标题里即算命中）
2. **反向**：每个非固定小节（排除 学习目标/前置知识/动手练习/自检/深入/验收）必须能映射到某个知识点，否则记 `issue`（注水信号）
3. **题目**：自测小节内的题目数 ≥ 知识点数

失败策略（关键取舍）：命中率低于 `COVERAGE_MIN_RATIO = 0.6` 才判**章级失败**（触发定向回炉）。个别措辞差异只记 issue —— 措辞匹配天然不精确，触发回炉会白烧 LLM 调用，这是防误杀的阈值设计。

### 跨章去重

`outline.yaml` 要求知识点**全局不重复**，重复时由 `outline_node` 记入 `review_notes`（不改大纲、不额外调用），让评审与读者都能看到"这两章在讲同一件事"。

### 改动清单

- `schemas.py`：`PhaseOutline` 加 `chapter_knowledge_points`
- `outline.yaml`：产出 `chapterKnowledgePoints`，等长同序，每章 3–6 个，全局不重复
- `chapter.yaml`：`## ` 小节标题**必须原样使用知识点**；自测题数量 = 知识点数，每题标注对应知识点
- `nodes.py`：`_chapter_knowledge_points(phase_outline, idx)` 防御助手（越界/缺失回落空列表）；`_expand_chapter_draft` 注入变量
- `validators.py`：`_check_coverage` + 常量
- `mock.py`：生成知识点并让小节与之对齐
- 测试：覆盖校验 3–4 条、mock 结构、注入变量

### 验收

- pytest 全绿；mock 全链路知识点与小节对齐
- 覆盖率阈值行为正确（缺 1 个知识点不罚，缺一半判失败）

---

## 任务 C：版本事实注入（+ 素材注入钩子）

### C1 版本探测（完整实现，无需任何 API key）

1. `outline.yaml` 产出 `techStack: [{name, ecosystem}]`，`ecosystem ∈ crate | npm | pypi | github`
2. `app/version_check.py` 并发查询公开权威接口：
   - `https://crates.io/api/v1/crates/{name}` → `max_stable_version`
   - `https://registry.npmjs.org/{name}/latest` → `version`
   - `https://pypi.org/pypi/{name}/json` → `info.version`
   - `https://api.github.com/repos/{owner}/{repo}/releases/latest` → `tag_name`
3. **查不到的一律丢弃** —— 这同时是幻觉过滤器：模型编造的库名查不到。
4. 产物是"事实卡"：`{name, ecosystem, latestVersion}`，注入 `outline.yaml`（资源 note 必须写真实版本）与 `chapter.yaml`（正文必须用当前版本 API，禁止用已废弃写法）。

这一步一个机制治三个病：资源真实性、内容陈旧、库名幻觉。

延迟预算：3–8 个库并发，约 1–2s；全部失败时静默降级（不阻塞生成）。

### C2 素材注入钩子（接口 + 降级，不接真实 provider）

`app/search.py`：

```
class SearchProvider(Protocol):
    async def search(self, query: str, *, limit: int) -> list[SearchHit]: ...

def current_provider() -> SearchProvider | None   # 无配置 → None
```

- 有 provider：生成每章前按"章标题 + 知识点"检索，取 top-3 摘要注入 `research_snippets`，并要求"必须使用摘要中的具体细节；素材不足时明写需查阅 X，不许编"
- 无 provider：变量渲染为「（未启用联网检索）」，**行为与现在完全一致**
- `config.json` 增可选 `search.provider` / `search.apiKey`（模板留空）

这轮只交付抽象与降级路径；真实 provider 需要用户提供 key 后接入并验证。

### 改动清单

- 新增 `app/version_check.py`、`app/search.py`
- `config.py`：可选 `search` 段读取
- `schemas.py`：`PlanOutline` 加 `tech_stack`
- `outline.yaml` / `chapter.yaml`：新变量与硬约束
- `nodes.py`：`outline_node` 调探测并注入；`_expand_chapter_draft` 注入事实卡与素材
- 测试：版本探测（mock httpx、查不到丢弃、全失败降级）、无 provider 降级、变量注入

### 验收

- 无网络 / 无 key 时生成行为不变（回归不破）
- 有网络时正文出现真实版本号与当前 API
- 单测覆盖降级路径

---

## 执行顺序与提交

| 顺序 | 内容 | 提交 |
|---|---|---|
| 1 | 任务 A | `feat: 资源可核查——真实 URL + 可达性检查 + 两档标注` |
| 2 | 任务 B | `feat: 知识点双向覆盖校验——小节对齐与跨章去重` |
| 3 | 任务 C | `feat: 版本事实注入——权威接口探测当前版本并禁用废弃写法` |

每步提交前：`pytest` + `cargo test` + `cargo clippy --all-targets -- -D warnings` + `vue-tsc` + `npm run check` 全绿；提交用 `git commit -F`（多行消息），显式 pathspec，`plan/` 与 `.agents/` 不入库。

A 与 B/C 无耦合可独立上线；B 与 C 都改 `outline.yaml`，B 先落地以免冲突。

---

## 后续排期（本轮不做，写入 `docs/roadmap.md` 备查）

1. **RAG 知识库 → 新项目**：爬虫 + 清洗 + 索引 + 向量 + 跨语言召回（中→英文档），以 HTTP API 供本项目调用。硬前提：只收"稳定语料"（官方文档、经典书、RFC、语言规范），不爬全网；不引入"学习算法"，冷启动阶段没有行为数据可学。
2. **向量检索**：`pgvector-rust`（支持 SQLx，落在现有 PG）+ `fastembed`（本地 ONNX，无需 API key）。引入的唯一正当理由是**跨语言召回**（中文查询→英文文档），BM25 对标识符与错误信息已足够强。形态是 hybrid（双路召回 + RRF 融合），不是向量替代搜索。
3. **动画**：只做"数据结构操作"与"执行轨迹"两类（有确定性状态机）。红线：LLM 只输出**状态序列数据**，前端用预置渲染器按帧播放，绝不让 LLM 写动画代码。抽象概念（指针/所有权/生命周期）**不做**——硬做只会产出错误类比。中间态优先：Mermaid 分步高亮 + ASCII 状态序列，成本近零，覆盖大部分场景。
4. **音频：明确不做。** TTS 不改善文本质量，长文没人听。
5. 评估驱动生成的完整版（题目→正文的硬解耦、情景判断题防"API 手册化"）、学习闭环（今天学什么 / 打卡态）继续留在待办。

## 不做什么

- 不引入向量库、不建向量列、不写爬虫（属于新项目）
- 不做资源"可信度分数"（假精度）
- 不做章级资源绑定（`resources.phase_id`）—— 本轮用 note 说明用途，绑定等真实需求出现
- 不因为加了覆盖校验就调低字数下限（`MIN_CHAPTER_CONTENT_CHARS` 的注水问题单独一轮处理）
