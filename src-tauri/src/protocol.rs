//! 三端协议的数据模型（camelCase JSON）。
//!
//! 与以下两处严格对齐（任何字段变更需三端同步）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use serde::{Deserialize, Serialize};

/// 大纲阶段载荷（outline 事件携带）：仅骨架级三字段（计划 §3：
/// objectives/project_brief 不进事件，保持事件轻），seed_outline 按此播种骨架行。
/// project_brief 仅 regenerate_chapter 的重建请求载荷（RegenOutline）携带（任务 15
/// 裁定：单章上下文读既有 project 行 name 充当简介）；outline 事件恒 None 且
/// 序列化跳过，wire 只增不改。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutlinePhasePayload {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub week_range: String,
    #[serde(default)]
    pub chapter_titles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_brief: Option<String>,
    // 以下四项是「重生成上下文」而非骨架 UI 数据：骨架树只读 name/weekRange/
    // chapterTitles，但这些字段必须随 outline 事件下发，否则 Rust 无从持久化，
    // 重生成时章型会兜底成 concept、知识点为空、版本事实丢失（检索词也退化成
    // 只剩章标题）。全部 serde default，旧 Python 不发时按空处理。
    /// 与 chapter_titles 等长的章型列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapter_kinds: Vec<String>,
    /// 与 chapter_titles 等长的知识点列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapter_knowledge_points: Vec<Vec<String>>,
    /// 阶段目标
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objectives: Vec<String>,
    /// 版本事实（name / ecosystem / latestVersion），原样透传
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tech_facts: Vec<serde_json::Value>,
    /// 与 chapter_titles 等长的检索素材留档（每章一组），由 generation_artifacts
    /// 读回填入：重生成直接复用，不再重复调用检索服务。空数组 = 无留档，照常联网。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapter_research_snippets: Vec<Vec<serde_json::Value>>,
}

/// regenerate 端点的出站请求（Rust -> Python，POST /api/regenerate/stream；
/// 镜像 agent-service/app/schemas.py::RegenRequest）。
/// 仅 Serialize：Rust 是唯一发送方，不经 IPC 反序列化。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenRequest {
    /// 原始计划生成请求（plans 七字段回填，逐字段与原请求一致）
    pub request: PlanRequest,
    /// 计划大纲骨架（同一次生成的产出骨架重建）
    pub outline: RegenOutline,
    /// 目标阶段索引（0-based）
    pub phase_index: u32,
    /// 目标章节索引（0-based）；None 表示全阶段重生成
    pub chapter_index: Option<u32>,
}

/// 重建的计划大纲骨架（wire 对齐 agent-service/app/schemas.py::PlanOutline：
/// PhaseOutline 的 objectives/projectBrief 有缺省值，未携带时 Python 侧自动补齐）。
/// resources regenerate 链路不使用，恒为空列表。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegenOutline {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub estimated_total_hours: u32,
    #[serde(default)]
    pub phases: Vec<OutlinePhasePayload>,
}

/// 学习计划生成请求（前端 -> Rust -> Python）
/// Serialize：转发给 Python 时由 reqwest .json() 序列化；Deserialize：接收 Tauri IPC 参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    /// 学习目标
    pub goal: String,
    /// 当前水平：beginner | intermediate | advanced
    pub level: String,
    /// 每周可用小时数
    pub weekly_hours: u32,
    /// 期望总周数
    pub weeks: u32,
    /// 偏好方向（多选标签）
    #[serde(default)]
    pub interests: Vec<String>,
    /// 已有基础描述
    #[serde(default)]
    pub background: String,
    /// 每周学习日（1=周一..7=周日，需求 5）：None = 全周（缺字段/显式 null），
    /// 仅作日历分配元数据，不进 LLM 提示词；取值校验（1..=7 去重非空）
    /// 由 Python 校验层与 DB 层负责，serde 层只做缺省归一
    #[serde(default, with = "study_days_serde")]
    pub study_days: Option<Vec<u8>>,
}

/// studyDays 的 serde 双向语义（Task E）：反序列化缺字段/null → None（旧载荷
/// 兼容）；序列化 None 归一为全周显式数组——转发给 Python 的 wire 恒为数组，
/// CamelModel 校验层不再需要兑底 null。
mod study_days_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    /// 全周默认（周一..周日）
    fn full_week() -> [u8; 7] {
        [1, 2, 3, 4, 5, 6, 7]
    }

    pub(super) fn serialize<S: Serializer>(
        days: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match days {
            Some(v) => serializer.serialize_some(v),
            None => serializer.serialize_some(&full_week()),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<Vec<u8>>::deserialize(deserializer)
    }
}

/// 阶段内的学习章节：完整 markdown 正文（图示内联 ```mermaid 码块）。
/// status 为两级状态机章节位（pending | complete | failed）：流式载荷缺省为
/// None（无徽标语义），由 Rust 存储层按 content_md 定价；
/// content_md 缺省空串（旧流式载荷未携带时不阻塞解析，终态由 status 列承载）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisualizationStepDef {
    #[serde(default)]
    pub state: Vec<String>,
    #[serde(default)]
    pub highlight: Vec<i32>,
    #[serde(default)]
    pub note: String,
}

/// 一段过程动画：模型只给状态序列，渲染器由前端预置
///
/// 刻意不放「怎么画」的信息——数据错了看得见，代码错了直接崩，
/// 而崩的时候分不清是渲染问题还是理解问题。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisualizationDef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Vec<VisualizationStepDef>,
}

