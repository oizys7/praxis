"""validators.py 程序化校验器测试：每条校验规则一正一反。

校验器吃 camelCase dict（assemble_node 中 StudyPlan 以 dict 形态流转），
本文件所有 fixture 均为 camelCase 键。
"""

import pytest

from app.validators import validate_plan

# 合法基准：3 个阶段 × 2 章，总学时 120 = weekly_hours(10) × weeks(12)


def _make_chapter(title: str = "章节", content_chars: int = 1000) -> dict:
    """章节正文：确定性 markdown，含知识点段落与一个 flowchart 码块。"""
    return {
        "title": title,
        "summary": "章节概述",
        "contentMd": (
            f"## {title}\n\n"
            f"### 概念入门\n\n" + "深" * content_chars
            + "\n\n```mermaid\nflowchart TD\n    A --> B\n```\n"
        ),
    }


def _make_phase(name: str = "阶段", chapters: int = 2) -> dict:
    return {
        "name": name,
        "weekRange": "第 1-4 周",
        "objectives": ["目标一", "目标二"],
        "chapters": [_make_chapter(f"章节{i}") for i in range(chapters)],
        "practice": ["实操任务"],
        "assignments": ["课后作业"],
        "project": {
            "name": "阶段项目",
            "description": "项目描述",
            "background": "项目背景",
            "steps": ["步骤一", "步骤二", "步骤三"],
            "estimatedHours": 40,
            "acceptanceCriteria": ["验收标准一", "验收标准二"],
        },
    }


def _make_plan(phases: int = 3) -> dict:
    return {
        "title": "学习计划",
        "overview": "整体概述",
        "estimatedTotalHours": 120,
        "phases": [_make_phase(f"阶段{i}") for i in range(phases)],
        "resources": [],
    }


# ---------------------------------------------------------------------------
# 规则 1：阶段数 1-8（边界：8 过，9 拒）
# ---------------------------------------------------------------------------


def test_phase_count_out_of_range_rejected():
    issues, pending = validate_plan(_make_plan(0), weekly_hours=10, weeks=12)
    assert issues, "0 个阶段应产生 issue"
    assert pending == [], "无阶段可映射，索引列表应为空"

    issues, pending = validate_plan(_make_plan(9), weekly_hours=10, weeks=12)
    assert issues, "9 个阶段应产生 issue"
    assert pending == list(range(9)), "阶段数全局失败应映射所有阶段索引"


def test_phase_count_upper_boundary_accepted():
    """量级放宽边界：MAX_PHASES=8 应整单通过。"""
    issues, pending = validate_plan(_make_plan(8), weekly_hours=10, weeks=12)
    assert issues == [], "8 个阶段处于上限边界，应零 issue 通过"
    assert pending == []


# ---------------------------------------------------------------------------
# 规则 2：每阶段章节数 1-7（边界：7 过，8 拒）
# ---------------------------------------------------------------------------


def test_chapter_count_out_of_range_rejected():
    plan = _make_plan(3)
    plan["phases"][1]["chapters"] = []
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 1 in pending and 0 not in pending and 2 not in pending

    plan = _make_plan(2)
    plan["phases"][0]["chapters"] = [_make_chapter(f"章节{i}") for i in range(8)]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "8 个章节超出上限应产生 issue"
    assert 0 in pending and 1 not in pending


def test_chapter_count_upper_boundary_accepted():
    """量级放宽边界：MAX_CHAPTERS_PER_PHASE=7 应整阶段通过。"""
    plan = _make_plan(3)
    plan["phases"][0]["chapters"] = [_make_chapter(f"章节{i}") for i in range(7)]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues == [], "单阶段 7 个章节处于上限边界，应零 issue 通过"
    assert pending == []


# ---------------------------------------------------------------------------
# 规则 3：contentMd 非空且 ≥ 1000 字
# ---------------------------------------------------------------------------


def test_valid_plan_passes():
    result = validate_plan(_make_plan(), weekly_hours=10, weeks=12)
    assert isinstance(result, tuple) and len(result) == 2
    issues, pending = result
    assert issues == []
    assert pending == []


def test_shallow_content_rejected():
    plan = _make_plan(3)
    for chapter in plan["phases"][1]["chapters"]:
        chapter["contentMd"] = "浅" * 999
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "999 字正文应被拒绝"
    assert any("1000" in issue for issue in issues), "浅内容 issue 需含阈值 1000"
    assert 1 in pending, "不合格阶段索引应进 pending"
    assert 0 not in pending and 2 not in pending


