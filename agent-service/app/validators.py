"""计划程序化校验器：assemble 拼装后、LLM 评审前的结构把关。

纯标准库实现：输入 camelCase dict（图内 StudyPlan 以 dict 形态流转，键如
``phases`` / ``chapters`` / ``contentMd`` / ``estimatedTotalHours``），
不 import schemas、不 import LangChain。

裁决约定：
- 缺失键按「不满足该规则」处理（全程 ``.get()`` 防御，不抛 KeyError）；
- 学时偏差（规则 8）只写 issues、不产生阶段索引——回炉重生任何一个阶段
  都修不了总学时；其余规则失败映射到对应阶段索引（规则 1 全局失败映射
  所有阶段索引）。
"""

from __future__ import annotations

import re

# ---------------------------------------------------------------------------
# 校验阈值（模块顶部常量）
# ---------------------------------------------------------------------------

# 阶段数范围（提示词已不设硬性数量：LLM 按难度/时长弹性决定，本值仅作
# 防跑飞安全上限）
MIN_PHASES = 1
MAX_PHASES = 16
# 每阶段章节数范围（同上：弹性生成，仅设防跑飞安全上限）
MIN_CHAPTERS_PER_PHASE = 1
MAX_CHAPTERS_PER_PHASE = 12
# 每章 markdown 正文最少字数（strip 后；contentMd 空直接 failed 可重生成）
MIN_CHAPTER_CONTENT_CHARS = 1000

# 章节固定小节：既是写给 LLM 的质量脚手架，也是可程序化校验的契约。
# 用子串而非精确标题匹配——「## 本章学习目标」「### 自检清单」都算命中，
# 避免措辞自然变体被误判为缺失而触发无谓回炉。
# 按章型分叉：概念章讲原理、实操与项目章重动手与验收、复习章做回顾。
# 与 prompts/chapter.yaml 的固定小节段、nodes._CHAPTER_KIND_REQUIREMENTS
# 三处必须成对变更——只罚不教会让模型反复因同一原因失败。
REQUIRED_SECTIONS_BY_KIND: dict[str, tuple[str, ...]] = {
    "concept": ("学习目标", "前置知识", "自检"),
    "practice": ("学习目标", "前置知识", "动手练习", "自检"),
    "project": ("学习目标", "动手练习", "验收", "自检"),
    "review": ("学习目标", "自检"),
}
DEFAULT_CHAPTER_KIND = "concept"
# 所有章型必需小节的并集（供文档与 mock 模板注释引用，不参与逐章判定）
REQUIRED_CHAPTER_SECTIONS = tuple(
    dict.fromkeys(
        section for sections in REQUIRED_SECTIONS_BY_KIND.values() for section in sections
    )
)
# 概念章必须有的深入剖析小节：关键词 + 最小字数。
# 这是唯一被强制的「深度」要求——其他章型允许没有深入小节。
DEEP_DIVE_KEYWORD = "深入"
DEEP_DIVE_MIN_CHARS = 600
# 阶段项目最少步骤 / 验收标准条数
MIN_PROJECT_STEPS = 2
MIN_PROJECT_ACCEPTANCE = 2
# 总学时相对偏差上限（|实际-期望|/期望）
MAX_HOUR_DRIFT = 0.2

#: 知识点在小节标题里的最低命中率。
#: 用比例而不是「一个都不能少」：知识点措辞与标题天然有差异，个别对不上
#: 不该触发回炉；但半数以上对不上，说明模型根本没照大纲组织正文。
COVERAGE_MIN_RATIO = 0.6

#: 匹配前先剔除的动作词/虚词，避免「理解」「掌握」这类词主导命中判定
_COVERAGE_STOPWORDS = (
    "理解",
    "掌握",
    "熟悉",
    "了解",
    "认识",
    "学会",
    "能够",
    "知道",
    "区分",
    "以及",
    "如何",
    "什么",
    "为什么",
    "并且",
    "与",
    "和",
    "的",
    "了",
    "是",
)