/// 结构化自测题：题目 + 它考的知识点 + 题型（recall / scenario）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseDef {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDef {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub content_md: String,
    /// 章型：concept / practice / project / review（空串按 concept 兜底）。
    /// 决定正文形态要求与前端口径，由大纲给定、生成时原样遵循
    #[serde(default)]
    pub kind: String,
    /// 本章核心术语（2-4 个）：计算属性列全计划术语表，不单独存储
    #[serde(default)]
    pub key_terms: Vec<String>,
    /// 过程动画（至多一段）：仅「执行过程不可见」的算法/数据结构章才有
    #[serde(default)]
    pub visualizations: Vec<VisualizationDef>,
    /// 结构化自测题：与正文「自检清单」同一批题，带知识点归属供程序校验
    #[serde(default)]
    pub exercises: Vec<ExerciseDef>,
    /// 只读派生：该章是否已打卡（由 study_schedule 聚合，Python 不产出）
    #[serde(default)]
    pub done: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
}

/// 阶段收尾项目：背景、步骤、产出与验收标准
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhaseProject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub estimated_hours: u32,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// 学习计划中的一个阶段：章节 / 实操 / 作业 / 项目
/// theory 为云端 PG plan_json 旧行只读兼容保留（只增不删演进策略），新数据不再产出
/// status 为两级状态机阶段位（pending | complete | failed）：旧数据/流式载荷缺省为
/// None（无徽标语义），wire 只增不改
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyPhase {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub week_range: String,
    #[serde(default)]
    pub objectives: Vec<String>,
    #[serde(default)]
    pub chapters: Vec<ChapterDef>,
    #[serde(default)]
    pub theory: Vec<String>,
    #[serde(default)]
    pub practice: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<String>,
    #[serde(default)]
    pub project: PhaseProject,
    #[serde(default)]
    pub status: Option<String>,
}

/// 推荐学习资源（type 字段与 Python 端对齐，Rust 侧改名避免关键字冲突）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyResource {
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub note: String,
    /// 资源真实地址；空串表示模型没有给出（禁止编造路径）
    #[serde(default)]
    pub url: String,
    /// 可达性：reachable / unverified（由 Python 侧真实探测得出，不信模型自评）
    #[serde(default)]
    pub verification: String,
}

/// 完整学习计划
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyPlan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub estimated_total_hours: u32,
    #[serde(default)]
    pub phases: Vec<StudyPhase>,
    #[serde(default)]
    pub resources: Vec<StudyResource>,
    #[serde(default)]
    pub review_notes: String,
    /// 全计划术语表：由各章 keyTerms 去重汇总（只读，不单独存储、不可编辑）
    #[serde(default)]
    pub glossary: Vec<GlossaryEntryDef>,
}

/// 术语表条目：术语名 + 首次出现的章节标题（wire 走 camelCase，与 Python 对齐）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntryDef {
    #[serde(default)]
    pub term: String,
    #[serde(default)]
    pub chapter_title: String,
}

/// 错误详情（Python 端返回的 envelope.error）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorDetail {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// 三端统一的响应包络：status = "plan" | "clarification" | "error"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanResponse {
    pub status: String,
    #[serde(default)]
    pub plan: Option<StudyPlan>,
    /// clarify 节点产出的澄清问题（status = "clarification" 时非空）
    #[serde(default)]
    pub clarifications: Option<Vec<String>>,
    #[serde(default)]
    pub error: Option<AgentErrorDetail>,
}

/// 学习计划历史记录（完整行：元数据 + 整份计划）。
/// 仅 Rust <-> 前端两端使用，不涉及 Python。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRecord {
    pub id: String,
    pub title: String,
    pub goal: String,
    /// 行级生成状态：generating | complete | failed（数据库字面量原样透传，
    /// 任务 24 补全：历史行徽章跨会话恢复，字段名单词 status 无 camel 歧义）
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub overview: String,
    /// 保存时的学习水平（冗余自 PlanRequest，列表页展示用）
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub weeks: u32,
    /// 每周学习日（1=周一..7=周日；Task E）。旧行列默认全周，读回恒为数组；
    /// serde default 兼容旧 wire 载荷（缺省空数组，前端按全周兑底）
    #[serde(default)]
    pub study_days: Vec<u8>,
    #[serde(default)]
    pub favorite: bool,
    /// 保存时间（ISO 8601）
    pub created_at: String,
    /// 整份计划
    pub plan: StudyPlan,
}

/// 用户自定义计划分组（列表页分区展示；plan_count 为组内计划数）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanGroup {
    pub id: String,
    pub name: String,
    /// 组内计划数（列表页标题上直接显示，省一次前端汇总）
    #[serde(default)]
    pub plan_count: i64,
}

/// 计划列表行（不含整份计划，列表页轻量加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub id: String,
    pub title: String,
    pub goal: String,
    /// 行级生成状态（同 PlanRecord.status，数据库字面量原样透传）
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub weeks: u32,
    /// 所属自定义分组 id（null = 未分组）
    #[serde(default)]
    pub group_id: Option<String>,
    /// 每周学习日（1=周一..7=周日；Task E，同 PlanRecord.study_days）
    #[serde(default)]
    pub study_days: Vec<u8>,
    /// 学习状态三分类：not_started（未开始）| active（学习中）| completed（已完成）
    #[serde(default)]
    pub learning_status: String,
    /// 开始学习日期（'YYYY-MM-DD'；未开始为 null）
    #[serde(default)]
    pub learning_started_at: Option<String>,
    /// 章节总数 / 已完成章节数（列表页完成度，免 get_plan；生成口径）
    #[serde(default)]
    pub chapter_total: u32,
    #[serde(default)]
    pub chapter_done: u32,
    /// 学习打卡进度（study_schedule 口径，打卡页专用）：已排日程总数
    /// （plans 冗余计数列，应用层在开始学习/重排事务内增量维护；存量 0）
    #[serde(default)]
    pub study_total: u32,
    /// 已打卡章数（plans 冗余计数列，打卡/取消打卡事务内增量维护）
    #[serde(default)]
    pub study_done: u32,
    #[serde(default)]
    pub favorite: bool,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// 学习日程（打卡日历真数据）
