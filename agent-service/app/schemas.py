"""Pydantic 数据模型：Vue / Rust / Python 三端统一的请求与响应结构。

约定：
- JSON 层字段一律使用 camelCase（通过 alias_generator 实现）；
- Python 代码内部仍使用 snake_case，读写均可（populate_by_name=True）。
- 对齐文件：src-tauri/src/commands.rs、frontend/src/types/agent.ts
"""

from __future__ import annotations

from enum import Enum
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


def to_camel(name: str) -> str:
    """snake_case -> camelCase，如 weekly_hours -> weeklyHours。"""
    head, *rest = name.split("_")
    return head + "".join(word.capitalize() for word in rest)


class CamelModel(BaseModel):
    """所有对外模型基类：JSON 走 camelCase，Python 内用 snake_case。"""

    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)


# ---------------------------------------------------------------------------
# 请求
# ---------------------------------------------------------------------------


class Level(str, Enum):
    """学习者当前水平。"""

    beginner = "beginner"
    intermediate = "intermediate"
    advanced = "advanced"


class PlanRequest(CamelModel):
    """学习计划生成请求（前端 -> Tauri -> Python）。"""

    goal: str = Field(..., min_length=1, description="学习目标")
    level: Level = Field(default=Level.beginner, description="当前水平")
    weekly_hours: int = Field(default=10, ge=1, le=80, description="每周可用小时数")
    weeks: int = Field(default=12, ge=1, le=52, description="期望总周数")
    interests: list[str] = Field(
        default_factory=list, description="偏好方向（多选标签）"
    )
    background: str = Field(default="", description="已有基础描述")


class RegenRequest(CamelModel):
    """阶段/章节级重生成请求（Rust -> Python，流式端点 /api/regenerate/stream）。

    chapterIndex 为 None 时重生成整个阶段（返回 StudyPhase），
    指定时只重生成单个章节（返回 Chapter）；上下文由调用方从入库的
    plans 七字段 + 大纲骨架重建（生成条件可完整复现，见任务 15）。
    """

    request: PlanRequest = Field(..., description="原始计划生成请求（重建回填）")
    outline: PlanOutline = Field(..., description="计划大纲骨架（同一次生成的产出）")
    phase_index: int = Field(..., ge=0, description="目标阶段索引（0-based）")
    chapter_index: int | None = Field(
        default=None, ge=0, description="目标章节索引（0-based）；None 表示全阶段"
    )


# ---------------------------------------------------------------------------
# 响应中的计划结构
# ---------------------------------------------------------------------------


class PhaseProject(CamelModel):
    """阶段收尾项目：背景、步骤、产出与验收标准。"""

    name: str = Field(..., description="项目名称")
    description: str = Field(default="", description="项目描述")
    background: str = Field(default="", description="项目背景与动机")
    steps: list[str] = Field(default_factory=list, description="建议实施步骤（有序）")
    estimated_hours: int = Field(default=0, description="预计投入学时")
    acceptance_criteria: list[str] = Field(
        default_factory=list, description="可验证的验收标准"
    )


class ConceptDef(CamelModel):
    """章节核心概念定义。"""

    name: str = Field(..., description="概念名称")
    definition: str = Field(default="", description="一句话定义")


class KnowledgePoint(CamelModel):
    """章节知识点。"""

    title: str = Field(..., description="知识点标题")
    detail: str = Field(default="", description="要点说明")


class KnowledgeDiagram(CamelModel):
    """章节配图：Mermaid 源码由前端渲染。"""

    title: str = Field(default="", description="图标题")
    mermaid: str = Field(default="", description="Mermaid 源码")
    caption: str = Field(default="", description="图注说明")


class MainKnowledge(CamelModel):
    """章节主要知识：概念 / 知识点 / 图示。"""

    concepts: list[ConceptDef] = Field(default_factory=list, description="核心概念定义")
    knowledge_points: list[KnowledgePoint] = Field(
        default_factory=list, description="知识点列表"
    )
    diagrams: list[KnowledgeDiagram] = Field(
        default_factory=list, description="Mermaid 图示"
    )


class DeepDive(CamelModel):
    """章节深入内容：围绕主题的延伸长文。"""

    topic: str = Field(..., description="深入主题")
    angle: str = Field(default="", description="切入角度")
    content: str = Field(default="", description="正文长文")


class Chapter(CamelModel):
    """阶段内的学习章节：主要知识 + 深入内容。"""

    title: str = Field(..., description="章节标题")
    summary: str = Field(default="", description="章节概述")
    main_knowledge: MainKnowledge = Field(
        default_factory=MainKnowledge, description="本章主要知识"
    )
    deep_dives: list[DeepDive] = Field(
        default_factory=list, description="深入内容列表"
    )


