"""资源可达性检查：给大纲产出的每条资源做一次真实探测。

为什么只分两档（reachable / unverified）而不给"可信度分数"：
"有没有真的可达"是客观事实，而模型自评 confidence 只会全给高分，那是假精度。

失败一律 fail-open——检查失败绝不能中断生成，最坏结果只是退回"未经核实"。
"""

from __future__ import annotations

import asyncio
import logging
from typing import Any

import httpx

logger = logging.getLogger(__name__)

#: 单次请求超时与整体预算（秒）：资源卡晚到两三秒无妨，但绝不能拖垮骨架
_HEAD_TIMEOUT_SECONDS = 3.0
_TOTAL_BUDGET_SECONDS = 8.0

REACHABLE = "reachable"
UNVERIFIED = "unverified"

#: 部分站点不支持 HEAD（405/501），这些状态码退回一次 GET 再判
_HEAD_UNSUPPORTED = {405, 501}


async def verify_resources(resources: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """并发探测每条资源的 url，回写 verification 字段并返回新列表。

    - url 为空串：不发请求，保持 unverified（模型没给就是没给，不猜）
    - 2xx/3xx：reachable
    - 其余（超时 / 连接失败 / 4xx / 5xx）：unverified，且不抛异常
    """
    checked = [dict(item) for item in resources]
    # 先一律置为未核实：空地址、探测失败、超时都不需要额外分支
    for item in checked:
        item["verification"] = UNVERIFIED
    targets = [item for item in checked if str(item.get("url") or "").strip()]
    if not targets:
        return checked

    async with httpx.AsyncClient(
        follow_redirects=True, timeout=_HEAD_TIMEOUT_SECONDS
    ) as client:
        try:
            results = await asyncio.wait_for(
                asyncio.gather(
                    *[_is_reachable(client, str(item["url"])) for item in targets],
                    return_exceptions=True,
                ),
                timeout=_TOTAL_BUDGET_SECONDS,
            )
        except (TimeoutError, asyncio.TimeoutError):
            # 整体超预算：全部退回 unverified，不阻塞生成
            logger.info("资源可达性检查整体超时 %.0fs，全部标记未核实", _TOTAL_BUDGET_SECONDS)
            return checked

    for item, ok in zip(targets, results, strict=True):
        item["verification"] = REACHABLE if ok is True else UNVERIFIED
    return checked


async def _is_reachable(client: httpx.AsyncClient, url: str) -> bool:
    """单条探测：HEAD 优先，遇到不支持的方法退回 GET。任何异常都只算不可达。"""
    try:
        resp = await client.head(url)
        if resp.status_code in _HEAD_UNSUPPORTED:
            resp = await client.get(url)
        return resp.status_code < 400
    except Exception:  # noqa: BLE001 —— 检查失败刻意吞掉，fail-open 退回未核实
        return False
