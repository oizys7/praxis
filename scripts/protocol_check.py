"""三端 wire 契约一致性校验（契约优先改造，2026-10-22）。

真源是 protocol/protocol.json；本脚本把「前端 TS 接口」与「Python Pydantic 模型」
的字段名与真源比对，两个方向都查：

- 缺字段：真源声明了但该端没有 → 消费方会拿到 undefined / 缺省值（最典型
  的漂移：新增字段时漏改一端）；
- 多字段：该端有但真源没声明，且不在 extra 白名单里 → 说明有人绕开契约
  偷偷加字段，下一代消费者无从预期。

Rust 侧不在此脚本内：protocol.rs 用 serde 类型 + rename_all 表达，配合
wire 序列化测试比正则解析可靠得多，强行生成会牺牲现有的精细注解。

用法（由 npm run check 调用）：
    uv run --project agent-service python scripts/protocol_check.py
"""

from __future__ import annotations

import importlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = ROOT / "protocol" / "protocol.json"
TS_PATH = ROOT / "frontend" / "src" / "types" / "agent.ts"

# 真源类型名 -> Python 模型名（两端的命名差异集中在这里，避免散落）
PYTHON_MODELS = {
    "PlanRequest": "PlanRequest",
    "Chapter": "Chapter",
    "StudyPhase": "StudyPhase",
    "PhaseProject": "PhaseProject",
    "StudyResource": "Resource",
    "GlossaryEntry": "GlossaryEntry",
    "StudyPlan": "StudyPlan",
}


def ts_interface_fields(source: str, name: str) -> set[str] | None:
    """从 agent.ts 里抽出一个 interface 的字段名集合（去掉注释）。"""
    match = re.search(rf"export interface {name} \{{(.*?)\n\}}", source, re.DOTALL)
    if not match:
        return None
    body = re.sub(r"/\*.*?\*/", "", match.group(1), flags=re.DOTALL)
    body = re.sub(r"//[^\n]*", "", body)
    return set(re.findall(r"^\s*(\w+)\??:", body, re.MULTILINE))


def python_model_fields() -> dict[str, set[str]]:
    """反射 Pydantic 模型的 wire 字段名（alias 优先，与 JSON 上的名字一致）。"""
    sys.path.insert(0, str(ROOT / "agent-service"))
    # 用 importlib 而非顶层 import：app 包只有在 sys.path 注入后才可见，写成
    # 顶层 import 会让静态分析器在扫描本脚本时报「无法解析 app」。
    schemas = importlib.import_module("app.schemas")

    result: dict[str, set[str]] = {}
    for wire_name, model_name in PYTHON_MODELS.items():
        model = getattr(schemas, model_name)
        result[wire_name] = {
            (field.alias or field_name) for field_name, field in model.model_fields.items()
        }
    return result


def diff(
    label: str, expected: set[str], actual: set[str], allowed_extra: set[str]
) -> list[str]:
    problems: list[str] = []
    missing = expected - actual
    if missing:
        problems.append(f"{label} 缺少字段：{sorted(missing)}")
    undeclared = actual - expected - allowed_extra
    if undeclared:
        problems.append(
            f"{label} 出现真源未声明的字段：{sorted(undeclared)}"
            f"（若是端特有字段，请写进该类型的 extra）"
        )
    return problems


def main() -> int:
    try:
        schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
        ts_source = TS_PATH.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"✗ 读取契约真源或前端类型文件失败：{exc}")
        return 1
    except json.JSONDecodeError as exc:
        print(f"✗ 契约真源不是合法 JSON：{exc}")
        return 1
    types = schema["types"]
    py_fields = python_model_fields()

    problems: list[str] = []
    for name, spec in types.items():
        expected = set(spec["fields"])
        allowed_extra = set(spec.get("extra") or [])

        ts_actual = ts_interface_fields(ts_source, name)
        if ts_actual is None:
            problems.append(f"TS 缺少 export interface {name}")
        else:
            problems.extend(diff(f"TS {name}", expected, ts_actual, allowed_extra))

        py_actual = py_fields.get(name)
        if py_actual is None:
            problems.append(f"Python 缺少模型 {PYTHON_MODELS.get(name, name)}")
        else:
            problems.extend(diff(f"Python {name}", expected, py_actual, allowed_extra))

    if problems:
        print("✗ 三端 wire 契约不一致：")
        for problem in problems:
            print(f"  - {problem}")
        print("\n真源：protocol/protocol.json")
        return 1

    checked = len(types)
    print(f"✓ 三端 wire 契约一致（{checked} 个类型 × 2 端）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
