"""离线演示模式（config.json agent.mockMode / 设置页 mock 热更开关）使用的确定性计划生成器。

用途：没有 API Key 时也能跑通“前端 -> Tauri -> FastAPI -> LangGraph”全链路，
便于开发调试 UI 与通信协议。生产使用请关闭该开关（config.json 不设
mockMode，并在设置页关闭离线演示开关）。

两级生成（与 outline_node / expand_chapter_node / expand_phase_wrap_node 节点对应）：
- build_mock_outline：先产出 PlanOutline 骨架（阶段大纲，不含章节正文）；
- build_mock_chapter：按章节标题出单章 mock 数据（Chapter 形状，供章级扇出主链）；
- build_mock_extras：按阶段出轻装（practice / assignments / project，供 wrap 节点）；
- build_mock_phase：整阶段组装（章节 + 轻装，仅供 regen_phase 端点复用）。

约定：camelCase dict 进出（与 schemas 模型 model_dump(by_alias=True) 形状
一致），但本模块不 import schemas、纯字典构造；每章产出单个 contentMd
markdown 正文（与 validators 的章节规则对齐随任务 19 重写后恢复「零 pending」
契约，本模块不再保留任何结构化形状）。
"""

from __future__ import annotations

from typing import Any

# 每阶段章节数（与 PhaseOutline.chapterTitles / StudyPhase.chapters 对齐）
CHAPTERS_PER_PHASE = 2
# 每章 markdown 正文目标字数（「深入剖析」节单独托底，整体正文恒达标）
CONTENT_TARGET_CHARS = 1000


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