#: 固定小节与深度小节本来就不承载知识点，反向检查时跳过
_COVERAGE_SKIP_TITLES = (*REQUIRED_CHAPTER_SECTIONS, DEEP_DIVE_KEYWORD, "验收")
# Mermaid 图类型白名单（源码首词，大小写敏感）
MERMAID_DIAGRAM_TYPES = {
    "graph",
    "flowchart",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "erDiagram",
    "gantt",
    "mindmap",
}


def _safe_int(text: str) -> int:
    """键片段转整数：格式异常时退回 0（键由本模块生成，这里只做防御）。"""
    try:
        return int(text)
    except (TypeError, ValueError):
        return 0


def _chapter_sort_key(key: str) -> tuple[int, int]:
    """把 "阶段:章" 复合键还原成整数元组（稳定排序用）。"""
    phase, _, chapter = key.partition(":")
    return (_safe_int(phase), _safe_int(chapter))


def _mark_chapter(
    failed: set[int], failed_chapters: set[str], index: int, chapter_index: int
) -> None:
    """登记一批只与单章有关的问题：阶段需回炉，且精确到章。

    仅有阶段索引时，expand_revision 只能整阶段重写（mermaid 写错一个词
    也会把同阶段其余章全部作废重来）；记下复合键即可只重扩这一章。
    """
    failed.add(index)
    failed_chapters.add(f"{index}:{chapter_index}")


def validate_plan(
    plan_dict: dict,
    weekly_hours: int,
    weeks: int,
    knowledge_points: dict[tuple[int, int], list[str]] | None = None,
    *,
    scope_phase: int | None = None,
    scope_chapters: list[int] | None = None,
) -> tuple[list[str], list[int], list[str]]:
    """校验 camelCase 计划 dict。

    scope_phase / scope_chapters 为重生成作用域（2026-10-22 编排统一）：
    此时 plan_dict 只含目标阶段的若干章，阶段级索引要回填成真实值，
    且计划级规则（阶段数、总学时漂移）不适用——它们衡量的是整份计划，
    而重生成只负责眼前这一段，跑它只会制造与本次无关的假失败。

    返回（问题描述列表, 不合格阶段索引列表, 不合格章复合键列表 "阶段:章"）。
    阶段索引仍覆盖全部不合格阶段（回炉入口）；章级键是它的精度提升——调用方
    有章级键时只重扩这些章，没有时（结构性问题）才整阶段回炉。

    knowledge_points 由调用方从大纲转成 {(阶段索引, 章索引): [知识点]}；
    缺省（旧路径 / mock 无大纲）时跳过覆盖率校验。
    """
    issues: list[str] = []
    failed_indexes: set[int] = set()
    failed_chapters: set[str] = set()

    phases = plan_dict.get("phases") or []
    scoped = scope_phase is not None
    if not scoped and not (MIN_PHASES <= len(phases) <= MAX_PHASES):
        issues.append(f"阶段数为 {len(phases)}，需在 {MIN_PHASES}-{MAX_PHASES} 个之间")
        # 规则 1 全局失败：映射所有阶段索引（0 个阶段时为空集）
        failed_indexes.update(range(len(phases)))

    points_by_phase: dict[int, dict[int, list[str]]] = {}
    for (phase_index, chapter_index), points in (knowledge_points or {}).items():
        points_by_phase.setdefault(phase_index, {})[chapter_index] = points

    for local_index, phase in enumerate(phases):
        # 作用域下 plan_dict 只含目标阶段，真实索引由 scope_phase 回填：
        # 否则回炉会按 0 号阶段找章，打错目标
        index = scope_phase if scoped else local_index
        _check_phase(
            phase,
            index,
            issues,
            failed_indexes,
            failed_chapters,
            points_by_phase.get(index),
            scope_chapters=scope_chapters,
            skip_phase_level=scope_chapters is not None,
        )

    if not scoped:
        _check_total_hours(plan_dict, weekly_hours, weeks, issues)

    return (
        issues,
        sorted(failed_indexes),
        sorted(failed_chapters, key=_chapter_sort_key),
    )


