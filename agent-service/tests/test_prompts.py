"""prompts 渲染 smoke：outline / chapter 等提示词全占位符渲染成功。

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
# phase_extras 为 wrap 轻量收尾。
# 注：phase.yaml（整阶段一次生成）已随「逐章扇出」与「regen 并入统一图」下线，
# 它的唯一消费者 _expand_phase_draft 已删除。
_STORY_PROMPTS = ("outline", "phase_extras", "chapter")

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
    "chapter_kind": "concept",
    "kind_requirements": (
        "讲透原理：机制如何运转、为何这样设计；必须有 800 字以上的「### 深入：××」小节"
    ),
    "plan_summary": "计划标题：X\n第 1 阶段「打基础」",
    "validation_issues": "- 阶段 1 学时偏差 25%",
    "chapter_knowledge_points": "- 什么是事件循环；宏任务与微任务",
    "tech_facts": "- serde（crate）当前版本 1.0.215",
    "research_snippets": "（未启用联网检索，按你的知识范围组织内容）",
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


@pytest.mark.parametrize("name", ("chapter",))
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


# 每份提示词期望的变量集（= system ∪ human 的占位符全集）。
# 这是「模板侧」的声明；「注入侧」由 test_nodes 的变量集用例锁定，
# 两端同时校验才能抓到「模板要了没给」与「给了模板没用」两类漂移。
_PROMPT_VARIABLES: dict[str, set[str]] = {
    "outline": {"goal", "level", "weekly_hours", "weeks", "interests", "background"},
    "phase_extras": {
        "goal",
        "level",
        "weekly_hours",
        "weeks",
        "interests",
        "background",
        "phase_outline",
        "plan_context",
        "review_notes",
    },
    "chapter": {
        "goal",
        "level",
        "weekly_hours",
        "weeks",
        "interests",
        "background",
        "phase_name",
        "phase_week_range",
        "phase_objectives",
        "chapter_title",
        "sibling_chapter_titles",
        "project_brief",
        "plan_context",
        "review_notes",
        "chapter_kind",
        "kind_requirements",
        "chapter_knowledge_points",
        "tech_facts",
        "research_snippets",
    },
    "review": {"plan_summary", "validation_issues"},
}


@pytest.mark.parametrize("name", sorted(_PROMPT_VARIABLES))
def test_prompt_variable_set_is_exact(name):
    """模板占位符集必须与声明完全一致：多一个或少一个都失败。

    这是反向校验（此前只有「模板要的变量都在全量字典里」这一个方向），
    能抓住「提示词删了某段但注入代码还在给词」这类静默漂移。
    """
    template = prompts.load_prompt(name)
    actual = prompts.template_variables(
        template.get("system") or ""
    ) | prompts.template_variables(template.get("human") or "")
    expected = _PROMPT_VARIABLES[name]
    assert actual == expected, (
        f"{name}.yaml 变量集与声明不一致："
        f"模板多出 {sorted(actual - expected)}，模板缺少 {sorted(expected - actual)}"
    )


@pytest.mark.parametrize("section", ["system", "human"])
@pytest.mark.parametrize("name", sorted(_PROMPT_VARIABLES))
def test_render_with_exact_variables_emits_no_warning(name, section, caplog):
    """用该 section 自己的变量集渲染不得产生双向契约告警。

    注意 system 与 human 的变量集不同（如 phase 的 system 只要学时/周数，
    human 还要 level 与上下文），必须分别按各自的占位符集合渲染——用并集
    渲染任一侧都会把另一侧的变量误判成「注入未使用」。
    """
    template = prompts.load_prompt(name)
    text = template.get(section) or ""
    if not text:
        pytest.skip(f"{name}.yaml 无 {section} 段")
    needed = prompts.template_variables(text)
    variables = {key: _FULL_VARIABLES[key] for key in needed}
    with caplog.at_level("WARNING", logger="app.prompts"):
        prompts.render(text, **variables)
    assert not caplog.messages, f"{name}.{section} 渲染出现契约告警：{caplog.messages}"


def test_render_warns_on_missing_and_unused_variables(caplog):
    """两个方向都要有告警：缺失（按空串渲染）与多余（注入未使用）。"""
    with caplog.at_level("WARNING", logger="app.prompts"):
        rendered = prompts.render("你好 {{ name }}，{{ missing }}", name="世界", extra="多余")
    assert rendered.startswith("你好 世界，")
    text = " ".join(caplog.messages)
    assert "missing" in text, "缺失变量应告警"
    assert "extra" in text, "未使用的注入变量应告警"
