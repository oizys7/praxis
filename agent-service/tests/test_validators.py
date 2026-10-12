"""validators.py 程序化校验器测试：每条校验规则一正一反。

校验器吃 camelCase dict（assemble_node 中 StudyPlan 以 dict 形态流转），
本文件所有 fixture 均为 camelCase 键。
"""

import pytest

from app.validators import validate_plan

# 合法基准：3 个阶段 × 2 章，总学时 120 = weekly_hours(10) × weeks(12)


def _make_chapter(title: str = "章节", deep_dive_chars: int = 600) -> dict:
    return {
        "title": title,
        "summary": "章节概述",
        "mainKnowledge": {
            "concepts": [{"name": "核心概念", "definition": "一句话定义"}],
            "knowledgePoints": [{"title": "知识点", "detail": "要点说明"}],
            "diagrams": [
                {
                    "title": "流程图",
                    "mermaid": "flowchart TD\n    A --> B",
                    "caption": "图注说明",
                }
            ],
        },
        "deepDives": [
            {"topic": "深入主题", "angle": "切入角度", "content": "深" * deep_dive_chars}
        ],
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
# 规则 1：阶段数 1-6
# ---------------------------------------------------------------------------


def test_phase_count_out_of_range_rejected():
    issues, pending = validate_plan(_make_plan(0), weekly_hours=10, weeks=12)
    assert issues, "0 个阶段应产生 issue"
    assert pending == [], "无阶段可映射，索引列表应为空"

    issues, pending = validate_plan(_make_plan(7), weekly_hours=10, weeks=12)
    assert issues, "7 个阶段应产生 issue"
    assert pending == list(range(7)), "阶段数全局失败应映射所有阶段索引"


# ---------------------------------------------------------------------------
# 规则 2：每阶段章节数 1-5
# ---------------------------------------------------------------------------


def test_chapter_count_out_of_range_rejected():
    plan = _make_plan(3)
    plan["phases"][1]["chapters"] = []
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 1 in pending and 0 not in pending and 2 not in pending

    plan = _make_plan(2)
    plan["phases"][0]["chapters"] = [_make_chapter(f"章节{i}") for i in range(6)]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 0 in pending and 1 not in pending


# ---------------------------------------------------------------------------
# 规则 3：deepDives 非空且每篇 ≥ 500 字
# ---------------------------------------------------------------------------


def test_valid_plan_passes():
    result = validate_plan(_make_plan(), weekly_hours=10, weeks=12)
    assert isinstance(result, tuple) and len(result) == 2
    issues, pending = result
    assert issues == []
    assert pending == []


def test_shallow_deep_dive_rejected():
    plan = _make_plan(3)
    plan["phases"][1]["chapters"][0]["deepDives"][0]["content"] = "浅" * 499
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "499 字深入内容应被拒绝"
    assert any("500" in issue for issue in issues), "浅内容 issue 需含数字 500"
    assert 1 in pending, "不合格阶段索引应进 pending"
    assert 0 not in pending and 2 not in pending


def test_empty_deep_dives_rejected():
    plan = _make_plan(2)
    del plan["phases"][0]["chapters"][1]["deepDives"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 deepDives 键应按不满足规则处理而非抛错"
    assert 0 in pending and 1 not in pending

    plan = _make_plan(2)
    plan["phases"][1]["chapters"][0]["deepDives"] = []
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "deepDives 空列表应被拒绝"
    assert 1 in pending and 0 not in pending


# ---------------------------------------------------------------------------
# 规则 4：concepts / knowledgePoints 非空
# ---------------------------------------------------------------------------


def test_missing_main_knowledge_rejected():
    plan = _make_plan(3)
    plan["phases"][0]["chapters"][0]["mainKnowledge"]["concepts"] = []
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues
    assert 0 in pending

    plan = _make_plan(3)
    del plan["phases"][2]["chapters"][0]["mainKnowledge"]["knowledgePoints"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 knowledgePoints 键应按不满足规则处理而非抛错"
    assert 2 in pending and 0 not in pending


# ---------------------------------------------------------------------------
# 规则 5：diagrams[].mermaid 非空且首词在白名单（容忍 ``` 围栏）
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
def test_mermaid_fenced_source_tolerated(diagram_type):
    plan = _make_plan(1)
    plan["phases"][0]["chapters"][0]["mainKnowledge"]["diagrams"][0]["mermaid"] = (
        f"```mermaid\n  {diagram_type} 示例\n    A --> B\n```\n"
    )
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues == [], f"围栏包裹的 {diagram_type} 应被容忍"
    assert pending == []


def test_bad_mermaid_prefix_rejected():
    plan = _make_plan(3)
    plan["phases"][2]["chapters"][0]["mainKnowledge"]["diagrams"][0]["mermaid"] = (
        "pie title 占比\n    A : 50\n    B : 50"
    )
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "白名单外的首词 pie 应被拒绝"
    assert 2 in pending and 0 not in pending and 1 not in pending


def test_empty_mermaid_rejected():
    plan = _make_plan(2)
    plan["phases"][0]["chapters"][0]["mainKnowledge"]["diagrams"][0]["mermaid"] = ""
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "空 Mermaid 源码应被拒绝"
    assert 0 in pending

    plan = _make_plan(2)
    del plan["phases"][1]["chapters"][0]["mainKnowledge"]["diagrams"][0]["mermaid"]
    issues, pending = validate_plan(plan, weekly_hours=10, weeks=12)
    assert issues, "缺失 mermaid 键应按不满足规则处理而非抛错"
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
