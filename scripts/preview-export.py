#!/usr/bin/env python3
"""只读导出真实学习计划到本机预览 JSON（开发预览专用）。

安全边界：
- 只建立只读连接 / 只读事务，仅执行 SELECT；不建表、不迁移、不写任何行。
- 连接串只从本机 config.json 读取，绝不打印、绝不写入产物。
- 输出文件只落在 frontend/preview-data.local.json（已在本机 .git/info/exclude），
  不入库、不上传。
- 默认只导出最近 3 份完整计划；列表摘要最多 50 份。

用法（在仓库根）：
    cd agent-service && uv pip install "psycopg[binary]"   # 仅本机环境，不改项目依赖
    cd .. && agent-service/.venv/Scripts/python scripts/preview-export.py
可选环境变量：
    PRAXIS_CONFIG        配置文件路径（默认仓库根 config.json）
    PRAXIS_PREVIEW_OUT   输出路径（默认 frontend/preview-data.local.json）
    PRAXIS_PREVIEW_PLANS 完整计划份数（默认 3）
"""

from __future__ import annotations

import json
import os
import sys
from datetime import date, datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG_PATH = Path(os.environ.get("PRAXIS_CONFIG", ROOT / "config.json"))
OUT_PATH = Path(
    os.environ.get("PRAXIS_PREVIEW_OUT", ROOT / "frontend" / "preview-data.local.json")
)
FULL_PLAN_LIMIT = max(1, int(os.environ.get("PRAXIS_PREVIEW_PLANS", "3")))
LIST_LIMIT = 50


def log(message: str) -> None:
    print(f"[preview-export] {message}", file=sys.stderr)


def fail(message: str) -> "NoReturn":  # type: ignore[name-defined]
    log(message)
    sys.exit(1)


def read_database_url() -> str:
    if not CONFIG_PATH.exists():
        fail(f"找不到配置文件：{CONFIG_PATH}（可设置 PRAXIS_CONFIG）")
    try:
        config = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"配置文件读取失败：{type(exc).__name__}")
    url = ((config.get("database") or {}).get("url") or "").strip()
    if not url:
        fail("config.json 未配置 database.url")
    return url


def iso(value: date | datetime | None) -> str | None:
    if value is None:
        return None
    if isinstance(value, datetime):
        if value.tzinfo is None:
            value = value.replace(tzinfo=timezone.utc)
        return value.isoformat()
    return value.isoformat()


def json_value(value):
    """psycopg 已把 jsonb 解成 Python 对象；NULL 归一为空列表/空对象。"""
    return value if value is not None else []


def load_summaries(cur) -> list[dict]:
    cur.execute(
        """
        SELECT p.id, p.title, p.goal, p.overview, p.level, p.weeks, p.favorite,
               p.created_at, p.status, p.group_id, p.study_days,
               p.learning_status, p.learning_started_at, p.study_total, p.study_done,
               COALESCE((SELECT COUNT(*) FROM chapters c JOIN phases ph ON c.phase_id = ph.id
                         WHERE ph.plan_id = p.id), 0) AS chapter_total,
               COALESCE((SELECT COUNT(*) FROM chapters c JOIN phases ph ON c.phase_id = ph.id
                         WHERE ph.plan_id = p.id AND c.status = 'complete'), 0) AS chapter_done
        FROM plans p
        ORDER BY p.favorite DESC, p.created_at DESC
        LIMIT %s
        """,
        (LIST_LIMIT,),
    )
    rows = cur.fetchall()
    summaries = []
    for row in rows:
        summaries.append(
            {
                "id": str(row["id"]),
                "title": row["title"],
                "goal": row["goal"],
                "status": row["status"],
                "overview": row["overview"],
                "level": row["level"],
                "weeks": max(0, int(row["weeks"] or 0)),
                "groupId": str(row["group_id"]) if row["group_id"] else None,
                "studyDays": list(json_value(row["study_days"])),
                "learningStatus": row["learning_status"],
                "learningStartedAt": iso(row["learning_started_at"]),
                "chapterTotal": max(0, int(row["chapter_total"] or 0)),
                "chapterDone": max(0, int(row["chapter_done"] or 0)),
                "studyTotal": max(0, int(row["study_total"] or 0)),
                "studyDone": max(0, int(row["study_done"] or 0)),
                "favorite": bool(row["favorite"]),
                "createdAt": iso(row["created_at"]),
            }
        )
    return summaries


