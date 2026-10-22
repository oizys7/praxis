"""提示词加载与模板渲染。

所有 LLM 提示词文案统一存放在 agent-service/prompts/*.yaml，
代码中不硬编码任何提示词，便于独立调优与版本管理。

YAML 中的变量占位符使用 `{{ var }}` 语法（选择双花括号是为了避免与
提示词中常见的 JSON 花括号示例冲突）。
"""

from __future__ import annotations

import logging
import re
from functools import lru_cache
from pathlib import Path
from typing import Any

import yaml

# prompts/ 目录位于 agent-service/prompts
PROMPTS_DIR = Path(__file__).resolve().parent.parent / "prompts"

# {{ var }} 占位符（允许花括号内两侧有空白）
_VAR_PATTERN = re.compile(r"\{\{\s*(\w+)\s*\}\}")

logger = logging.getLogger(__name__)


@lru_cache(maxsize=16)
def load_prompt(name: str) -> dict[str, Any]:
    """按名称加载 prompts/<name>.yaml，返回整个 YAML dict（带缓存）。

    raise FileNotFoundError / ValueError：文件缺失或格式不合法时抛出。
    """
    path = PROMPTS_DIR / f"{name}.yaml"
    if not path.exists():
        raise FileNotFoundError(f"提示词文件不存在：{path}")
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise TypeError(f"提示词文件 {path} 的根节点必须是映射（dict）")
    return data


def template_variables(template: str) -> set[str]:
    """模板里出现的全部占位符名（去重）。"""
    return {match.group(1) for match in _VAR_PATTERN.finditer(template)}


def render(template: str, **variables: Any) -> str:
    """渲染模板：把 {{ var }} 替换为对应变量值。

    双向契约（2026-10-22 架构收敛）：此前只保证「模板要了但没给 → 渲染空串」，
    属静默失败——`_expand_phase_extras` 漏注 goal/level/interests/background
    四个学习者画像字段很久无人发现，而练习与项目恰恰最依赖这些信息（模型只能
    按通用口味设计）。现在两个方向都记录告警：

    - 模板要了但没给：仍按空串渲染（保持链路可用），warn 出变量名；
    - 给了但模板没用：同样 warn——通常意味着注入代码与提示词已经脱节，
      多注入的变量不会报错也不会生效，是最难发现的一类漂移。

    生产不抛错（改个模板名不该让生成链路崩），契约由测试双向锁定：
    test_prompts 锁「模板 == 声明」，test_nodes 锁「注入 == 模板」。
    """
    required = template_variables(template)
    provided = set(variables)
    missing = required - provided
    unused = provided - required
    if missing:
        logger.warning("提示词变量缺失，已按空串渲染：%s", "、".join(sorted(missing)))
    if unused:
        logger.warning("注入的变量未被提示词使用：%s", "、".join(sorted(unused)))
    return _VAR_PATTERN.sub(lambda m: str(variables.get(m.group(1), "")), template)


def fallback_questions(name: str) -> dict[str, str]:
    """读取 YAML 中的 fallback_questions 映射（问题代码 -> 降级文案）。

    当 LLM 不可用（未配置 Key / 调用失败 / mock 模式）时，
    clarify 节点使用这些内置文案作为澄清问题的降级来源。
    """
    return load_prompt(name).get("fallback_questions") or {}