// ---------------------------------------------------------------------------

/// 日程条目：某计划某天的单章安排（行级打卡 done 独立存 study_schedule）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItem {
    pub plan_id: String,
    pub plan_title: String,
    pub phase_idx: u32,
    #[serde(default)]
    pub phase_name: String,
    pub chapter_idx: u32,
    #[serde(default)]
    pub chapter_title: String,
    pub done: bool,
}

/// 月历单日：该天全部计划的安排（查询接口按天组织）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthCalendarDay {
    /// 'YYYY-MM-DD'
    pub date: String,
    pub items: Vec<ScheduleItem>,
    pub total: u32,
    pub done_count: u32,
}

/// 年热力图单日强度（total/done 由前端映射色阶）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YearHeatmapDay {
    pub date: String,
    pub total: u32,
    pub done: u32,
}

// ---------------------------------------------------------------------------
// 章节化生成链路（流式进度 / LLM 评审结论）
// ---------------------------------------------------------------------------

/// POST /api/llm-check 的响应：生成前的 LLM 连通性探测结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmCheckResponse {
    /// 是否可用（Key 有效 / 网络可达 / 模型名存在）
    #[serde(default)]
    pub ok: bool,
    /// 可读的成败说明（失败时直接展示给用户）
    #[serde(default)]
    pub message: String,
    /// 探测时实际使用的提供商
    #[serde(default)]
    pub provider: String,
}

/// Rust -> 前端的生成进度事件（NDJSON 流式链路）。
/// tag = "type" 产出 {"type":"outline","phaseCount":…} 形式；
/// rename_all 作用于变体名，rename_all_fields 作用于字段名（serde >= 1.0.186）。
/// 消费方：commands::generate_plan 经 tauri::Channel 逐事件推送。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanProgressEvent {
    /// 大纲就绪：阶段总数 + 计划标题；phases/overview 为大纲骨架载荷
    /// （渐进式大纲渲染，任务 16 消费；旧调用方可缺省反序列化）
    Outline {
        phase_count: u32,
        title: String,
        #[serde(default)]
        overview: Option<String>,
        #[serde(default)]
        phases: Option<Vec<OutlinePhasePayload>>,
    },
    /// 单个阶段扩展完成
    PhaseDone { index: u32, total: u32 },
    /// 单章重生成完成：章级一等公民进度事件（阶段联动由 upsert_chapter
    /// 事务内推进；旧 PhaseDone{total:1} 变体保留，前端计数依赖它）
    ChapterDone {
        phase_index: u32,
        chapter_index: u32,
    },
    /// 章节开始生成（2026-10-16 逐章动画）：前端点亮该章「生成中」转圈
    ChapterStart {
        phase_index: u32,
        chapter_index: u32,
    },
    /// 存储层异常提示（生成不中断，前端降级提示）
    StorageIssue { message: String },
}

/// Python 流式端点的入站事件模型（消费方向：Python -> Rust，NDJSON 逐行）。
/// 镜像 agent-service/app/main.py `_stream_plan_events` / `_stream_regen_events`
/// 的事件集；与 [PlanProgressEvent]（Rust -> 前端出站）是两个方向的独立模型：
/// - {"type":"outline","phaseCount":N,"title":"…"}：骨架到达时 1 条；
/// - {"type":"phase","index":i,"phase":{…camelCase 阶段…}}：每阶段 1 条，
///   可乱序，回炉重生成时同 index 重复推送（覆盖语义）；
/// - {"type":"chapter","index":c,"phaseIndex":i,"chapter":{…}}：单章重生成
///   完成 1 条（/api/regenerate/stream chapterIndex 指定路径，任务 14）；
/// - {"type":"done","response":{…统一包络…}}：终态，必为最后一行。
#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StreamEvent {
    /// 大纲就绪：阶段总数 + 计划标题；phases/overview 为可选骨架载荷
    /// （任务 13 起由 Python outline 节点携带，缺省兼容旧载荷）
    Outline {
        phase_count: u32,
        title: String,
        #[serde(default)]
        overview: Option<String>,
        #[serde(default)]
        phases: Option<Vec<OutlinePhasePayload>>,
        /// 完整大纲（PlanOutline 原样）：落 plans.outline_json，供重生成还原
        /// 提示词上下文。旧 Python 不发此字段 → None，regen 回退骨架重建。
        #[serde(default)]
        outline: Option<serde_json::Value>,
    },
    /// 单个阶段扩展完成
    Phase { index: u32, phase: StudyPhase },
    /// 单章重生成完成（/api/regenerate/stream 的 chapterIndex 指定路径）：
    /// index 为章节索引、phase_index 为所属阶段索引（0-based）
    Chapter {
        index: u32,
        phase_index: u32,
        chapter: ChapterDef,
        /// 本次生成实际使用的检索素材（留档到 generation_artifacts 供重生成复用）。
        /// 不进 ChapterDef：它不是章节内容，前端也永远不读。
        #[serde(default)]
        research_snippets: Vec<serde_json::Value>,
        /// 提示词变量快照（审计用）。提示词是「模板 + 变量」的确定性函数，
        /// 所以存变量即可复原完整提示词，不必存渲染后文本。
        #[serde(default)]
        prompt_trace: Option<serde_json::Value>,
    },
    /// 章节开始生成（2026-10-16 逐章动画）：重生成流在每章 LLM 调用前发出，
    /// Rust 置该章 generating 落库并推前端转圈——状态退出重进可保持。
    /// phase_index 为 wire 完整性保留（Python 回显）；Rust 消费一律以入口
    /// 校验后的目标值为准，本字段不读——防御性豁免 dead_code。
    ChapterStart {
        #[allow(dead_code)]
        phase_index: u32,
        chapter_index: u32,
    },
    /// 终态：统一包络（plan / clarification / error），必为最后一行
    Done { response: PlanResponse },
}

