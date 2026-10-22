"""Pydantic 数据模型：Vue / Rust / Python 三端统一的请求与响应结构。

约定：
- JSON 层字段一律使用 camelCase（通过 alias_generator 实现）；
- Python 代码内部仍使用 snake_case，读写均可（populate_by_name=True）。
- 对齐文件：src-tauri/src/commands.rs、frontend/src/types/agent.ts
"""

from __future__ import annotations

import json
from enum import Enum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator


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
    # 每周学习日（需求 5）：1=周一..7=周日，默认全周；仅作为日历分配的
    # request 元数据，不走 LLM 提示词
    study_days: list[int] = Field(
        default_factory=lambda: [1, 2, 3, 4, 5, 6, 7],
        description="每周学习日（1=周一..7=周日，默认全周）",
    )

    @field_validator("study_days", mode="before")
    @classmethod
    def _normalize_study_days(cls, v: Any) -> list[int]:
        """归一化：null 回退全周默认；去重升序；非空且取值 1..=7。"""
        if v is None:
            return [1, 2, 3, 4, 5, 6, 7]
        if not isinstance(v, list):
            # Pydantic validator 约定必须抛 ValueError（包装为 ValidationError），
            # TypeError 不会被 Pydantic 捕获——禁用 ruff TRY004 的 TypeError 建议
            raise ValueError("studyDays 必须是数组")  # noqa: TRY004
        if not v:
            raise ValueError("studyDays 不能为空（至少选择一个学习日）")
        for item in v:
            if not isinstance(item, int) or isinstance(item, bool):
                raise ValueError(  # noqa: TRY004
                    f"studyDays 每项须为整数，实际含 {item!r}"
                )
        days = sorted(set(v))
        for d in days:
            if not 1 <= d <= 7:
                raise ValueError(f"studyDays 取值须为 1（周一）..7（周日），实际含 {d}")
        return days


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


class GenerationSubmit(CamelModel):
    """后台生成任务提交体（POST /api/generations）。

    kind 与载荷二选一严格配对：plan ↔ planRequest；regen_phase / regen_chapter
    ↔ regenRequest。配对关系由 model_post_init 显式校验（扁平模型，非
    discriminated union），错配（如 plan 却带 regenRequest）在校验层直接拒绝（422）。
    """

    kind: str = Field(..., description="任务种类：plan | regen_phase | regen_chapter")
    plan_request: PlanRequest | None = Field(
        default=None, description="plan 任务的载荷"
    )
    regen_request: RegenRequest | None = Field(
        default=None, description="regen 任务的载荷"
    )

    def model_post_init(self, __context: Any, /) -> None:
        """显式校验 kind 与载荷的配对关系（本模型的核心防线，无 discriminated union）。"""
        allowed = {"plan", "regen_phase", "regen_chapter"}
        if self.kind not in allowed:
            raise ValueError(
                f"kind 必须是 {'|'.join(sorted(allowed))}，实际为: {self.kind!r}"
            )
        payload_present = (
            self.plan_request is not None or self.regen_request is not None
        )
        wants_plan = self.kind == "plan"
        wants_regen = self.kind != "plan"
        if not payload_present:
            raise ValueError(
                "缺少任务载荷：plan 任务必须带 planRequest，"
                "regen_phase / regen_chapter 任务必须带 regenRequest"
            )
        if wants_plan and self.plan_request is None:
            raise ValueError(
                "kind=plan 时必须携带 planRequest（不得携带 regenRequest）"
            )
        if wants_regen and self.regen_request is None:
            raise ValueError(
                f"kind={self.kind} 时必须携带 regenRequest（不得携带 planRequest）"
            )
        if wants_plan and self.regen_request is not None:
            raise ValueError("kind=plan 时不得携带 regenRequest")
        if wants_regen and self.plan_request is not None:
            raise ValueError(f"kind={self.kind} 时不得携带 planRequest")

    def payload(self) -> PlanRequest | RegenRequest:
        """取与 kind 配对的载荷对象（shape 在 model_post_init 已保证）。"""
        if self.kind == "plan":
            assert self.plan_request is not None
            return self.plan_request
        assert self.regen_request is not None
        return self.regen_request


# ---------------------------------------------------------------------------
# 响应中的计划结构
# ---------------------------------------------------------------------------


def _coerce_points_matrix(value: Any) -> Any:
    """逐项兜底「每章知识点」：字符串/对象/None 都归一为字符串数组。

    与长度对齐解耦：条目数少于章节数时，缺的章按「未指定」处理
    （nodes 侧的防御助手读不到就给空列表，覆盖率校验随之跳过），
    因此这里只做类型归一，不臆造内容。
    """
    if not isinstance(value, list):
        return value
    normalized: list[list[str]] = []
    for item in value:
        if isinstance(item, (list, tuple)):
            normalized.append([str(entry) for entry in item if entry is not None])
        elif isinstance(item, str):
            normalized.append([item])
        elif isinstance(item, dict):
            normalized.append(_coerce_str_list(list(item.values())))
        elif item is None:
            normalized.append([])
        else:
            normalized.append([str(item)])
    return normalized


