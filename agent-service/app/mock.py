"""离线演示模式（PRAXIS_ALLOW_MOCK_LLM=true）使用的确定性计划生成器。

用途：没有 API Key 时也能跑通"前端 -> Tauri -> FastAPI -> LangGraph"全链路，
便于开发调试 UI 与通信协议。生产使用请关闭该开关。

两级生成（与 outline_node / expand_phase_node 节点一一对应）：
- build_mock_outline：先产出 PlanOutline 骨架（阶段大纲，不含章节正文）；
- build_mock_phase：再按阶段大纲逐个扩展为 StudyPhase（章节 + 深入内容 + 项目）；
- build_mock_chapter：仅重生成单个章节（Chapter 形状），供 regen 端点单章路径。

约定：camelCase dict 进出（与 schemas 模型 model_dump(by_alias=True) 形状
一致），但本模块不 import schemas、纯字典构造；输出必须能通过
validators.validate_plan 零 pending（校验阈值见 validators.py 顶部常量）。
"""

from __future__ import annotations

from typing import Any

# 每阶段章节数（与 PhaseOutline.chapterTitles / StudyPhase.chapters 对齐）
CHAPTERS_PER_PHASE = 2
# 每章核心概念 / 知识点条数（校验器只要求非空，mock 取充足余量）
CONCEPTS_PER_CHAPTER = 3
KNOWLEDGE_POINTS_PER_CHAPTER = 4
# 每篇深入内容正文目标字数（校验器下限 500，mock 留余量）
DEEP_DIVE_TARGET_CHARS = 600


def _to_int(value: Any, default: int) -> int:
    """安全转 int：非法输入回退默认值（mock 输入本已经 Pydantic 校验，此处纯防御）。"""
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


# ---------------------------------------------------------------------------
# 阶段大纲模板：难度递进（基础 -> 进阶 -> 综合项目）
# ---------------------------------------------------------------------------

_STAGES: tuple[dict[str, Any], ...] = (
    {
        "name": "第一阶段 · 打基础",
        "objectives": [
            "建立「{goal}」所需的核心概念框架",
            "搭好本地环境，跑通第一个可运行的练习",
        ],
        "chapterTitles": ("核心概念入门", "动手初体验"),
        "projectBrief": "完成一个入门级小作品，把基础概念落地成可运行的东西",
    },
    {
        "name": "第二阶段 · 核心进阶",
        "objectives": [
            "吃透「{goal}」的核心机制与常用模式",
            "能独立排查并修复练习中的典型问题",
        ],
        "chapterTitles": ("核心机制剖析", "进阶模式与技巧"),
        "projectBrief": "完成一个有中等复杂度的综合练习，串联前段所学",
    },
    {
        "name": "第三阶段 · 综合实战",
        "objectives": [
            "综合运用全部知识完成一个完整项目",
            "沉淀可复用的笔记与知识体系，能向他人讲清楚",
        ],
        "chapterTitles": ("综合项目实战", "复盘与知识体系化"),
        "projectBrief": "完成毕业项目并做系统复盘，形成完整的成果展示",
    },
)

# ---------------------------------------------------------------------------
# 阶段项目模板：按 index 取用，index ≥ len 时循环（防御）
# 每套均含 name / description / background / steps(≥3) /
# acceptanceCriteria(≥2，可验证表述，禁「了解/熟悉」) / estimatedHours(>0)
# ---------------------------------------------------------------------------