/// NDJSON 行 -> 入站事件：空行 / 坏 JSON / 未知 type 一律返回 None，
/// 由调用方决定告警与跳过策略（坏行不中断流）。
pub fn parse_ndjson_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str(line).ok()
}

// ---------------------------------------------------------------------------
// 生成任务化协议（任务 22：提交即返 + WS 事件消费）
// ---------------------------------------------------------------------------

/// 后台生成任务提交体（Rust -> Python，POST /api/generations）。
/// 镜像 agent-service/app/schemas.py::GenerationSubmit（CamelModel，camelCase）：
/// kind 与载荷二选一严格配对，本任务只用 plan ↔ planRequest。
/// 仅 Serialize：Rust 是唯一发送方。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSubmit {
    /// 任务种类：本任务恒为 "plan"（regen 继续走流式端点）
    pub kind: String,
    /// plan 任务载荷（与既有 PlanRequest 同构）
    pub plan_request: PlanRequest,
}

/// 提交响应（Python -> Rust）：毫秒级返回的任务 id。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSubmitResponse {
    pub task_id: String,
}

/// 任务状态快照（Python -> Rust，GET /api/generations/{taskId}/snapshot）：
/// WS 连接前的诊断与断线重连游标依据（cursor 不得超过 lastSeq）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationTaskSnapshot {
    pub task_id: String,
    pub kind: String,
    /// 任务生命周期状态：running | finished | cancelled
    pub status: String,
    pub first_seq: u64,
    pub last_seq: u64,
    /// 事件日志是否已收尾（末条必为 done）
    pub done: bool,
}

/// WS 事件信封（Python -> Rust）：每条 WS 消息 {"seq": n, "event": {...}}，
/// event 与 NDJSON 行同构（StreamEvent）；seq 从 1 自增，回放/直播共用。
#[derive(Debug, Clone, Deserialize)]
pub struct WsEnvelope {
    pub seq: u64,
    pub event: StreamEvent,
}

/// WS 信封解析：坏 JSON / 缺字段 / event 内未知 type 一律 None（告警跳过）。
pub fn parse_ws_envelope(text: &str) -> Option<WsEnvelope> {
    serde_json::from_str(text).ok()
}