def _coerce_str_list(value: Any) -> Any:
    """把「本该是字符串数组」的值归一成字符串列表（LLM 输出容错）。

    真实运行里 GLM 在 steps 这类「有序步骤」字段上会输出对象
    （{"step": 1, "action": "..."}），也出现过编号数字，导致
    ValidationError 整轮重试——一次 8 条错误、白烧 25-60 秒。
    内容本身是可用的，为格式瑕疵重烧调用不划算，这里做一次轻度归一：
    - 字符串原样保留；数字/布尔转字符串
    - dict 优先取常见文本字段，取不到则按「键：值」展平
    - 其余类型紧凑 JSON 序列化；None 直接丢弃

    非列表（含 None）原样返回，交给 Pydantic 正常报错——不掩盖真正
    缺失的字段（缺 project 本身仍应让调用方兜底或重试）。
    """
    if not isinstance(value, list):
        return value
    text_keys = ("action", "description", "content", "task", "text", "title", "detail")
    normalized: list[str] = []
    for item in value:
        if isinstance(item, str):
            text = item.strip()
        elif isinstance(item, (int, float, bool)):
            text = str(item)
        elif isinstance(item, dict):
            text = ""
            for key in text_keys:
                candidate = item.get(key)
                if isinstance(candidate, str) and candidate.strip():
                    text = candidate.strip()
                    break
            if not text:
                parts: list[str] = []
                for key, raw in item.items():
                    if raw is None or str(raw).strip() == "":
                        continue
                    rendered = (
                        raw if isinstance(raw, str) else json.dumps(raw, ensure_ascii=False)
                    )
                    parts.append(f"{key}：{rendered}")
                text = "；".join(parts)
        elif item is None:
            continue
        else:
            text = json.dumps(item, ensure_ascii=False)
        if text:
            normalized.append(text)
    return normalized


class PhaseProject(CamelModel):
    """阶段收尾项目：背景、步骤、产出与验收标准。"""

    # name 允许缺省：GLM 偶发只给 steps / acceptanceCriteria 不给 name。
    # 空名由 _expand_phase_extras 补「阶段 N 项目」兜底文案，比整轮重试划算。
    name: str = Field(default="", description="项目名称")
    description: str = Field(default="", description="项目描述")
    background: str = Field(default="", description="项目背景与动机")
    steps: list[str] = Field(default_factory=list, description="建议实施步骤（有序）")
    estimated_hours: int = Field(default=0, description="预计投入学时")
    acceptance_criteria: list[str] = Field(
        default_factory=list, description="可验证的验收标准"
    )

    @field_validator("steps", "acceptance_criteria", mode="before")
    @classmethod
    def _normalize_text_list(cls, value: Any) -> Any:
        """模型把「一句话一步」写成对象时归一为字符串，而不是判失败。"""
        return _coerce_str_list(value)


