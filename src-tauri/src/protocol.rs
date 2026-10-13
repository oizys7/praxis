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
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
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
#[derive(Debug, Serialize, Deserialize)]
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
}

/// 阶段内的学习章节：完整 markdown 正文（图示内联 ```mermaid 码块）。
/// status 为两级状态机章节位（pending | complete | failed）：流式载荷缺省为
/// None（无徽标语义），由 Rust 存储层按 content_md 定价；
/// content_md 缺省空串（旧流式载荷未携带时不阻塞解析，终态由 status 列承载）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDef {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub content_md: String,
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

/// 保存学习计划请求（前端 -> Rust）：id 与保存时间由 Rust 端生成，不在请求内。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePlanRequest {
    pub title: String,
    pub goal: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub weeks: u32,
    pub plan: StudyPlan,
}

/// 学习计划历史记录（完整行：元数据 + 整份计划）。
/// 仅 Rust <-> 前端两端使用，不涉及 Python。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRecord {
    pub id: String,
    pub title: String,
    pub goal: String,
    #[serde(default)]
    pub overview: String,
    /// 保存时的学习水平（冗余自 PlanRequest，列表页展示用）
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub weeks: u32,
    #[serde(default)]
    pub favorite: bool,
    /// 保存时间（ISO 8601）
    pub created_at: String,
    /// 整份计划
    pub plan: StudyPlan,
}

/// 计划列表行（不含整份计划，列表页轻量加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub id: String,
    pub title: String,
    pub goal: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub weeks: u32,
    #[serde(default)]
    pub favorite: bool,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// 章节化生成链路（流式进度 / LLM 评审结论）
// ---------------------------------------------------------------------------

/// review 节点的 LLM 评审结论。
/// 镜像 Python 端 ReviewOutput（BaseModel，非 CamelModel）：JSON 为 snake_case，
/// 仅图内流转 / 入库，不入响应包络，故不加 rename_all。
/// 尚无消费方：任务 9（入库）将使用，防御性保留。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewOutputDef {
    #[serde(default)]
    pub passed: bool,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub suggestions: Vec<String>,
    /// 评审未通过的阶段索引（0-based）
    #[serde(default)]
    pub failed_phase_indexes: Vec<i64>,
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
    },
    /// 单个阶段扩展完成
    Phase { index: u32, phase: StudyPhase },
    /// 单章重生成完成（/api/regenerate/stream 的 chapterIndex 指定路径）：
    /// index 为章节索引、phase_index 为所属阶段索引（0-based）
    Chapter {
        index: u32,
        phase_index: u32,
        chapter: ChapterDef,
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
                    content_md: "## 所有权与借用\n\n```mermaid\nflowchart LR\n```".into(),
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
            StreamEvent::Outline { phase_count: 4, title, ref phases, ref overview }
                if title == "Rust 入门" && phases.is_none() && overview.is_none()
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
            } => {
                assert_eq!(index, 1);
                assert_eq!(phase_index, 0);
                assert_eq!(chapter.title, "所有权与借用");
                assert!(chapter.content_md.starts_with("## 所有权与借用"));
                assert!(chapter.content_md.contains("```mermaid"));
            }
            other => panic!("应为 Chapter 事件，实际：{other:?}"),
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
}