def test_empty_content_md_failed_for_regen():
    """空正文 → failed_indexes：可重生成的失败形态（浅轮裁定）。"""
    plan = _make_plan(2)
    plan["phases"][0]["chapters"][1]["contentMd"] = "   "
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "空白 contentMd 应按缺少正文处理而非抛错"
    assert 0 in pending and 1 not in pending

    plan = _make_plan(2)
    del plan["phases"][1]["chapters"][0]["contentMd"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 contentMd 键应按不满足规则处理而非抛错"
    assert 1 in pending and 0 not in pending


# ---------------------------------------------------------------------------
# 规则 5：markdown 内联 ```mermaid 码块首词在白名单
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "diagram_type",
    [
        "graph",
        "flowchart",
        "sequenceDiagram",
        "classDiagram",
        "stateDiagram",
        "erDiagram",
        "gantt",
        "mindmap",
    ],
)
def test_mermaid_block_whitelisted_prefix(diagram_type):
    """正文内联码块首词在白名单内的 8 类图应全部通过。"""
    plan = _make_plan(1)
    chapters = plan["phases"][0]["chapters"]
    blocks = (
        f"\n\n```mermaid\n  {diagram_type} 示例\n    A --> B\n```\n",
        f"\n\n```mermaid\n{diagram_type} Td\n    A --> B\n```\n",
    )
    for chapter, block in zip(chapters, blocks, strict=False):
        chapter["contentMd"] = chapter["contentMd"].rstrip() + block
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues == [], f"码块首词 {diagram_type} 应被容忍"
    assert pending == []


def test_bad_mermaid_block_prefix_rejected():
    plan = _make_plan(3)
    for chapter in plan["phases"][2]["chapters"]:
        chapter["contentMd"] = (
            chapter["contentMd"].rstrip()
            + "\n\n```mermaid\npie title 占比\n    A : 50\n    B : 50\n```\n"
        )
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "白名单外的首词 pie 应被拒绝"
    assert 2 in pending and 0 not in pending and 1 not in pending


def test_unclosed_mermaid_block_checked_by_prefix():
    """未闭合码块同样按首词检查（提示 LLM 修复围栏）。"""
    plan = _make_plan(2)
    for chapter in plan["phases"][1]["chapters"]:
        chapter["contentMd"] = (
            chapter["contentMd"].rstrip() + "\n\n```mermaid\npie xp\n"
        )
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "未闭合码块首词 pie 应被拒绝"
    assert 1 in pending and 0 not in pending


# ---------------------------------------------------------------------------
# 规则 6：project 完整性
# ---------------------------------------------------------------------------


def test_missing_project_rejected():
    plan = _make_plan(2)
    plan["phases"][0]["project"] = None
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 0 in pending and 1 not in pending

    plan = _make_plan(2)
    del plan["phases"][1]["project"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 project 键应按不满足规则处理而非抛错"
    assert 1 in pending and 0 not in pending


def test_missing_project_steps_rejected():
    plan = _make_plan(2)
    plan["phases"][1]["project"]["steps"] = ["只有一条步骤"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "steps 少于 2 条应被拒绝"
    assert 1 in pending and 0 not in pending


def test_project_incomplete_fields_rejected():
    plan = _make_plan(4)
    plan["phases"][0]["project"]["name"] = "  "
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues and 0 in pending

    plan = _make_plan(4)
    plan["phases"][1]["project"]["description"] = ""
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "description 为空应被拒绝"
    assert 1 in pending and 0 not in pending

    plan = _make_plan(4)
    plan["phases"][2]["project"]["acceptanceCriteria"] = ["唯一一条"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues and 2 in pending

    plan = _make_plan(4)
    plan["phases"][3]["project"]["estimatedHours"] = 0
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "estimatedHours 为 0 应被拒绝"
    assert 3 in pending and 0 not in pending and 1 not in pending and 2 not in pending


# ---------------------------------------------------------------------------
# 规则 7：objectives 非空
# ---------------------------------------------------------------------------


def test_empty_objectives_rejected():
    plan = _make_plan(2)
    plan["phases"][0]["objectives"] = []
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 0 in pending and 1 not in pending

    plan = _make_plan(2)
    del plan["phases"][1]["objectives"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 objectives 键应按不满足规则处理而非抛错"
    assert 1 in pending and 0 not in pending


# ---------------------------------------------------------------------------
# 规则 8：estimatedTotalHours 与 weekly_hours × weeks 偏差 ≤ 20%
# ---------------------------------------------------------------------------


def test_hours_mismatch_rejected():
    plan = _make_plan(3)
    plan["estimatedTotalHours"] = 200  # 偏差 |200-120|/120 ≈ 66.7% > 20%
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "学时偏差 > 20% 应产生 issue"
    assert pending == [], "裁决：学时偏差只写 issues，不产生 pending 阶段索引"

    plan = _make_plan(1)
    del plan["estimatedTotalHours"]  # 缺失键按 0 处理，偏差 100%
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 estimatedTotalHours 应按不满足规则处理而非抛错"
    assert pending == []


def test_hours_check_skipped_when_expected_zero():
    plan = _make_plan(2)
    plan["estimatedTotalHours"] = 99999
    issues, pending = validate_plan(plan, weekly_hours=0, weeks=12)
    assert issues == [], "期望学时为 0 时应跳过学时校验"
    assert pending == []
