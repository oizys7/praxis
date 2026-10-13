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

# ---------------------------------------------------------------------------
# 校验阈值（模块顶部常量）
# ---------------------------------------------------------------------------

# 阶段数范围
MIN_PHASES = 1
MAX_PHASES = 6
# 每阶段章节数范围
MIN_CHAPTERS_PER_PHASE = 1
MAX_CHAPTERS_PER_PHASE = 5
# 每章 markdown 正文最少字数（strip 后；contentMd 空直接 failed 可重生成）
MIN_CHAPTER_CONTENT_CHARS = 1000
# 阶段项目最少步骤 / 验收标准条数
MIN_PROJECT_STEPS = 2
MIN_PROJECT_ACCEPTANCE = 2
# 总学时相对偏差上限（|实际-期望|/期望）
MAX_HOUR_DRIFT = 0.2
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


def validate_plan(
    plan_dict: dict, weekly_hours: int, weeks: int
) -> tuple[list[str], list[int]]:
    """校验 camelCase 计划 dict，返回（问题描述列表, 不合格阶段索引列表 0-based）。"""
    issues: list[str] = []
    failed_indexes: set[int] = set()

    phases = plan_dict.get("phases") or []
    if not (MIN_PHASES <= len(phases) <= MAX_PHASES):
        issues.append(
            f"阶段数为 {len(phases)}，需在 {MIN_PHASES}-{MAX_PHASES} 个之间"
        )
        # 规则 1 全局失败：映射所有阶段索引（0 个阶段时为空集）
        failed_indexes.update(range(len(phases)))

    for index, phase in enumerate(phases):
        _check_phase(phase, index, issues, failed_indexes)

    _check_total_hours(plan_dict, weekly_hours, weeks, issues)

    return issues, sorted(failed_indexes)


def _check_phase(phase: dict, index: int, issues: list[str], failed: set[int]) -> None:
    """单阶段校验：章节 / 深入内容 / 主要知识 / 项目 / 目标（规则 2-7）。"""
    label = f"第 {index + 1} 阶段「{phase.get('name', '')}」"

    chapters = phase.get("chapters") or []
    if not (MIN_CHAPTERS_PER_PHASE <= len(chapters) <= MAX_CHAPTERS_PER_PHASE):
        issues.append(
            f"{label}章节数为 {len(chapters)}，"
            f"需在 {MIN_CHAPTERS_PER_PHASE}-{MAX_CHAPTERS_PER_PHASE} 个之间"
        )
        failed.add(index)
        return

    for chapter_index, chapter in enumerate(chapters):
        _check_chapter(chapter, f"{label}第 {chapter_index + 1} 章", issues, failed, index)

    _check_project(phase, label, issues, failed, index)

    if not (phase.get("objectives") or []):
        issues.append(f"{label}缺少阶段目标（objectives 为空）")
        failed.add(index)


def _check_chapter(
    chapter: dict, label: str, issues: list[str], failed: set[int], index: int
) -> None:
    """单章校验：markdown 正文非空、≥ 1000 字、```mermaid 码块首词白名单。"""
    title = chapter.get("title", "")
    label = f"{label}「{title}」"

    content = (chapter.get("contentMd") or "").strip()
    if not content:
        issues.append(f"{label}缺少 markdown 正文（contentMd 为空，可重新生成）")
        failed.add(index)
        return
    if len(content) < MIN_CHAPTER_CONTENT_CHARS:
        issues.append(
            f"{label}正文仅 {len(content)} 字，要求不少于 {MIN_CHAPTER_CONTENT_CHARS} 字"
        )
        failed.add(index)

    for block in _mermaid_code_blocks(content):
        first_word = _mermaid_first_word(block)
        if first_word not in MERMAID_DIAGRAM_TYPES:
            issues.append(
                f"{label}Mermaid 码块首词「{first_word or '（空）'}」不在支持类型内"
                f"（{sorted(MERMAID_DIAGRAM_TYPES)}）"
            )
            failed.add(index)


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
        issues.append(
            f"{label}项目步骤不足 {MIN_PROJECT_STEPS} 条（project.steps）"
        )
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
