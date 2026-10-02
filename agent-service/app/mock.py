"""离线演示模式（PRAXIS_ALLOW_MOCK_LLM=true）使用的确定性计划生成器。

用途：没有 API Key 时也能跑通"前端 -> Tauri -> FastAPI -> LangGraph"全链路，
便于开发调试 UI 与通信协议。生产使用请关闭该开关。
"""

from __future__ import annotations

from typing import Any


def _to_int(value: Any, default: int) -> int:
    """安全转 int：非法输入回退默认值（mock 输入本已经 Pydantic 校验，此处纯防御）。"""
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def build_mock_plan(request: dict[str, Any]) -> dict[str, Any]:
    """根据请求生成一份确定性的 mock 计划（camelCase dict，与 StudyPlan schema 对齐）。

    参数 request 是 PlanRequest.model_dump(by_alias=True) 的结果。
    """
    goal: str = request.get("goal", "你的学习目标")
    weeks: int = _to_int(request.get("weeks"), 12)
    hours: int = _to_int(request.get("weeklyHours"), 10)
    interests: list[str] = request.get("interests") or []
    background: str = request.get("background") or ""

    # 三阶段均分周期：基础 -> 进阶 -> 综合项目
    seg = max(1, weeks // 3)
    ranges = [
        f"第 1-{seg} 周",
        f"第 {seg + 1}-{seg * 2} 周",
        f"第 {seg * 2 + 1}-{weeks} 周",
    ]
    focus = f"（偏好：{'、'.join(interests)}）" if interests else ""

    def phase(name: str, week: str, stage: str) -> dict[str, Any]:
        return {
            "name": name,
            "weekRange": week,
            "objectives": [
                f"完成{stage}阶段的核心概念学习",
                "产出一批可复习的练习成果",
            ],
            "theory": [f"{stage}核心概念", "推荐阅读与视频资料"],
            "practice": [f"每章配套的{stage}动手练习", "整理个人知识笔记"],
            "assignments": [f"{stage}自测题", "小任务：独立完成一个迷你练习"],
            "project": {
                "name": f"{stage}收尾小项目",
                "description": f"围绕「{goal}」完成一个小型综合练习{focus}",
                "acceptanceCriteria": [
                    "项目可运行并附简短说明文档",
                    "能口头复述项目中用到的关键概念",
                ],
            },
        }

    return {
        "title": f"「{goal}」学习计划（Mock 演示版）",
        "overview": (
            f"这是一份由离线 Mock 模式生成的演示计划（{weeks} 周 × 每周 {hours} 小时，"
            f"总学时约 {weeks * hours} 小时）{focus}。"
            + (f"已考虑你的基础：{background[:40]}。" if background else "")
            + " 配置真实 API Key 后重新生成，可获得由 LLM 定制的计划。"
        ),
        "estimatedTotalHours": weeks * hours,
        "phases": [
            phase("第一阶段 · 打基础", ranges[0], "基础"),
            phase("第二阶段 · 核心进阶", ranges[1], "进阶"),
            phase("第三阶段 · 综合实战", ranges[2], "实战"),
        ],
        "resources": [
            {"title": "官方入门文档", "type": "docs", "note": "最权威的第一手资料"},
            {"title": "经典入门书籍", "type": "book", "note": "系统建立知识框架"},
            {
                "title": "视频实战课",
                "type": "video",
                "note": "跟着做一遍，降低上手门槛",
            },
        ],
        "reviewNotes": "（Mock 模式：未执行真实 LLM 评审）",
    }