def load_full_plan(cur, plan_id: str) -> dict | None:
    cur.execute(
        """
        SELECT id, title, goal, status, overview, level, weeks, study_days,
               favorite, created_at, estimated_total_hours
        FROM plans WHERE id = %s
        """,
        (plan_id,),
    )
    row = cur.fetchone()
    if row is None:
        return None

    cur.execute(
        """
        SELECT id, name, week_range, status, objectives, practice, assignments
        FROM phases WHERE plan_id = %s ORDER BY idx ASC
        """,
        (plan_id,),
    )
    phase_rows = cur.fetchall()

    cur.execute(
        """
        SELECT c.phase_id, c.title, c.summary, c.status, c.content_md, c.kind,
               c.key_terms, c.visualizations, c.exercises,
               EXISTS (SELECT 1 FROM study_schedule s
                       WHERE s.plan_id = p.plan_id AND s.phase_idx = p.idx
                         AND s.chapter_idx = c.idx AND s.done) AS done
        FROM chapters c JOIN phases p ON c.phase_id = p.id
        WHERE p.plan_id = %s ORDER BY p.idx ASC, c.idx ASC
        """,
        (plan_id,),
    )
    chapters_by_phase: dict[str, list[dict]] = {}
    for crow in cur.fetchall():
        chapters_by_phase.setdefault(str(crow["phase_id"]), []).append(
            {
                "title": crow["title"],
                "summary": crow["summary"],
                "contentMd": crow["content_md"],
                "kind": crow["kind"],
                "keyTerms": list(json_value(crow["key_terms"])),
                "visualizations": list(json_value(crow["visualizations"])),
                "exercises": list(json_value(crow["exercises"])),
                "done": bool(crow["done"]),
                "status": crow["status"],
            }
        )

    cur.execute(
        """
        SELECT pr.phase_id, pr.name, pr.description, pr.background, pr.steps,
               pr.acceptance_criteria, pr.estimated_hours
        FROM projects pr JOIN phases p ON pr.phase_id = p.id
        WHERE p.plan_id = %s
        """,
        (plan_id,),
    )
    project_by_phase = {
        str(prow["phase_id"]): {
            "name": prow["name"],
            "description": prow["description"],
            "background": prow["background"],
            "steps": list(json_value(prow["steps"])),
            "estimatedHours": int(prow["estimated_hours"] or 0),
            "acceptanceCriteria": list(json_value(prow["acceptance_criteria"])),
        }
        for prow in cur.fetchall()
    }

    cur.execute(
        "SELECT title, type, note, url, verification FROM resources "
        "WHERE plan_id = %s ORDER BY idx ASC",
        (plan_id,),
    )
    resources = [
        {
            "title": rrow["title"],
            "type": rrow["type"],
            "note": rrow["note"],
            "url": rrow["url"] or "",
            "verification": rrow["verification"],
        }
        for rrow in cur.fetchall()
    ]

    phases = []
    glossary: list[dict] = []
    seen_terms: set[str] = set()
    for prow in phase_rows:
        chapters = chapters_by_phase.get(str(prow["id"]), [])
        for chapter in chapters:
            for term in chapter["keyTerms"]:
                name = str(term).strip()
                if not name or name in seen_terms:
                    continue
                seen_terms.add(name)
                if len(glossary) < 60:
                    glossary.append({"term": name, "chapterTitle": chapter["title"]})
        phases.append(
            {
                "name": prow["name"],
                "weekRange": prow["week_range"],
                "objectives": list(json_value(prow["objectives"])),
                "chapters": chapters,
                "theory": [],
                "practice": list(json_value(prow["practice"])),
                "assignments": list(json_value(prow["assignments"])),
                "project": project_by_phase.get(
                    str(prow["id"]),
                    {
                        "name": "",
                        "description": "",
                        "background": "",
                        "steps": [],
                        "estimatedHours": 0,
                        "acceptanceCriteria": [],
                    },
                ),
                "status": prow["status"],
            }
        )

    return {
        "id": str(row["id"]),
        "title": row["title"],
        "goal": row["goal"],
        "status": row["status"],
        "overview": row["overview"],
        "level": row["level"],
        "weeks": max(0, int(row["weeks"] or 0)),
        "studyDays": list(json_value(row["study_days"])),
        "favorite": bool(row["favorite"]),
        "createdAt": iso(row["created_at"]),
        "plan": {
            "title": row["title"],
            "overview": row["overview"],
            "estimatedTotalHours": max(0, int(row["estimated_total_hours"] or 0)),
            "phases": phases,
            "resources": resources,
            "reviewNotes": "",
            "glossary": glossary,
        },
    }


