"""prompts 渲染 smoke：outline / phase / chapter 三份提示词全占位符渲染成功。

随任务 19（叙事化重写 + level 注入）引入：
- 渲染后不得残留 {{ var }} 占位符（漏传变量会静默渲染为空串，破坏受众/学时注入）；
- phase / chapter 的 mermaid 白名单须与 validators.MERMAID_DIAGRAM_TYPES 逐字一致
  （大小写敏感，写错将无法渲染）。
"""

from __future__ import annotations

import re

import pytest

from app import prompts, validators

# 叙事化提示词（章级扇出拓扑：chapter 为主链 + 重生成共用；
# phase_extras 为 wrap 轻量收尾；phase 仅发掘于 regen 全阶段路径）
_STORY_PROMPTS = ("outline", "phase", "phase_extras", "chapter")

# 与各提示词 human 模板占位符一一对应的填充值（无默认、全覆盖）
_FULL_VARIABLES: dict[str, str] = {
    "goal": "掌握 Web 前端",
    "level": "从零开始的初学者（没有任何相关经验）",
    "weekly_hours": "10",
    "weeks": "12",
    "interests": "工程实践",
    "background": "有 Python 基础",
    "phase_name": "第一阶段 · 打基础",
    "phase_week_range": "第 1-4 周",
    "phase_objectives": "- 建立核心概念框架",
    "chapter_title": "核心概念入门",
    "sibling_chapter_titles": "- 动手初体验",
    "project_brief": "完成一个入门级小作品",
    "plan_context": "计划标题：X\n整体概述：Y",
    "phase_outline": '{"name": "第一阶段", "chapterTitles": ["核心概念入门"]}',
    "review_notes": "（首次生成，无评审意见）",
}


def _placeholders(text: str) -> set[str]:
    """从模板文本抽取 {{ var }} 占位符变量名（与 app.prompts 同一模式）。"""
    return set(re.findall(r"\{\{\s*(\w+)\s*\}\}", text))


@pytest.mark.parametrize("name", _STORY_PROMPTS)
def test_story_prompt_fully_rendered(name):
    """三份提示词的所有占位符都能被全变量字典渲染，且不残留 {{ }}。"""
    template = prompts.load_prompt(name)
    needed = _placeholders(template["human"])
    missing = needed - set(_FULL_VARIABLES)
    assert not missing, f"{name}.yaml 存在未覆盖的占位符：{sorted(missing)}"

    rendered = prompts.render(template["human"], **_FULL_VARIABLES)
    assert "{{" not in rendered and "}}" not in rendered, (
        f"{name} 渲染后残留未替换占位符"
    )


@pytest.mark.parametrize("name", ("phase", "chapter"))
def test_mermaid_whitelist_matches_validators(name):
    """正文规格里的图类型白名单与 validators.MERMAID_DIAGRAM_TYPES 词级逐字一致。

    子串式断言（diagram_type in system_text）拦不住未来混入的超集词
    （如 sequenceDiagramX），故用词边界匹配。
    """
    system_text = prompts.load_prompt(name)["system"]
    for diagram_type in validators.MERMAID_DIAGRAM_TYPES:
        assert re.search(rf"(?<![\w]){re.escape(diagram_type)}(?![\w])", system_text), (
            f"{name} 缺少白名单图类型 {diagram_type}"
        )
    # 大小写敏感抽查：错误大小写变体不得出现在提示词里
    for wrong in ("sequencediagram", "classdiagram", "Flowchart"):
        assert wrong not in system_text, f"{name} 含错误大小写的 {wrong}"