_PROJECT_TEMPLATES: tuple[dict[str, Any], ...] = (
    {
        "name": "入门实战：最小可用作品",
        "description": "围绕学习目标做出一个能跑起来的最小作品，验证环境与基础概念。",
        "background": (
            "刚入门时最大的风险是只看不练：本阶段用一个小而完整的作品，"
            "强制把概念转化为可运行、可展示的产出。"
        ),
        "steps": [
            "按官方文档完成环境搭建，并跑通一个官方示例",
            "实现作品的核心功能闭环（输入—处理—输出）",
            "撰写 README，记录运行步骤与踩坑笔记",
        ],
        "estimatedHours": 8,
        "acceptanceCriteria": [
            "作品在本机可运行，README 含他人可照做的复现步骤",
            "能现场演示核心功能，并逐个说出用到的概念",
        ],
    },
    {
        "name": "综合练习：多知识点串联",
        "description": "把前两个阶段的知识点组合起来，解决一个带约束的完整小问题。",
        "background": (
            "孤立的知识点容易遗忘：用一个需要多知识点协作的练习，"
            "检验组合运用与查文档自救的能力。"
        ),
        "steps": [
            "把需求拆成 3 个以上可独立验证的子任务",
            "逐个实现子任务，每个完成后立即自测",
            "集成联调，修复边界情况并记录取舍理由",
        ],
        "estimatedHours": 12,
        "acceptanceCriteria": [
            "全部子任务的输出与预期一致，附自测记录",
            "针对至少 2 个边界输入给出正确处理并能解释原因",
        ],
    },
    {
        "name": "毕业项目：完整成果展示",
        "description": "独立完成一个覆盖全程所学的完整项目，并做公开式复盘。",
        "background": (
            "学习闭环的最后一环是输出：用一个完整项目证明能独立交付，"
            "同时暴露知识盲区，为后续进阶定位方向。"
        ),
        "steps": [
            "写一页项目方案：目标、范围、里程碑与技术选型",
            "按里程碑迭代开发，每个里程碑结束做一次演示",
            "完成复盘文档：做对了什么、踩了什么坑、下一步学什么",
        ],
        "estimatedHours": 16,
        "acceptanceCriteria": [
            "项目可部署或可运行，包含方案与复盘两份文档",
            "完成一次 15 分钟以内的完整讲解，回答疑问不留死角",
        ],
    },
)

# 深入内容确定性填充句：同输入必得同输出（无随机、无时间依赖）
_DEEP_DIVE_FILLER = (
    "按同样的三段式（场景—动手—误区）推进每个小节："
    "先描述一个具体使用场景，再做一次最小实验验证，"
    "最后列出一条典型误区与纠正方法。"
    "每轮结束后用自己的话复述要点，再对照原文查漏补缺，"
    "把短期能看懂的内容转化为长期可调用的经验。"
)