def _check_phase(
    phase: dict,
    index: int,
    issues: list[str],
    failed: set[int],
    failed_chapters: set[str],
    chapter_points: dict[int, list[str]] | None = None,
    scope_chapters: list[int] | None = None,
    skip_phase_level: bool = False,
) -> None:
    """单阶段校验：章节正文 / 项目 / 目标（markdown 化后的规则 2-6）。

    scope_chapters 为重生成作用域内的真实章索引：此时 chapters 列表只含
    目标章，回填真实索引才能让章级失败键命中回炉目标、知识点覆盖也查得对。

    skip_phase_level=单章重生成：项目/作业等阶段级产物这次根本不生产，
    校验它们只会把「修一章」误判成「阶段结构性问题」并连带回炉整个阶段。
    """
    label = f"第 {index + 1} 阶段「{phase.get('name', '')}」"

    chapters = phase.get("chapters") or []
    if not (MIN_CHAPTERS_PER_PHASE <= len(chapters) <= MAX_CHAPTERS_PER_PHASE):
        issues.append(
            f"{label}章节数为 {len(chapters)}，"
            f"需在 {MIN_CHAPTERS_PER_PHASE}-{MAX_CHAPTERS_PER_PHASE} 个之间"
        )
        failed.add(index)
        return

    for local_index, chapter in enumerate(chapters):
        chapter_index = scope_chapters[local_index] if scope_chapters else local_index
        _check_chapter(
            chapter,
            f"{label}第 {chapter_index + 1} 章",
            issues,
            failed,
            failed_chapters,
            index,
            chapter_index,
            (chapter_points or {}).get(chapter_index),
        )

    if not skip_phase_level:
        _check_project(phase, label, issues, failed, index)

    if not (phase.get("objectives") or []):
        issues.append(f"{label}缺少阶段目标（objectives 为空）")
        failed.add(index)


def _check_chapter(
    chapter: dict,
    label: str,
    issues: list[str],
    failed: set[int],
    failed_chapters: set[str],
    index: int,
    chapter_index: int,
    knowledge_points: list[str] | None = None,
) -> None:
    """单章校验：markdown 正文非空、≥ 1000 字、```mermaid 码块首词白名单。"""
    title = chapter.get("title", "")
    label = f"{label}「{title}」"

    content = (chapter.get("contentMd") or "").strip()
    if not content:
        issues.append(f"{label}缺少 markdown 正文（contentMd 为空，可重新生成）")
        _mark_chapter(failed, failed_chapters, index, chapter_index)
        return
    if len(content) < MIN_CHAPTER_CONTENT_CHARS:
        issues.append(
            f"{label}正文仅 {len(content)} 字，要求不少于 {MIN_CHAPTER_CONTENT_CHARS} 字"
        )
        _mark_chapter(failed, failed_chapters, index, chapter_index)

    for block in _mermaid_code_blocks(content):
        first_word = _mermaid_first_word(block)
        if first_word not in MERMAID_DIAGRAM_TYPES:
            issues.append(
                f"{label}Mermaid 码块首词「{first_word or '（空）'}」不在支持类型内"
                f"（{sorted(MERMAID_DIAGRAM_TYPES)}）"
            )
            _mark_chapter(failed, failed_chapters, index, chapter_index)

    kind = str(chapter.get("kind") or DEFAULT_CHAPTER_KIND)
    required = REQUIRED_SECTIONS_BY_KIND.get(
        kind, REQUIRED_SECTIONS_BY_KIND[DEFAULT_CHAPTER_KIND]
    )
    missing = [name for name in required if name not in content]
    if missing:
        issues.append(f"{label}（{kind}）缺少必备小节：{'、'.join(missing)}")
        _mark_chapter(failed, failed_chapters, index, chapter_index)

    # 概念章的深入剖析是唯一强制的深度要求（800 字目标、600 字硬底线）：
    # 其他章型的价值在动手与验收，不该被同一把尺子罚
    if kind == "concept":
        deep_chars = _deep_dive_chars(content)
        if deep_chars is None:
            issues.append(
                f"{label}（concept）缺少「{DEEP_DIVE_KEYWORD}」小节："
                "概念章必须有一节深入剖析"
            )
            _mark_chapter(failed, failed_chapters, index, chapter_index)
        elif deep_chars < DEEP_DIVE_MIN_CHARS:
            issues.append(
                f"{label}（concept）「{DEEP_DIVE_KEYWORD}」小节仅 {deep_chars} 字，"
                f"要求不少于 {DEEP_DIVE_MIN_CHARS} 字"
            )
            _mark_chapter(failed, failed_chapters, index, chapter_index)

    if knowledge_points:
        missed = _check_coverage(
            content, knowledge_points, title, label, issues
        )
        hit_ratio = 1 - len(missed) / len(knowledge_points)
        if hit_ratio < COVERAGE_MIN_RATIO:
            issues.append(
                f"{label}知识点覆盖率 {hit_ratio:.0%}"
                f"（{len(knowledge_points) - len(missed)}/{len(knowledge_points)}），"
                f"低于 {COVERAGE_MIN_RATIO:.0%}：正文没有按大纲知识点组织小节"
            )
            _mark_chapter(failed, failed_chapters, index, chapter_index)