class StudyPhase(CamelModel):
    """学习计划的一个阶段：章节 / 实操 / 作业 / 项目。"""

    name: str = Field(..., description="阶段名称")
    week_range: str = Field(default="", description="周区间，如 '第 1-3 周'")
    objectives: list[str] = Field(default_factory=list, description="阶段目标")
    chapters: list[Chapter] = Field(
        default_factory=list, description="章节列表（替代旧 theory）"
    )
    practice: list[str] = Field(default_factory=list, description="动手实操任务")
    assignments: list[str] = Field(default_factory=list, description="课后作业")
    project: PhaseProject | None = Field(default=None, description="阶段项目")


class Resource(CamelModel):
    """推荐学习资源。"""

    title: str = Field(..., description="资源名称")
    type: str = Field(default="docs", description="类型：book/course/docs/video/other")
    note: str = Field(default="", description="推荐理由")


class StudyPlan(CamelModel):
    """完整学习计划。"""

    title: str = Field(..., description="计划标题")
    overview: str = Field(default="", description="整体概述")
    estimated_total_hours: int = Field(default=0, description="预计总学时")
    phases: list[StudyPhase] = Field(default_factory=list, description="阶段列表")
    resources: list[Resource] = Field(default_factory=list, description="推荐资源")
    review_notes: str = Field(default="", description="评审节点给出的备注")


# ---------------------------------------------------------------------------
# 图内流转内部模型（仅图内流转，不入响应协议）
# ---------------------------------------------------------------------------


class PhaseOutline(CamelModel):
    """大纲阶段的轻量描述（仅图内流转，不入响应协议）。"""

    name: str = Field(..., description="阶段名称")
    week_range: str = Field(default="", description="周区间，如 '第 1-3 周'")
    objectives: list[str] = Field(default_factory=list, description="阶段目标")
    chapter_titles: list[str] = Field(default_factory=list, description="章节标题列表")
    project_brief: str = Field(default="", description="阶段项目一句话简介")


class PlanOutline(CamelModel):
    """计划总大纲：先产出骨架，再逐阶段展开（仅图内流转，不入响应协议）。"""

    title: str = Field(..., description="计划标题")
    overview: str = Field(default="", description="整体概述")
    estimated_total_hours: int = Field(default=0, description="预计总学时")
    phases: list[PhaseOutline] = Field(default_factory=list, description="阶段大纲列表")
    resources: list[Resource] = Field(default_factory=list, description="推荐资源")


# ---------------------------------------------------------------------------
# 响应包络
# ---------------------------------------------------------------------------


class ErrorDetail(CamelModel):
    """错误详情。"""

    code: str = Field(default="internal", description="错误码")
    message: str = Field(..., description="错误信息")


class PlanResponse(CamelModel):
    """三端统一响应包络。

    status 取值：
    - "plan"：生成成功，plan 非空
    - "clarification"：输入信息不完整，clarifications 为澄清问题列表
    - "error"：失败，error 非空
    """

    status: Literal["plan", "clarification", "error"]
    plan: StudyPlan | None = None
    clarifications: list[str] | None = None
    error: ErrorDetail | None = None


class HealthResponse(CamelModel):
    """健康检查响应。"""

    status: str = "ok"
    service: str = "praxis-agent"
    version: str = "0.0.1"


class AgentSettingsModel(CamelModel):
    """Agent 运行配置（三端协议 AgentSettings）。

    设置页读写的载体：Rust 壳持久化到 settings.json，并经 POST /api/config
    推送给本服务热加载。API Key 原文传输（桌面单用户本机场景）。
    五家提供商各自独立的 Key 与模型名，切换互不覆盖。
    """

    llm_provider: Literal["openai", "anthropic", "deepseek", "glm", "opencode"] = (
        "openai"
    )
    # 通用 OpenAI
    openai_api_key: str = ""
    openai_model: str = "gpt-4o-mini"
    # Anthropic（Claude）
    anthropic_api_key: str = ""
    anthropic_model: str = "claude-3-5-sonnet-latest"
    # DeepSeek（官方 SDK）
    deepseek_api_key: str = ""
    deepseek_model: str = "deepseek-chat"
    # 智谱 GLM
    glm_api_key: str = ""
    glm_model: str = "glm-4-flash"
    # OpenCode Go 套餐（OpenAI 兼容端点）
    opencode_api_key: str = ""
    opencode_model: str = "grok-4.7"
    # 离线演示模式：不调用 LLM，返回确定性 mock 计划
    allow_mock_llm: bool = False


# ---------------------------------------------------------------------------
# LLM 结构化输出模型（作为 with_structured_output 的 schema）
# ---------------------------------------------------------------------------


class ClarificationOutput(BaseModel):
    """clarify 节点的 LLM 输出：生成的澄清问题列表。"""

    questions: list[str] = Field(..., description="向用户提出的澄清问题，最多 3 条")


class ReviewOutput(BaseModel):
    """review 节点的 LLM 输出：计划质量评审结论。"""

    passed: bool = Field(..., description="计划是否通过质量评审")
    issues: list[str] = Field(default_factory=list, description="发现的问题")
    suggestions: list[str] = Field(default_factory=list, description="改进建议")
    failed_phase_indexes: list[int] = Field(
        default_factory=list, description="评审未通过的阶段索引（0-based）"
    )