def _week_ranges(weeks: int) -> list[str]:
    """三阶段周区间：seg 均分、区间连续覆盖全部周数（weeks 过小时钳制到合法区间）。"""
    seg = max(1, weeks // 3)
    bounds = [(1, seg), (seg + 1, seg * 2), (seg * 2 + 1, weeks)]
    return [
        f"第 {min(start, weeks)}-{max(min(start, weeks), min(end, weeks))} 周"
        for start, end in bounds
    ]


def _deep_dive_content(topic: str, goal: str) -> str:
    """确定性长文：固定模板段落 + 填充句循环展开到 ≥600 字（strip 后仍达标）。"""
    paragraphs = (
        f"「{topic}」是「{goal}」学习路径中承上启下的一环。",
        (
            "先把概念放到具体场景里：它解决什么问题、不解决什么问题、"
            "与相邻概念的边界在哪里，尝试各用一句话说清楚；"
            "说不清楚的部分就是下一步要重点攻克的对象。"
        ),
        (
            "再动手验证：为它写一个最小可运行示例，然后逐步加入变化并观察输出的差异，"
            "把「看懂了」升级为「做出来了」，示例连同结果一起记进笔记。"
        ),
        (
            "最后回到常见误区：初学者最容易在哪些细节上踩坑、"
            "错误的直觉通常从哪里来、如何设计一次刻意的对照实验来纠正它。"
        ),
    )
    content = "\n\n".join(paragraphs)
    while len(content) < DEEP_DIVE_TARGET_CHARS:
        content += "\n\n" + _DEEP_DIVE_FILLER
    return content


def _chapter(title: str, goal: str) -> dict[str, Any]:
    """按章节标题构造 StudyPhase.chapters 元素（StudyChapter 形状，camelCase）。"""
    return {
        "title": title,
        "summary": f"本章围绕「{title}」展开，结合「{goal}」的目标场景梳理主线内容。",
        "mainKnowledge": {
            "concepts": [
                {
                    "name": f"{title}·{role}",
                    "definition": f"用一句话定义「{title}」中{role}的内涵，"
                    f"并说明它在「{goal}」里的位置。",
                }
                for role in ("它是什么", "为什么需要它", "怎么使用它")
            ],
            "knowledgePoints": [
                {
                    "title": f"{title}要点{i + 1}：{label}",
                    "detail": f"{label}——结合「{goal}」的具体场景给出示例与反例。",
                }
                for i, label in enumerate(
                    ("核心定义", "适用场景", "常见误区", "动手验证")
                )
            ],
            "diagrams": [
                {
                    "title": f"{title} 学习路径图",
                    "mermaid": (
                        "flowchart TD\n"
                        "    A[输入: 概念与场景] --> B{能独立复述吗}\n"
                        "    B -- 否 --> C[回看概念定义]\n"
                        "    C --> B\n"
                        "    B -- 是 --> D[完成配套动手练习]\n"
                        "    D --> E[记入个人笔记]"
                    ),
                    "caption": f"「{title}」从理解到动手的循环路径",
                }
            ],
        },
        "deepDives": [
            {
                "topic": f"{title}的三层理解",
                "angle": "从原理到实操，再回到常见误区",
                "content": _deep_dive_content(title, goal),
            }
        ],
    }


# ---------------------------------------------------------------------------
# 两级生成接口
# ---------------------------------------------------------------------------


def build_mock_outline(request: dict[str, Any]) -> dict[str, Any]:
    """根据请求生成确定性的 mock 大纲（PlanOutline 形状，camelCase dict）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果。
    仅产出骨架：阶段名称 / 周区间 / 目标 / 章节标题 / 项目一句话简介。
    """
    goal: str = request.get("goal", "你的学习目标")
    weeks: int = _to_int(request.get("weeks"), 12)
    hours: int = _to_int(request.get("weeklyHours"), 10)
    interests: list[str] = request.get("interests") or []
    background: str = request.get("background") or ""
    focus = f"（偏好：{'、'.join(interests)}）" if interests else ""

    ranges = _week_ranges(weeks)
    phases = [
        {
            "name": stage["name"],
            "weekRange": ranges[index],
            "objectives": [obj.format(goal=goal) for obj in stage["objectives"]],
            "chapterTitles": list(stage["chapterTitles"]),
            "projectBrief": stage["projectBrief"],
        }
        for index, stage in enumerate(_STAGES)
    ]

    return {
        "title": f"「{goal}」学习计划（Mock 演示版）",
        "overview": (
            f"这是一份由离线 Mock 模式生成的演示计划（{weeks} 周 × 每周 {hours} 小时，"
            f"总学时约 {weeks * hours} 小时）{focus}。"
            + (f"已考虑你的基础：{background[:40]}。" if background else "")
            + " 配置真实 API Key 后重新生成，可获得由 LLM 定制的计划。"
        ),
        "estimatedTotalHours": weeks * hours,
        "phases": phases,
        "resources": [
            {"title": "官方入门文档", "type": "docs", "note": "最权威的第一手资料"},
            {"title": "经典入门书籍", "type": "book", "note": "系统建立知识框架"},
            {
                "title": "视频实战课",
                "type": "video",
                "note": "跟着做一遍，降低上手门槛",
            },
        ],
    }


def build_mock_chapter(
    request: dict[str, Any], phase_outline: dict[str, Any], chapter_index: int
) -> dict[str, Any]:
    """按阶段大纲摘出单章标题，生成单个章节（Chapter 形状，camelCase dict）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果，
    phase_outline 是 build_mock_outline 输出 phases 中的元素。
    章节正交给 build_mock_phase 复用的同一 _chapter 构造器，保证
    全阶段重生成与单章重生成产出的章节目内容一致。
    """
    goal: str = request.get("goal", "你的学习目标")
    chapter_titles: list[str] = phase_outline.get("chapterTitles") or []
    title = chapter_titles[chapter_index] if chapter_titles else "未命名章节"
    return _chapter(title, goal)


def build_mock_phase(
    request: dict[str, Any], phase_outline: dict[str, Any], index: int
) -> dict[str, Any]:
    """按阶段大纲扩展出完整阶段（StudyPhase 形状，camelCase dict）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果，
    phase_outline 是 build_mock_outline 输出 phases 中的元素。
    index ≥ 项目模板数时循环取模板（防御，正常调用 0-2）。
    """
    goal: str = request.get("goal", "你的学习目标")
    chapter_titles: list[str] = phase_outline.get("chapterTitles") or []

    template = _PROJECT_TEMPLATES[index % len(_PROJECT_TEMPLATES)]
    project = {
        "name": template["name"],
        "description": f"{template['description']}目标方向：「{goal}」。",
        "background": template["background"],
        "steps": list(template["steps"]),
        "estimatedHours": template["estimatedHours"],
        "acceptanceCriteria": list(template["acceptanceCriteria"]),
    }

    return {
        "name": phase_outline.get("name", ""),
        "weekRange": phase_outline.get("weekRange", ""),
        "objectives": list(phase_outline.get("objectives") or []),
        "chapters": [_chapter(title, goal) for title in chapter_titles],
        "practice": [
            f"跟随章节完成「{title}」的配套动手练习" for title in chapter_titles
        ],
        "assignments": [
            "整理本章概念卡片：每个概念一句话定义加一个自选示例",
            "独立完成一个迷你练习并写 200 字左右的实现说明",
        ],
        "project": project,
    }