def _section_titles(content: str) -> list[str]:
    """提取正文的 ## / ### 小节标题（含章节标题那一行，由调用方排除）。"""
    titles: list[str] = []
    for line in content.splitlines():
        match = re.match(r"^#{2,3}\s+(.+?)\s*$", line)
        if match:
            titles.append(match.group(1).strip())
    return titles


def _coverage_tokens(point: str) -> list[str]:
    """把知识点切成可用于标题匹配的 token（剔除虚词，保留 ≥2 字片段）。"""
    text = str(point or "")
    for word in _COVERAGE_STOPWORDS:
        text = text.replace(word, " ")
    return [token for token in re.split(r"[^\w]+", text) if len(token) >= 2]


def _point_covered(point: str, titles: list[str]) -> bool:
    """知识点是否落到某个小节标题上：任一 token 出现在任一标题即算命中。"""
    tokens = _coverage_tokens(point)
    if not tokens:
        # 整条都是虚词，无法检验，按已覆盖处理（不罚）
        return True
    return any(token in title for title in titles for token in tokens)


def _check_coverage(
    content: str,
    knowledge_points: list[str],
    chapter_title: str,
    label: str,
    issues: list[str],
) -> list[str]:
    """检查正文小节是否覆盖大纲知识点，返回未命中的知识点列表。

    正向（必须）：每个知识点都要在小节标题里有落点，命中率低于
    COVERAGE_MIN_RATIO 才判不合格——措辞差异不该频繁误杀并白烧回炉调用。
    反向（提示）：与任何知识点都对不上的小节只记 issue 不罚：它可能是
    正常过渡段，只是注水信号，交由评审与读者判断。
    """
    titles = [
        title
        for title in _section_titles(content)
        if title != chapter_title
        and not any(skip in title for skip in _COVERAGE_SKIP_TITLES)
    ]

    missed = [point for point in knowledge_points if not _point_covered(point, titles)]
    extra = [
        title
        for title in titles
        if not any(_point_covered(point, [title]) for point in knowledge_points)
    ]
    # 单个过渡小节（如「为什么需要它」）是正常写法，不算注水；
    # 只有小节数明显多于知识点时才值得提示
    if len(extra) >= 2:
        issues.append(
            f"{label}有 {len(extra)} 个小节与大纲知识点对不上"
            f"（可能是补充内容，也可能是注水）：{'、'.join(extra[:3])}"
        )
    return missed