def load_groups(cur) -> list[dict]:
    cur.execute(
        """
        SELECT g.id, g.name, COUNT(p.id) AS plan_count
        FROM plan_groups g LEFT JOIN plans p ON p.group_id = g.id
        GROUP BY g.id, g.name, g.sort_order, g.created_at
        ORDER BY g.sort_order, g.created_at
        """
    )
    return [
        {"id": str(row["id"]), "name": row["name"], "planCount": int(row["plan_count"] or 0)}
        for row in cur.fetchall()
    ]


def load_calendar(cur, today: date) -> dict:
    month_start = today.replace(day=1)
    next_month = (
        date(today.year + 1, 1, 1)
        if today.month == 12
        else date(today.year, today.month + 1, 1)
    )
    year_start = date(today.year, 1, 1)
    next_year = date(today.year + 1, 1, 1)

    cur.execute(
        """
        SELECT s.date, s.plan_id, p.title AS plan_title, s.phase_idx, s.phase_name,
               s.chapter_idx, s.chapter_title, s.done
        FROM study_schedule s JOIN plans p ON p.id = s.plan_id
        WHERE s.date >= %s AND s.date < %s
        ORDER BY s.date, s.plan_id, s.phase_idx, s.chapter_idx
        """,
        (month_start, next_month),
    )
    days: dict[str, dict] = {}
    for row in cur.fetchall():
        key = iso(row["date"])
        day = days.setdefault(key, {"date": key, "items": [], "total": 0, "doneCount": 0})
        day["items"].append(
            {
                "planId": str(row["plan_id"]),
                "planTitle": row["plan_title"],
                "phaseIdx": int(row["phase_idx"]),
                "phaseName": row["phase_name"],
                "chapterIdx": int(row["chapter_idx"]),
                "chapterTitle": row["chapter_title"],
                "done": bool(row["done"]),
            }
        )
        day["total"] += 1
        if row["done"]:
            day["doneCount"] += 1

    cur.execute(
        """
        SELECT date, COUNT(*) AS total, COUNT(*) FILTER (WHERE done) AS done
        FROM study_schedule WHERE date >= %s AND date < %s
        GROUP BY date ORDER BY date
        """,
        (year_start, next_year),
    )
    heatmap = [
        {"date": iso(row["date"]), "total": int(row["total"]), "done": int(row["done"])}
        for row in cur.fetchall()
    ]
    return {"month": list(days.values()), "year": heatmap}


def main() -> None:
    try:
        import psycopg
        from psycopg.rows import dict_row
    except ImportError:
        fail(
            "缺少 psycopg。请在本机虚拟环境安装（不改项目依赖）："
            "cd agent-service && uv pip install \"psycopg[binary]\""
        )

    url = read_database_url()
    today = date.today()

    try:
        with psycopg.connect(
            url,
            connect_timeout=8,
            options="-c statement_timeout=15000",
            row_factory=dict_row,
        ) as conn:
            conn.read_only = True
            with conn.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                read_only = cur.fetchone()["transaction_read_only"]
                if read_only not in ("on", "true"):
                    fail("当前事务未处于只读状态，已中止导出")

                summaries = load_summaries(cur)
                full_plans = {}
                for summary in summaries[:FULL_PLAN_LIMIT]:
                    record = load_full_plan(cur, summary["id"])
                    if record is not None:
                        full_plans[record["id"]] = record
                groups = load_groups(cur)
                try:
                    calendar = load_calendar(cur, today)
                except psycopg.errors.UndefinedTable:
                    conn.rollback()
                    calendar = {"month": [], "year": []}
                    log("study_schedule 表不存在，日历数据置空")
    except psycopg.OperationalError as exc:
        # 连接错误可能携带主机信息：只报告类型，不回显原始消息
        fail(f"数据库连接失败（{type(exc).__name__}），请检查本机网络与 config.json")
    except psycopg.Error as exc:
        fail(f"只读查询失败（{type(exc).__name__}）")

    payload = {
        "mode": "real-readonly",
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "plans": summaries,
        "records": full_plans,
        "groups": groups,
        "calendar": calendar,
    }
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    temporary = OUT_PATH.with_suffix(".tmp")
    temporary.write_text(
        json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    temporary.replace(OUT_PATH)
    log(
        f"已导出 {len(summaries)} 条摘要 / {len(full_plans)} 份完整计划 / "
        f"{len(groups)} 个分组 / {len(calendar['month'])} 天本月日程"
    )
    log(f"输出：{OUT_PATH}")


if __name__ == "__main__":
    main()