/// Rust -> 前端的全局进度事件信封（任务 22 起 'plan-progress' 事件的载荷）。
/// [PlanProgressEvent] 三变体形状保持不动，仅外包一层定位信息（planId 归属 /
/// kind 区分链路），前端经 tauri 全局 listen 按事件名收取。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanProgressEnvelope {
    /// 归属计划 id（generate_plan 预置行 id，与命令返回值一致）
    pub plan_id: String,
    /// 链路种类：generate 链路恒 "plan"（regen 仍走 Channel 通道）
    pub kind: String,
    /// 进度事件本体（Outline / PhaseDone / StorageIssue）
    pub payload: PlanProgressEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 云端 PG plan_json 旧行样例：含 theory 数组、无 chapters，project 无 steps/background/estimatedHours
    const LEGACY_PLAN_JSON: &str = r#"{
        "title": "Rust 入门",
        "overview": "旧版生成的计划",
        "estimatedTotalHours": 60,
        "phases": [
            {
                "name": "基础阶段",
                "weekRange": "第 1-3 周",
                "objectives": ["掌握基础语法"],
                "theory": ["所有权", "借用检查"],
                "practice": ["写 CLI 工具"],
                "assignments": ["完成练习 1"],
                "project": {
                    "name": "命令行待办",
                    "description": "实现一个待办管理工具",
                    "acceptanceCriteria": ["支持增删改查"]
                }
            }
        ],
        "resources": [
            { "title": "The Rust Book", "type": "docs", "note": "官方教程" }
        ],
        "reviewNotes": ""
    }"#;

    /// 旧 plan_json 旧行只读兼容：缺新字段反序列化不报错，theory 保留、chapters 为空
    #[test]
    fn old_plan_record_still_deserializes() {
        let plan: StudyPlan =
            serde_json::from_str(LEGACY_PLAN_JSON).expect("旧 plan_json 应可反序列化");
        assert_eq!(plan.estimated_total_hours, 60);
        let phase = &plan.phases[0];
        assert!(phase.chapters.is_empty(), "缺 chapters 字段时应为空 vec");
        assert_eq!(
            phase.theory,
            vec!["所有权".to_string(), "借用检查".to_string()]
        );
        assert_eq!(phase.project.name, "命令行待办");
    }

    /// 新章节化结构：序列化 -> 反序列化后字段相等，且线上 JSON 走 camelCase 嵌套
    #[test]
    fn new_plan_roundtrip() {
        let plan = StudyPlan {
            title: "Rust 进阶".into(),
            overview: "章节化新计划".into(),
            estimated_total_hours: 80,
            phases: vec![StudyPhase {
                name: "深入阶段".into(),
                week_range: "第 4-6 周".into(),
                objectives: vec!["吃透生命周期".into()],
                theory: vec![],
                practice: vec![],
                assignments: vec![],
                chapters: vec![ChapterDef {
                    title: "所有权与借用".into(),
                    summary: "深入 Rust 内存模型".into(),
                    kind: "concept".into(),
                    key_terms: vec!["所有权".into(), "借用".into()],
                    content_md: "## 所有权与借用\n\n```mermaid\nflowchart LR\n```".into(),
                    visualizations: Vec::new(),
                    exercises: Vec::new(),
                    done: None,
                    status: Some("complete".into()),
                }],
                project: PhaseProject {
                    name: "章节笔记站".into(),
                    description: "整理各章笔记".into(),
                    background: "补全项目背景".into(),
                    steps: vec!["搭建脚手架".into(), "填充内容".into()],
                    estimated_hours: 12,
                    acceptance_criteria: vec!["页面可访问".into()],
                },
                status: Some("complete".into()),
            }],
            resources: vec![],
            review_notes: String::new(),
            glossary: vec![GlossaryEntryDef {
                term: "所有权".into(),
                chapter_title: "所有权与借用".into(),
            }],
        };

        let json = serde_json::to_string(&plan).expect("序列化");
        let back: StudyPlan = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, plan);

        // 线上格式抽查：新字段 camelCase 嵌套
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let phase = &value["phases"][0];
        let chapter = &phase["chapters"][0];
        assert_eq!(chapter["title"], "所有权与借用");
        assert_eq!(
            chapter["contentMd"],
            "## 所有权与借用\n\n```mermaid\nflowchart LR\n```"
        );
        assert_eq!(phase["project"]["estimatedHours"], 12);
        // 章型与术语：camelCase 走线，术语表挂在计划级
        assert_eq!(chapter["kind"], "concept");
        assert_eq!(chapter["keyTerms"][0], "所有权");
        assert_eq!(value["glossary"][0]["chapterTitle"], "所有权与借用");
        // 两级状态机：status 走 camelCase，序列化不丢
        assert_eq!(phase["status"], "complete");
    }

    /// 两级状态机字段（任务 13）：缺省反序列化为 None（旧数据兼容），
    /// 显式值可无损回转
    #[test]
    fn two_level_status_defaults_and_roundtrip() {
        // 旧 JSON（无 status）：StudyPhase/ChapterDef 均反序列化为 None
        let old: StudyPhase = serde_json::from_str(
            r#"{"name":"旧阶段","weekRange":"第 1 周","chapters":[{"title":"旧章"}]}"#,
        )
        .expect("旧 JSON 应可反序列化");
        assert_eq!(old.status, None, "缺 status 应反序列化为 None");
        assert_eq!(old.chapters[0].status, None);

        // 新 JSON：status 显式携带
        let new: StudyPhase = serde_json::from_str(
            r#"{"name":"阶段","status":"failed","chapters":[{"title":"章","status":"pending"}]}"#,
        )
        .expect("新 JSON 应可反序列化");
        assert_eq!(new.status.as_deref(), Some("failed"));
        assert_eq!(new.chapters[0].status.as_deref(), Some("pending"));
    }

    /// OutlinePhasePayload（任务 13）：camelCase wire（chapterTitles）、
    /// 缺省字段反序列化不报错
    #[test]
    fn outline_phase_payload_wire() {
        let payload: OutlinePhasePayload =
            serde_json::from_str(r#"{"name":"阶段一","chapterTitles":["线程池","虚拟线程"]}"#)
                .expect("outline 载荷应可反序列化");
        assert_eq!(payload.week_range, "", "缺 weekRange 应反序列化为空串");
        assert_eq!(payload.chapter_titles.len(), 2);

        let value = serde_json::to_value(OutlinePhasePayload {
            name: "阶段一".into(),
            week_range: "第 1-4 周".into(),
            chapter_titles: vec!["甲".into()],
            project_brief: None,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value["chapterTitles"], serde_json::json!(["甲"]));
        assert_eq!(value["weekRange"], "第 1-4 周");
    }

    /// StreamEvent 行解析（任务 10 消费）：NDJSON 三种合法事件依次解析出正确变体与字段；
    /// 旧 outline 载荷（无 phases/overview）不受影响，新载荷（任务 13）可解
    #[test]
    fn parse_ndjson_line_accepts_three_events() {
        let outline = parse_ndjson_line(r#"{"type":"outline","phaseCount":4,"title":"Rust 入门"}"#)
            .expect("outline 事件行应解析成功");
        assert!(matches!(
            outline,
            StreamEvent::Outline {
                phase_count: 4,
                title,
                ref phases,
                ref overview,
                ..
            } if title == "Rust 入门" && phases.is_none() && overview.is_none()
        ));

        // 新 outline 载荷：phases/overview 携带（任务 14 Python 侧产出后接通）
        let rich = parse_ndjson_line(
            r#"{"type":"outline","phaseCount":1,"title":"T","overview":"概览","phases":[{"name":"阶段一","weekRange":"第 1 周","chapterTitles":["甲章"]}]}"#,
        )
        .expect("带骨架的 outline 事件行应解析成功");
        assert!(matches!(
            rich,
            StreamEvent::Outline { phases: Some(ps), overview: Some(ov), .. }
                if ps.len() == 1 && ps[0].chapter_titles == vec!["甲章".to_string()] && ov == "概览"
        ));

        let phase = parse_ndjson_line(
            r#"{"type":"phase","index":2,"phase":{"name":"进阶","weekRange":"第 3-4 周"}}"#,
        )
        .expect("phase 事件行应解析成功");
        match phase {
            StreamEvent::Phase { index, phase } => {
                assert_eq!(index, 2);
                assert_eq!(phase.name, "进阶");
            }
            other => panic!("应为 Phase 事件，实际：{other:?}"),
        }

        let done = parse_ndjson_line(r#"{"type":"done","response":{"status":"plan"}}"#)
            .expect("done 事件行应解析成功");
        match done {
            StreamEvent::Done { response } => assert_eq!(response.status, "plan"),
            other => panic!("应为 Done 事件，实际：{other:?}"),
        }
    }

    /// 单章重生成事件（任务 14 端点 chapterIndex 指定路径）：
    /// phaseIndex / contentMd 字段走 camelCase，chapter 为 markdown ChapterDef
    #[test]
    fn parse_ndjson_line_accepts_chapter_event() {
        let event = parse_ndjson_line(
            r###"{"type":"chapter","index":1,"phaseIndex":0,"chapter":{"title":"所有权与借用","summary":"深入 Rust 内存模型","contentMd":"## 所有权与借用\n\n```mermaid\nflowchart TD\n```"}}"###,
        )
        .expect("chapter 事件行应解析成功");
        match event {
            StreamEvent::Chapter {
                index,
                phase_index,
                chapter,
                research_snippets,
                prompt_trace,
            } => {
                assert_eq!(index, 1);
                assert_eq!(phase_index, 0);
                // 留档字段可选：旧载荷不带也要能解析（serde default）
                assert!(research_snippets.is_empty());
                assert!(prompt_trace.is_none());
                assert_eq!(chapter.title, "所有权与借用");
                assert!(chapter.content_md.starts_with("## 所有权与借用"));
                assert!(chapter.content_md.contains("```mermaid"));
            }
            other => panic!("应为 Chapter 事件，实际：{other:?}"),
        }
    }

    /// 章节开始事件（2026-10-16 逐章动画）：chapterStart + phaseIndex/chapterIndex
    #[test]
    fn parse_ndjson_line_accepts_chapter_start_event() {
        let event = parse_ndjson_line(r#"{"type":"chapterStart","phaseIndex":1,"chapterIndex":2}"#)
            .expect("chapterStart 事件行应解析成功");
        match event {
            StreamEvent::ChapterStart {
                phase_index,
                chapter_index,
            } => {
                assert_eq!(phase_index, 1);
                assert_eq!(chapter_index, 2);
            }
            other => panic!("应为 ChapterStart 事件，实际：{other:?}"),
        }
    }

    /// RegenRequest 出站 wire：request/outline 走 camelCase（phaseIndex/
    /// chapterIndex 字段名 + outline 骨架字段），单章 projectBrief 序列化进载荷
    #[test]
    fn regen_request_serializes_camel_case() {
        let body = RegenRequest {
            request: PlanRequest {
                goal: "学 Rust".into(),
                level: "intermediate".into(),
                weekly_hours: 12,
                weeks: 8,
                interests: vec!["并发".into()],
                background: "写过 Go".into(),
                study_days: Some(vec![1, 3, 5]),
            },
            outline: RegenOutline {
                title: "T".into(),
                overview: "O".into(),
                estimated_total_hours: 96,
                phases: vec![OutlinePhasePayload {
                    name: "阶段一".into(),
                    week_range: "第 1-2 周".into(),
                    chapter_titles: vec!["甲章".into(), "乙章".into()],
                    project_brief: Some("做章节笔记站".into()),
                    ..Default::default()
                }],
            },
            phase_index: 0,
            chapter_index: Some(1),
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["request"]["weeklyHours"], 12);
        assert_eq!(value["request"]["level"], "intermediate");
        assert_eq!(value["outline"]["estimatedTotalHours"], 96);
        assert_eq!(
            value["outline"]["phases"][0]["chapterTitles"],
            serde_json::json!(["甲章", "乙章"])
        );
        assert_eq!(
            value["outline"]["phases"][0]["projectBrief"],
            "做章节笔记站"
        );
        assert_eq!(value["phaseIndex"], 0);
        assert_eq!(value["chapterIndex"], 1);

        // 全阶段重生成：chapterIndex 缺省为 null（Python 侧 int | None 收 None）
        let phase_regen = serde_json::to_value(RegenRequest {
            chapter_index: None,
            ..body
        })
        .unwrap();
        assert!(phase_regen["chapterIndex"].is_null());
        // projectBrief 缺省（regenerate_phase 裁定传空上下文）：不序列化进载荷
        let no_brief = serde_json::to_value(RegenOutline {
            phases: vec![OutlinePhasePayload {
                project_brief: None,
                ..OutlinePhasePayload::default()
            }],
            ..RegenOutline::default()
        })
        .unwrap();
        assert!(no_brief["phases"][0].get("projectBrief").is_none());
    }

    /// 空行 / 坏 JSON / 未知 type 一律 None（调用方告警跳过，不中断流）
    #[test]
    fn parse_ndjson_line_rejects_junk_lines() {
        assert!(parse_ndjson_line("").is_none(), "空行应返回 None");
        assert!(parse_ndjson_line("   ").is_none(), "纯空白行应返回 None");
        assert!(
            parse_ndjson_line("{not-json").is_none(),
            "坏 JSON 应返回 None"
        );
        assert!(
            parse_ndjson_line("null").is_none(),
            "非对象 JSON 应返回 None"
        );
        assert!(
            parse_ndjson_line(r#"{"type":"unknown","x":1}"#).is_none(),
            "未知 type 应返回 None"
        );
    }

    /// Windows 行尾：行尾 \r（含 \r\n 分帧产物）不应影响解析
    #[test]
    fn parse_ndjson_line_tolerates_trailing_cr() {
        let event =
            parse_ndjson_line("{\"type\":\"outline\",\"phaseCount\":2,\"title\":\"标题\"}\r");
        assert!(
            matches!(event, Some(StreamEvent::Outline { phase_count: 2, .. })),
            "尾随 \\r 的事件行应容忍"
        );
    }

    /// 进度事件（任务 10 消费）：type 标签 + camelCase 字段；
    /// outline 骨架载荷（任务 13）走 camelCase（chapterTitles）且 None 时字段可为 null
    #[test]
    fn progress_event_uses_type_tag() {
        let outline = PlanProgressEvent::Outline {
            phase_count: 4,
            title: "计划标题".into(),
            overview: Some("整体概述".into()),
            phases: Some(vec![OutlinePhasePayload {
                name: "阶段一".into(),
                week_range: "第 1-2 周".into(),
                chapter_titles: vec!["甲章".into()],
                project_brief: None,
                ..Default::default()
            }]),
        };
        let value = serde_json::to_value(&outline).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type": "outline", "phaseCount": 4, "title": "计划标题",
                "overview": "整体概述",
                "phases": [{"name": "阶段一", "weekRange": "第 1-2 周", "chapterTitles": ["甲章"]}]})
        );
        let parsed: PlanProgressEvent = serde_json::from_value(value).unwrap();
        assert!(matches!(
            parsed,
            PlanProgressEvent::Outline { phase_count: 4, phases: Some(ps), .. }
                if ps.len() == 1 && ps[0].name == "阶段一"
        ));

        // 旧调用方（无骨架）：None 载荷照常序列化/反序列化
        let bare = PlanProgressEvent::Outline {
            phase_count: 2,
            title: "旧调用".into(),
            overview: None,
            phases: None,
        };
        let parsed: PlanProgressEvent =
            serde_json::from_value(serde_json::to_value(&bare).unwrap()).unwrap();
        assert!(matches!(
            bare,
            PlanProgressEvent::Outline {
                phases: None,
                overview: None,
                ..
            }
        ));
        assert!(matches!(
            parsed,
            PlanProgressEvent::Outline { phase_count: 2, .. }
        ));

        let done = PlanProgressEvent::PhaseDone { index: 1, total: 4 };
        assert_eq!(
            serde_json::to_value(&done).unwrap(),
            serde_json::json!({"type": "phaseDone", "index": 1, "total": 4})
        );

        let issue = PlanProgressEvent::StorageIssue {
            message: "存储不可用".into(),
        };
        assert_eq!(
            serde_json::to_value(&issue).unwrap(),
            serde_json::json!({"type": "storageIssue", "message": "存储不可用"})
        );
    }

    /// 章级进度事件 wire 形态（task-B）：与 PhaseDone 同构 camelCase 变体，
    /// 序列化字段名 / 反序列化回旋双锚定
    #[test]
    fn chapter_done_progress_event_wire_form() {
        let chapter_done = PlanProgressEvent::ChapterDone {
            phase_index: 2,
            chapter_index: 3,
        };
        assert_eq!(
            serde_json::to_value(&chapter_done).unwrap(),
            serde_json::json!({"type": "chapterDone", "phaseIndex": 2, "chapterIndex": 3})
        );
        let parsed: PlanProgressEvent =
            serde_json::from_value(serde_json::to_value(&chapter_done).unwrap()).unwrap();
        assert!(matches!(
            parsed,
            PlanProgressEvent::ChapterDone {
                phase_index: 2,
                chapter_index: 3,
            }
        ));
    }

    /// WS 信封解析（任务 22）：{"seq": n, "event": {...}}，event 与 NDJSON 同构；
    /// seq 缺失 / 结构破坏 / event 未知 type 一律 None
    #[test]
    fn ws_envelope_parse_aligns_with_ndjson_events() {
        let phase = parse_ws_envelope(
            r#"{"seq":3,"event":{"type":"phase","index":0,"phase":{"name":"进阶"}}}"#,
        )
        .expect("phase 信封应解析成功");
        assert_eq!(phase.seq, 3);
        match phase.event {
            StreamEvent::Phase { index, phase } => {
                assert_eq!(index, 0);
                assert_eq!(phase.name, "进阶");
            }
            other => panic!("应为 Phase 事件，实际：{other:?}"),
        }

        let done =
            parse_ws_envelope(r#"{"seq":9,"event":{"type":"done","response":{"status":"plan"}}}"#)
                .expect("done 信封应解析成功");
        match done.event {
            StreamEvent::Done { response } => assert_eq!(response.status, "plan"),
            other => panic!("应为 Done 事件，实际：{other:?}"),
        }

        // event 复用既有 StreamEvent 载荷（含可选骨架缺省）
        let outline = parse_ws_envelope(
            r#"{"seq":1,"event":{"type":"outline","phaseCount":4,"title":"Rust 入门"}}"#,
        )
        .expect("outline 信封应解析成功");
        assert!(matches!(
            outline.event,
            StreamEvent::Outline { phase_count: 4, .. }
        ));

        // 非信封 / 缺 seq / event 坏行一律 None
        assert!(parse_ws_envelope("{not-json").is_none());
        assert!(
            parse_ws_envelope(r#"{"event":{"type":"done"}}"#).is_none(),
            "缺 seq 应 None"
        );
        assert!(
            parse_ws_envelope(r#"{"seq":1,"event":{"type":"unknown"}}"#).is_none(),
            "未知 event 应 None"
        );
    }

    /// 提交体 wire：kind + planRequest（camelCase），与 Python GenerationSubmit
    /// （CamelModel）逐字段对齐
    #[test]
    fn generation_submit_serializes_camel_case() {
        let body = GenerationSubmit {
            kind: "plan".into(),
            plan_request: PlanRequest {
                goal: "学 Rust".into(),
                level: "intermediate".into(),
                weekly_hours: 12,
                weeks: 8,
                interests: vec!["并发".into()],
                background: "写过 Go".into(),
                study_days: None,
            },
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["kind"], "plan");
        assert!(
            !value
                .get("regenRequest")
                .map(|v| !v.is_null())
                .unwrap_or(false),
            "plan 提交不带 regenRequest"
        );
        assert_eq!(value["planRequest"]["weeklyHours"], 12);
        // studyDays None 序列化归一为全周显式数组（study_days_serde）
        assert_eq!(
            value["planRequest"]["studyDays"],
            serde_json::json!([1, 2, 3, 4, 5, 6, 7])
        );
        assert_eq!(
            value["planRequest"]["interests"],
            serde_json::json!(["并发"])
        );
    }

    /// 全局进度事件信封（任务 22）：{planId, kind, payload}，payload 三变体形状不动
    #[test]
    fn plan_progress_envelope_wraps_payload() {
        let envelope = PlanProgressEnvelope {
            plan_id: "uuid-1".into(),
            kind: "plan".into(),
            payload: PlanProgressEvent::PhaseDone { index: 0, total: 4 },
        };
        let value = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"planId": "uuid-1", "kind": "plan",
                "payload": {"type": "phaseDone", "index": 0, "total": 4}})
        );
    }

    /// 提交响应与任务快照反序列化：Python 端 camelCase（taskId/firstSeq/lastSeq）
    #[test]
    fn task_meta_models_deserialize_camel_case() {
        let submitted: GenerationSubmitResponse =
            serde_json::from_str(r#"{"taskId":"abc123"}"#).expect("提交响应应可反序列化");
        assert_eq!(submitted.task_id, "abc123");

        let snapshot: GenerationTaskSnapshot = serde_json::from_str(
            r#"{"taskId":"abc123","kind":"plan","status":"running","firstSeq":1,"lastSeq":5,"done":false}"#,
        )
        .expect("快照应可反序列化");
        assert_eq!(snapshot.task_id, "abc123");
        assert_eq!(snapshot.kind, "plan");
        assert_eq!(snapshot.status, "running");
        assert_eq!(snapshot.last_seq, 5);
        assert!(!snapshot.done);
    }

    /// 行级生成状态（任务 24）：PlanSummary/PlanRecord 携带数据库字面量
    /// generating | complete | failed，蛇形/小写原样透传（不 rename 为 camel）；
    /// 旧 wire（无 status 字段）反序列化不报错（serde default 兼容）
    #[test]
    fn plan_summary_and_record_carry_row_status_verbatim() {
        let summary: PlanSummary = serde_json::from_str(
            r#"{"id":"uuid-1","title":"T","goal":"G","createdAt":"2026-01-01T00:00:00Z","status":"generating"}"#,
        )
        .expect("PlanSummary 应可反序列化");
        assert_eq!(summary.status, "generating");

        // 字面量原样：failed / generating 不改写（与蛇形枚举词的价值一致）
        let value = serde_json::to_value(PlanSummary {
            id: "uuid-2".into(),
            title: "T".into(),
            goal: "G".into(),
            status: "failed".into(),
            overview: "O".into(),
            level: "beginner".into(),
            weeks: 8,
            group_id: None,
            study_days: vec![2, 4, 6],
            learning_status: "not_started".into(),
            learning_started_at: None,
            chapter_total: 12,
            chapter_done: 5,
            study_total: 10,
            study_done: 3,
            favorite: false,
            created_at: "2026-01-01T00:00:00Z".into(),
        })
        .unwrap();
        assert_eq!(value["status"], "failed", "status 字面量透传不改写");
        assert_eq!(
            value["studyDays"],
            serde_json::json!([2, 4, 6]),
            "studyDays camelCase wire（Task E）"
        );

        // 旧 wire：无 status 字段（经验库/旧前端快照）缺省反序列化为空串不炸
        let legacy: PlanSummary =
            serde_json::from_str(r#"{"id":"u","title":"T","goal":"G","createdAt":"c"}"#)
                .expect("旧 wire 应兼容");
        assert_eq!(legacy.status, "");

        // PlanRecord 同口径：携带行级 status
        let record: PlanRecord = serde_json::from_str(
            r#"{"id":"uuid-1","title":"T","goal":"G","createdAt":"2026-01-01T00:00:00Z","plan":{},"status":"complete"}"#,
        )
        .expect("PlanRecord 应可反序列化");
        assert_eq!(record.status, "complete");
        let plan_record_value = serde_json::to_value(PlanRecord {
            id: "uuid-3".into(),
            title: "T".into(),
            goal: "G".into(),
            status: "generating".into(),
            overview: "O".into(),
            level: "beginner".into(),
            weeks: 8,
            study_days: vec![1, 2, 3, 4, 5, 6, 7],
            favorite: false,
            created_at: "2026-01-01T00:00:00Z".into(),
            plan: StudyPlan::default(),
        })
        .unwrap();
        assert_eq!(plan_record_value["status"], "generating");
        assert_eq!(
            plan_record_value["studyDays"],
            serde_json::json!([1, 2, 3, 4, 5, 6, 7]),
            "PlanRecord.studyDays camelCase wire（Task E）"
        );
    }
}