def _deep_dive_chars(content: str) -> int | None:
    """返回标题含「深入」的小节正文长度；该小节不存在时返回 None。

    小节边界按 markdown 标题级别判定：从标题行的下一行起，到下一个
    同级或更高级标题为止——所以「### 深入：××」会被后面的「### 动手练习」
    正确截断，不会把后续小节的字数算进来。
    """
    lines = content.splitlines()
    start = -1
    start_level = 0
    for i, line in enumerate(lines):
        stripped = line.strip()
        if not stripped.startswith("#"):
            continue
        level = len(stripped) - len(stripped.lstrip("#"))
        if start < 0:
            if DEEP_DIVE_KEYWORD in stripped[level:]:
                start = i + 1
                start_level = level
            continue
        if level <= start_level:
            return len(chr(10).join(lines[start:i]).strip())
    if start >= 0:
        return len(chr(10).join(lines[start:]).strip())
    return None


def _mermaid_code_blocks(content: str) -> list[str]:
    """扫描 markdown 正文中的 ```mermaid 码块，返回围栏内源码列表。"""
    blocks: list[str] = []
    in_block = False
    chunk: list[str] = []
    for line in content.splitlines():
        stripped = line.strip()
        if not in_block:
            if stripped.startswith("```mermaid"):
                in_block = True
            continue
        if stripped.startswith("```"):
            blocks.append("\n".join(chunk))
            chunk = []
            in_block = False
            continue
        chunk.append(line)
    # 未闭合的码块：把已收集内容当作源码交给首词检查（提示 LLM 修复）
    if in_block and any(chunk):
        blocks.append("\n".join(chunk))
    return blocks


def _check_project(
    phase: dict, label: str, issues: list[str], failed: set[int], index: int
) -> None:
    """阶段项目完整性：name / description 非空、steps 与 acceptanceCriteria 条数、学时。"""
    project = phase.get("project")
    if not project:
        issues.append(f"{label}缺少阶段项目（project 缺失）")
        failed.add(index)
        return

    if not (project.get("name") or "").strip():
        issues.append(f"{label}项目名称为空（project.name）")
        failed.add(index)
    if not (project.get("description") or "").strip():
        issues.append(f"{label}项目描述为空（project.description）")
        failed.add(index)
    if len(project.get("steps") or []) < MIN_PROJECT_STEPS:
        issues.append(f"{label}项目步骤不足 {MIN_PROJECT_STEPS} 条（project.steps）")
        failed.add(index)
    if len(project.get("acceptanceCriteria") or []) < MIN_PROJECT_ACCEPTANCE:
        issues.append(
            f"{label}项目验收标准不足 {MIN_PROJECT_ACCEPTANCE} 条"
            f"（project.acceptanceCriteria）"
        )
        failed.add(index)

    hours = project.get("estimatedHours")
    if not isinstance(hours, (int, float)) or hours <= 0:
        issues.append(f"{label}项目预计学时必须大于 0（project.estimatedHours）")
        failed.add(index)


def _check_total_hours(
    plan_dict: dict, weekly_hours: int, weeks: int, issues: list[str]
) -> None:
    """总学时偏差校验（规则 8）：只写 issues，不产生阶段索引。"""
    expected = weekly_hours * weeks
    if expected <= 0:
        # 期望学时为 0 时跳过此校验
        return
    actual = plan_dict.get("estimatedTotalHours") or 0
    drift = abs(actual - expected) / expected
    if drift > MAX_HOUR_DRIFT:
        issues.append(
            f"预计总学时 {actual} 与可用学时（每周 {weekly_hours} 小时 × "
            f"{weeks} 周 = {expected}）偏差 {drift:.0%}，超过上限 {MAX_HOUR_DRIFT:.0%}"
        )


def _mermaid_first_word(mermaid: str) -> str:
    """剥离 ``` 围栏行后返回 Mermaid 源码首个单词（源码可能带围栏或前后空白）。"""
    body_lines = [
        line for line in mermaid.splitlines() if not line.strip().startswith("```")
    ]
    source = "\n".join(body_lines).strip()
    if not source:
        return ""
    return source.split()[0]