class Chapter(CamelModel):
    """阶段内的学习章节：完整 markdown 正文（图示内联 ```mermaid 码块）。

    kind 决定正文的形态要求（概念章讲原理、实操章重步骤、项目章要验收、
    复习章做回顾），由大纲给定、生成时原样遵循，校验按类型分叉。
    key_terms 是本章核心术语（2-4 个），供 assemble 汇总为全计划术语表。
    """

    title: str = Field(..., description="章节标题")
    summary: str = Field(default="", description="章节概述")
    content_md: str = Field(default="", description="章节 markdown 正文全文")
    kind: str = Field(
        default="concept", description="章型：concept / practice / project / review"
    )
    key_terms: list[str] = Field(
        default_factory=list, description="本章核心术语（2-4 个）"
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
    """推荐学习资源。

    url 为空串表示模型没有给出确切地址（禁止编造路径）；
    verification 由 resource_check 的真实探测得出，两档：reachable / unverified。
    """

    title: str = Field(..., description="资源名称")
    type: str = Field(default="docs", description="类型：book/course/docs/video/other")
    note: str = Field(default="", description="推荐理由")
    url: str = Field(default="", description="资源真实地址；空串表示未给出")
    verification: str = Field(
        default="unverified", description="可达性：reachable / unverified"
    )


class GlossaryEntry(CamelModel):
    """术语表条目（只读）：assemble 由各章 keyTerms 去重汇总，不单独存储、不可编辑。"""

    term: str = Field(..., description="术语")
    chapter_title: str = Field(default="", description="首次出现的章节标题")


class StudyPlan(CamelModel):
    """完整学习计划。"""

    title: str = Field(..., description="计划标题")
    overview: str = Field(default="", description="整体概述")
    estimated_total_hours: int = Field(default=0, description="预计总学时")
    phases: list[StudyPhase] = Field(default_factory=list, description="阶段列表")
    resources: list[Resource] = Field(default_factory=list, description="推荐资源")
    review_notes: str = Field(default="", description="评审节点给出的备注")
    glossary: list[GlossaryEntry] = Field(
        default_factory=list,
        description="全计划术语表（只读，由各章 keyTerms 去重汇总）",
    )


# ---------------------------------------------------------------------------
# 图内流转内部模型（仅图内流转，不入响应协议）
# ---------------------------------------------------------------------------


class PhaseOutline(CamelModel):
    """大纲阶段的轻量描述（仅图内流转，不入响应协议）。"""

    name: str = Field(..., description="阶段名称")
    week_range: str = Field(default="", description="周区间，如 '第 1-3 周'")
    objectives: list[str] = Field(default_factory=list, description="阶段目标")
    chapter_titles: list[str] = Field(default_factory=list, description="章节标题列表")
    # 与 chapter_titles 等长的章型列表（并行数组：保持 outline 事件载荷不变，
    # 避免破坏已上线的骨架渲染契约）。越界/缺失时读取方按 concept 兜底。
    chapter_knowledge_points: list[list[str]] = Field(
        default_factory=list,
        description="与 chapterTitles 等长同序的知识点列表（每章 3-6 个）",
    )

    @field_validator("chapter_knowledge_points", mode="before")
    @classmethod
    def _normalize_points_matrix(cls, value: Any) -> Any:
        """把「每章知识点」归一成 list[list[str]]，避免嵌套数组把整份大纲判废。

        实测（GLM）嵌套两层的数组极不稳定：会把 chapterKnowledgePoints 直接
        输出成扁平 list[str]（一章一个长字符串），于是 8 阶段 × 5 章一次报出
        40 条 "Input should be a valid list"，整份大纲连同这次生成一起报废。
        一章一个字符串仍然是有效信息，包成单元素列表即可。
        """
        return _coerce_points_matrix(value)
    chapter_kinds: list[str] = Field(
        default_factory=list, description="与 chapterTitles 等长的章型列表"
    )
    project_brief: str = Field(default="", description="阶段项目一句话简介")
    # 版本事实（计划级，outline 阶段探测后下发到每个阶段）。重生成必须能读回它，
    # 否则正文里只能写「版本待确认」。用裸 dict 承载：它只在图内与 plans.outline_json
    # 之间流转，不参与响应协议。
    tech_facts: list[dict[str, Any]] = Field(
        default_factory=list,
        description="技术栈当前版本事实（name / ecosystem / latestVersion）",
    )


class PhaseExtrasOutput(BaseModel):
    """expand_phase_wrap 轻量调用的 LLM 输出（仅图内流转，不入响应协议）。

    章级扇出拓扑下阶段收尾内容（practice / assignments / project）由每阶段
    一次轻量调用单独产出，章节正文不在此模型内。
    """

    practice: list[str] = Field(default_factory=list, description="动手实操任务")
    assignments: list[str] = Field(default_factory=list, description="课后作业")
    project: PhaseProject = Field(..., description="阶段项目")

    @field_validator("practice", "assignments", mode="before")
    @classmethod
    def _normalize_text_list(cls, value: Any) -> Any:
        """与 PhaseProject.steps 同源容错：对象形式的条目归一为字符串。"""
        return _coerce_str_list(value)


class TechRef(CamelModel):
    """技术栈条目：版本探测的输入（计划级，与学习目标直接相关的库/工具）。

    条目查不到会被丢弃——这层同时充当幻觉过滤器：模型编造的库名没有版本，
    不会作为事实传给下游。
    """

    name: str = Field(
        ..., description="库/工具名（crate / npm / PyPI 名称，或 GitHub 的 owner/repo）"
    )
    ecosystem: str = Field(
        default="crate", description="生态：crate | npm | pypi | github"
    )


class PlanOutline(CamelModel):
    """计划总大纲：先产出骨架，再逐阶段展开（仅图内流转，不入响应协议）。"""

    title: str = Field(..., description="计划标题")
    overview: str = Field(default="", description="整体概述")
    estimated_total_hours: int = Field(default=0, description="预计总学时")
    phases: list[PhaseOutline] = Field(default_factory=list, description="阶段大纲列表")
    resources: list[Resource] = Field(default_factory=list, description="推荐资源")
    tech_stack: list[TechRef] = Field(
        default_factory=list, description="本计划涉及的核心技术栈（用于探测当前版本）"
    )


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
    version: str = "0.1.1"


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


class ReviewDimension(BaseModel):
    """评审维度打分：让「为什么没过」可解释，也便于定位低分环节。"""

    name: str = Field(
        ...,
        description="维度名称，如 结构完整 / 正文质量 / 可测性 / 时间可行 / 难度递进",
    )
    score: int = Field(..., ge=1, le=5, description="1-5 分，5 表示该维度无明显问题")
    comment: str = Field(default="", description="给出该分数的具体依据")


class ReviewOutput(BaseModel):
    """review 节点的 LLM 输出：计划质量评审结论。"""

    passed: bool = Field(..., description="计划是否通过质量评审")
    issues: list[str] = Field(default_factory=list, description="发现的问题")
    suggestions: list[str] = Field(default_factory=list, description="改进建议")
    dimensions: list[ReviewDimension] = Field(
        default_factory=list,
        description="分维度打分；低分维度（≤3）会被汇总进建议，便于定位问题环节",
    )
    failed_phase_indexes: list[int] = Field(
        default_factory=list, description="评审未通过的阶段索引（0-based）"
    )