# 深入剖析节确定性填充句：同输入必得同输出（无随机、无时间依赖）
_DEEP_DIVE_FILLER = (
    "按同样的三段式（场景—动手—误区）推进每个小节："
    "先描述一个具体使用场景，再做一次最小实验验证，"
    "最后列出一条典型误区与纠正方法。"
    "每轮结束后用自己的话复述要点，再对照原文查漏补缺，"
    "把短期能看懂的内容转化为长期可调用的经验。"
)
# 知识点主线句确定性填充句（与深入剖析填充句同源、无随机依赖）
_POINT_FILLER = (
    "为这个知识点设计一个可复述的口念总结，"
    "并在下一次回顾时先复述再翻书校正；"
    "能在不看资料的情况下讲清楚，才算真正掌握。"
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
    """确定性长文：固定模板段落 + 填充句循环展开到 ≥1000 字（strip 后仍达标）。

    供「深入剖析」节单独托底：正文总字数与该节字数都恒达标，
    极限输入（极短标题 / goal）下也一样，不依赖输入长度。"""
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
    while len(content) < CONTENT_TARGET_CHARS:
        content += "\n\n" + _DEEP_DIVE_FILLER
    return content


def _knowledge_section(title: str, goal: str) -> str:
    """章节主线：每个知识点的段落式讲解（确定性，同输入同输出）。"""
    points = (
        (
            f"它是什么——结合「{goal}」的具体场景给出定义与反例。"
            "先用一句话说清它的边界，再举一个贴近真实项目场景的例子；"
            "读完后尝试不看原文复述这句定义。"
        ),
        (
            "适用场景——什么样的问题值得用它，什么样的场景不用。"
            "列出至少一个典型的适用情形和一个明显不适用的情况，"
            "两者对比着记录在笔记里。"
        ),
        (
            "常见误区——初学者最容易在哪里踩坑、错误的直觉从何而来。"
            "写下一个常见的错误认识，再给出纠错的对照实验；"
            "把错误直觉的来源也一并记下来。"
        ),
        (
            "动手验证——写一个最小可运行示例，逐步加入变化观察输出差异。"
            f"示例连同结果一起记进笔记，回到「{goal}」的目标场景再作一次自查。"
        ),
    )
    body = "\n\n".join(f"### {title}·{i + 1}\n\n{p}" for i, p in enumerate(points))
    while len(body) < CONTENT_TARGET_CHARS:
        body += "\n\n" + _POINT_FILLER
    return body


# mock 章节配图的确定性 mermaid 源码（flowchart TD，供前端渲染的码块）
_CHAPTER_MERMAID = (
    "```mermaid\n"
    "flowchart TD\n"
    "    A[输入: 概念与场景] --> B{能独立复述吗}\n"
    "    B -- 否 --> C[回看概念定义]\n"
    "    C --> B\n"
    "    B -- 是 --> D[完成配套动手练习]\n"
    "    D --> E[记入个人笔记]\n"
    "```"
)


def _chapter(
    title: str,
    goal: str,
    *,
    kind: str = "concept",
    key_terms: list[str] | None = None,
    knowledge_points: list[str] | None = None,
) -> dict[str, Any]:
    """按章节标题构造 StudyPhase.chapters 元素（Chapter 形状，camelCase）。

    形状：{title, summary, contentMd, kind, keyTerms}——contentMd 为确定性模板 markdown
    （标题 + 知识点段落 + 图示码块 + 深入剖析），同输入必得同输出，
    无任何结构化字段残留。"""
    content_md = "\n\n".join(
        (
            f"## {title}\n\n本章围绕「{title}」展开，结合「{goal}」的目标场景梳理主线内容。",
            # 固定小节覆盖 validators.REQUIRED_SECTIONS_BY_KIND 的并集：
            # mock 产物同样要过真实校验，否则离线全链路会被自己判失败
            (
                "### 学习目标\n\n读完本章，你能说清它解决什么问题、在整体目标中"
                "处于什么位置，并独立完成一个最小可运行示例。"
            ),
            (
                "### 前置知识\n\n需要先掌握上一章的术语与基本操作；本章出现的"
                "新概念一律就地解释，不假设你已经会。"
            ),
            # 知识点小节：标题与大纲 chapterKnowledgePoints 一字不差，
            # 覆盖率校验必然命中（mock 产物同样要过真实校验）
            *[
                f"## {point}\n\n{_knowledge_section(point, goal)}"
                for point in (knowledge_points or [])
            ],
            "### 什么是它 / 为什么需要它\n\n" + _knowledge_section(title, goal),
            _CHAPTER_MERMAID,
            # 用 ### 级（与后续「### 动手练习」同级）：validators 的深入小节
            # 按标题级别截断，## 级会把动手练习与自检的字数也算进去
            f"### 深入：机制剖析\n\n{_deep_dive_content(title, goal)}",
            (
                f"### 动手练习\n\n围绕「{title}」写一个最小示例：先跑通，再改参数"
                "观察行为变化，最后用自己的话记录结论。"
            ),
            (
                "### 自检清单\n\n能不看资料复述主线；能逐行解释图示中的每一步；"
                "能指出一个常见误用并说明它的后果。"
            ),
        )
    )
    return {
        "title": title,
        "summary": f"本章围绕「{title}」展开，结合「{goal}」的目标场景梳理主线内容。",
        "contentMd": content_md,
        "kind": kind,
        "keyTerms": list(key_terms or []),
    }


# ---------------------------------------------------------------------------
# 两级生成接口
# ---------------------------------------------------------------------------


# 章型轮转：mock 大纲与 mock 章节共用同一规则，保证骨架章型与正文章型一致。
# 真实链路里章型由 outline.yaml 决定，这里只是离线演示的确定性替身。
_KIND_CYCLE = ("concept", "concept", "practice", "project")


def _mock_knowledge_points(title: str) -> list[str]:
    """mock 章节的知识点：与正文 `## ` 小节标题一字不差，保证覆盖率校验命中。"""
    return [
        f"{title}的定位与作用",
        f"{title}的核心机制",
        f"{title}的常见误区",
    ]


def _kind_for(index: int) -> str:
    """按章索引确定 mock 章型（越界安全，负数也能落到合法值）。"""
    return _KIND_CYCLE[index % len(_KIND_CYCLE)]


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
            "chapterKinds": [
                _kind_for(i) for i in range(len(stage["chapterTitles"]))
            ],
            "chapterKnowledgePoints": [
                _mock_knowledge_points(title) for title in stage["chapterTitles"]
            ],
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
        # mock 资源给真实可点的通用地址（离线演示也可验证链接渲染），
        # verification 保持 unverified：mock 分支刻意不联网，不谎报「已核实」
        "resources": [
            {
                "title": "官方入门文档",
                "type": "docs",
                "note": "最权威的第一手资料",
                "url": "https://developer.mozilla.org/",
            },
            {
                "title": "经典入门书籍",
                "type": "book",
                "note": "系统建立知识框架",
                "url": "https://openlibrary.org/",
            },
            {
                "title": "视频实战课",
                "type": "video",
                "note": "跟着做一遍，降低上手门槛",
                "url": "https://www.bilibili.com/",
            },
        ],
    }


