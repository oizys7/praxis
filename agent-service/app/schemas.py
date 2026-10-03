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


# ---------------------------------------------------------------------------
# 响应中的计划结构
# ---------------------------------------------------------------------------


class Project(CamelModel):
    """阶段收尾项目及验收标准。"""

    name: str = Field(..., description="项目名称")
    description: str = Field(default="", description="项目描述")
    acceptance_criteria: list[str] = Field(
        default_factory=list, description="可验证的验收标准"
    )


class Phase(CamelModel):
    """学习计划的一个阶段：理论 / 实操 / 作业 / 项目 四位一体。"""

    name: str = Field(..., description="阶段名称")
    week_range: str = Field(default="", description="周区间，如 '第 1-3 周'")
    objectives: list[str] = Field(default_factory=list, description="阶段目标")
    theory: list[str] = Field(default_factory=list, description="理论学习内容")
    practice: list[str] = Field(default_factory=list, description="动手实操任务")
    assignments: list[str] = Field(default_factory=list, description="课后作业")
    project: Project = Field(
        default_factory=lambda: Project(name=""), description="阶段项目"
    )


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
    phases: list[Phase] = Field(default_factory=list, description="阶段列表")
    resources: list[Resource] = Field(default_factory=list, description="推荐资源")
    review_notes: str = Field(default="", description="评审节点给出的备注")


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
