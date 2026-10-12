//! 三端协议的数据模型（camelCase JSON）。
//!
//! 与以下两处严格对齐（任何字段变更需三端同步）：
//! - frontend/src/types/agent.ts
//! - agent-service/app/schemas.py

use serde::{Deserialize, Serialize};

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

/// 章节核心概念定义
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConceptDef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub definition: String,
}

/// 章节知识点
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePointDef {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
}

/// 章节配图（Mermaid 源码由前端渲染）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDiagramDef {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub mermaid: String,
    #[serde(default)]
    pub caption: String,
}

/// 章节主要知识：概念 / 知识点 / 图示
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainKnowledgeDef {
    #[serde(default)]
    pub concepts: Vec<ConceptDef>,
    #[serde(default)]
    pub knowledge_points: Vec<KnowledgePointDef>,
    #[serde(default)]
    pub diagrams: Vec<KnowledgeDiagramDef>,
}

/// 章节深入内容：围绕主题的延伸长文
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeepDiveDef {
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub angle: String,
    #[serde(default)]
    pub content: String,
}

/// 阶段内的学习章节：主要知识 + 深入内容
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDef {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub main_knowledge: MainKnowledgeDef,
    #[serde(default)]
    pub deep_dives: Vec<DeepDiveDef>,
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
/// 尚无消费方：任务 10（流式消费 + Channel 进度）将使用，防御性保留。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanProgressEvent {
    /// 大纲就绪：阶段总数 + 计划标题
    Outline { phase_count: u32, title: String },
    /// 单个阶段扩展完成
    PhaseDone { index: u32, total: u32 },
    /// 存储层异常提示（生成不中断，前端降级提示）
    StorageIssue { message: String },
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
                    main_knowledge: MainKnowledgeDef {
                        concepts: vec![ConceptDef {
                            name: "所有权".into(),
                            definition: "值在同一时刻只有唯一所有者".into(),
                        }],
                        knowledge_points: vec![KnowledgePointDef {
                            title: "移动语义".into(),
                            detail: "赋值默认转移所有权".into(),
                        }],
                        diagrams: vec![KnowledgeDiagramDef {
                            title: "所有权转移".into(),
                            mermaid: "flowchart LR\n  a[变量a] -->|move| b[变量b]".into(),
                            caption: "move 后原变量失效".into(),
                        }],
                    },
                    deep_dives: vec![DeepDiveDef {
                        topic: "Drop 与 RAII".into(),
                        angle: "资源管理视角".into(),
                        content: "Rust 借 RAII 在编译期保证资源释放".into(),
                    }],
                }],
                project: PhaseProject {
                    name: "章节笔记站".into(),
                    description: "整理各章笔记".into(),
                    background: "补全项目背景".into(),
                    steps: vec!["搭建脚手架".into(), "填充内容".into()],
                    estimated_hours: 12,
                    acceptance_criteria: vec!["页面可访问".into()],
                },
            }],
            resources: vec![],
            review_notes: String::new(),
        };

        let json = serde_json::to_string(&plan).expect("序列化");
        let back: StudyPlan = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, plan);

        // 线上格式抽查：新字段 camelCase 嵌套
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let chapter = &value["phases"][0]["chapters"][0];
        assert_eq!(chapter["title"], "所有权与借用");
        assert_eq!(chapter["mainKnowledge"]["concepts"][0]["name"], "所有权");
        assert_eq!(
            chapter["mainKnowledge"]["knowledgePoints"][0]["title"],
            "移动语义"
        );
        assert_eq!(chapter["deepDives"][0]["topic"], "Drop 与 RAII");
        assert_eq!(value["phases"][0]["project"]["estimatedHours"], 12);
    }

    /// 进度事件（任务 10 消费）：type 标签 + camelCase 字段
    #[test]
    fn progress_event_uses_type_tag() {
        let outline = PlanProgressEvent::Outline {
            phase_count: 4,
            title: "计划标题".into(),
        };
        assert_eq!(
            serde_json::to_value(&outline).unwrap(),
            serde_json::json!({"type": "outline", "phaseCount": 4, "title": "计划标题"})
        );
        let parsed: PlanProgressEvent =
            serde_json::from_value(serde_json::to_value(&outline).unwrap()).unwrap();
        assert!(matches!(
            parsed,
            PlanProgressEvent::Outline { phase_count: 4, .. }
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