def build_mock_chapter(
    request: dict[str, Any], phase_outline: dict[str, Any], chapter_index: int
) -> dict[str, Any]:
    """按阶段大纲摘出单章标题，生成单个章节（Chapter 形状，camelCase dict）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果，
    phase_outline 是 build_mock_outline 输出 phases 中的元素。
    章级扇出主链与单章重生成路径共用本构造器，保证两条路径产出的
    同一章节目内容一致。
    """
    goal: str = request.get("goal", "你的学习目标")
    chapter_titles: list[str] = phase_outline.get("chapterTitles") or []
    title = chapter_titles[chapter_index] if chapter_titles else "未命名章节"
    kinds: list[str] = phase_outline.get("chapterKinds") or []
    kind = kinds[chapter_index] if chapter_index < len(kinds) else _kind_for(chapter_index)
    points_map = phase_outline.get("chapterKnowledgePoints") or []
    points = points_map[chapter_index] if chapter_index < len(points_map) else []
    return _chapter(
        title,
        goal,
        kind=kind,
        key_terms=[title],
        knowledge_points=[str(p) for p in (points or [])],
    )


def build_mock_extras(
    request: dict[str, Any], phase_outline: dict[str, Any], index: int
) -> dict[str, Any]:
    """按阶段大纲生成阶段轻装：practice / assignments / project（camelCase dict）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果，
    phase_outline 是 build_mock_outline 输出 phases 中的元素。
    index ≥ 项目模板数时循环取模板（防御，正常调用 0-2）。不产出章节正文。
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
        "practice": [
            f"跟随章节完成「{title}」的配套动手练习" for title in chapter_titles
        ],
        "assignments": [
            "整理本章概念卡片：每个概念一句话定义加一个自选示例",
            "独立完成一个迷你练习并写 200 字左右的实现说明",
        ],
        "project": project,
    }


def build_mock_phase(
    request: dict[str, Any], phase_outline: dict[str, Any], index: int
) -> dict[str, Any]:
    """按阶段大纲组装完整阶段（StudyPhase 形状，camelCase dict）。

    章节由 build_mock_chapter 逐章生成、轻装由 build_mock_extras 生成，
    本函数只做拼装（仅供 regen_phase 端点全阶段重生成复用）。
    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果，
    phase_outline 是 build_mock_outline 输出 phases 中的元素。
    """
    chapter_titles: list[str] = phase_outline.get("chapterTitles") or []

    return {
        "name": phase_outline.get("name", ""),
        "weekRange": phase_outline.get("weekRange", ""),
        "objectives": list(phase_outline.get("objectives") or []),
        "chapters": [
            build_mock_chapter(request, phase_outline, chapter_index)
            for chapter_index in range(len(chapter_titles))
        ],
        **build_mock_extras(request, phase_outline, index),
    }
