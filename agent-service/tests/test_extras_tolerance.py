"""PhaseExtrasOutput 输出容错（2026-10-22）。

真实运行里 4 次解析失败有 3 类同源：GLM 把「一句话一步」写成结构化对象
（project.steps.0-4: Input should be a valid string，一次 8 条校验错误），
还有一次只给 steps 不给 name。以下用例锁住归一行为，避免为了格式瑕疵
整轮重烧 LLM 调用。
"""

from app.schemas import PhaseExtrasOutput, PhaseProject


def _extras(**overrides):
    base = {
        "practice": ["动手实现一个最小可用版本"],
        "assignments": ["写一篇 800 字复盘"],
        "project": {"name": "检索增强问答", "steps": ["搭骨架", "接数据"]},
    }
    base.update(overrides)
    return base


def test_string_lists_pass_through_unchanged():
    """正常字符串数组不受容错逻辑影响。"""
    model = PhaseExtrasOutput.model_validate(_extras())
    assert model.practice == ["动手实现一个最小可用版本"]
    assert model.project.steps == ["搭骨架", "接数据"]


def test_object_steps_are_flattened_to_strings():
    """对象形式的步骤取 action 文本，而不是整轮失败。"""
    model = PhaseExtrasOutput.model_validate(
        _extras(
            project={
                "name": "项目甲",
                "steps": [
                    {"step": 1, "action": "安装依赖并初始化工程"},
                    {"step": 2, "action": "实现检索接口"},
                ],
            }
        )
    )
    assert model.project.steps == ["安装依赖并初始化工程", "实现检索接口"]


def test_object_without_known_text_key_falls_back_to_flattened_pairs():
    """没有 action/description 等键时按「键：值」展平，信息不丢。"""
    model = PhaseExtrasOutput.model_validate(
        _extras(project={"name": "项目乙", "steps": [{"阶段": "准备", "产出": "设计稿"}]})
    )
    assert model.project.steps == ["阶段：准备；产出：设计稿"]


def test_acceptance_criteria_and_practice_tolerate_objects():
    """验收标准与实操/作业列表享受同一套容错。"""
    model = PhaseExtrasOutput.model_validate(
        _extras(
            practice=[{"task": "跑通端到端链路", "hours": 3}],
            assignments=[{"description": "整理笔记"}],
            project={
                "name": "项目丙",
                "acceptanceCriteria": [{"text": "检索结果可复现"}, "冷启动 10 秒内完成"],
            },
        )
    )
    assert model.practice == ["跑通端到端链路"]
    assert model.assignments == ["整理笔记"]
    assert model.project.acceptance_criteria == ["检索结果可复现", "冷启动 10 秒内完成"]


def test_non_string_scalars_are_stringified():
    """数字等标量转成字符串（模型偶发给编号）。"""
    model = PhaseExtrasOutput.model_validate(
        _extras(project={"name": "项目丁", "steps": [1, 2, 3]})
    )
    assert model.project.steps == ["1", "2", "3"]


def test_missing_name_defaults_to_empty_string():
    """缺 name 不再整体校验失败（空名由 nodes 侧补兜底文案）。"""
    model = PhaseExtrasOutput.model_validate(
        _extras(project={"steps": ["第一步"], "acceptanceCriteria": ["能跑"]})
    )
    assert model.project.name == ""


def test_non_list_value_is_left_to_pydantic():
    """非列表原样返回，不掩盖真正的类型错误。"""
    try:
        PhaseExtrasOutput.model_validate(_extras(project={"name": "x", "steps": "只有一步"}))
    except ValueError:
        return
    raise AssertionError("steps 为字符串时应由 Pydantic 报错")


def test_project_steps_accept_camel_alias():
    """camelCase 别名仍可用（三端协议对齐）。"""
    model = PhaseProject.model_validate(
        {"name": "别名项目", "estimatedHours": 12, "acceptanceCriteria": ["能跑"]}
    )
    assert model.estimated_hours == 12
    assert model.acceptance_criteria == ["能跑"]
