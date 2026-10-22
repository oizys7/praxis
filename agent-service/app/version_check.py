"""技术栈版本探测：从公开权威接口取「这个库现在是什么版本」。

这是整条内容链里最硬的事实锚点，一个机制同时治三件事：
- 资源真实性：版本号可验证，模型编造的库名查不到就被丢弃；
- 内容陈旧：正文能对着当前版本的 API 写，而不是训练截止时的写法；
- 幻觉过滤：查不到的条目直接丢弃，不会作为事实注入提示词。

全程 fail-open：任何失败都只是「少一条事实」，绝不阻断生成。
"""

from __future__ import annotations

import asyncio
import logging
from typing import Any

import httpx

logger = logging.getLogger(__name__)

_TIMEOUT_SECONDS = 5.0
#: 整体预算：版本探测是锦上添花，绝不该拖慢骨架
_TOTAL_BUDGET_SECONDS = 10.0
#: crates.io 要求调用方带 User-Agent
_USER_AGENT = "praxis-agent/0.1 (+https://github.com/oizys7/praxis)"

#: 包生态 → 版本接口模板
_ENDPOINTS = {
    "crate": "https://crates.io/api/v1/crates/{name}",
    "npm": "https://registry.npmjs.org/{name}/latest",
    "pypi": "https://pypi.org/pypi/{name}/json",
}
#: 仓库类走 GitHub Releases
_GITHUB_ECOSYSTEM = "github"
_SUPPORTED = {*_ENDPOINTS, _GITHUB_ECOSYSTEM}


async def detect_versions(refs: list[dict[str, Any]]) -> list[dict[str, str]]:
    """并发探测各技术栈条目的当前版本，查不到的一律丢弃。

    返回 [{name, ecosystem, latestVersion}]，顺序与输入一致（丢弃项除外）。
    丢弃是刻意的：模型编的库名一定查不到，这层就是幻觉过滤器。
    """
    targets = [
        ref
        for ref in refs
        if str(ref.get("name") or "").strip()
        and str(ref.get("ecosystem") or "").strip().lower() in _SUPPORTED
    ]
    if not targets:
        return []

    async with httpx.AsyncClient(
        timeout=_TIMEOUT_SECONDS, headers={"User-Agent": _USER_AGENT}
    ) as client:
        try:
            results = await asyncio.wait_for(
                asyncio.gather(
                    *[_latest_version(client, ref) for ref in targets],
                    return_exceptions=True,
                ),
                timeout=_TOTAL_BUDGET_SECONDS,
            )
        except (TimeoutError, asyncio.TimeoutError):
            logger.info(
                "版本探测整体超时 %.0fs，本次不注入版本事实",
                _TOTAL_BUDGET_SECONDS,
            )
            return []

    facts: list[dict[str, str]] = []
    for ref, version in zip(targets, results, strict=True):
        if not isinstance(version, str) or not version.strip():
            continue
        facts.append(
            {
                "name": str(ref["name"]).strip(),
                "ecosystem": str(ref["ecosystem"]).strip().lower(),
                "latestVersion": version.strip(),
            }
        )
    logger.info("版本探测命中 %d/%d 条", len(facts), len(targets))
    return facts


async def _latest_version(client: httpx.AsyncClient, ref: dict[str, Any]) -> str:
    """取单个条目的最新版本号；非 2xx 与缺字段都抛异常（由 gather 吞掉→丢弃）。"""
    ecosystem = str(ref.get("ecosystem") or "").strip().lower()
    name = str(ref.get("name") or "").strip()
    if ecosystem == _GITHUB_ECOSYSTEM:
        url = f"https://api.github.com/repos/{name}/releases/latest"
    else:
        url = _ENDPOINTS[ecosystem].format(name=name)

    resp = await client.get(url)
    if resp.status_code >= 400:
        raise ValueError(f"{ecosystem}:{name} 返回 {resp.status_code}")
    data = resp.json()

    if ecosystem == "crate":
        return str((data.get("crate") or {}).get("max_stable_version") or "")
    if ecosystem == "npm":
        return str(data.get("version") or "")
    if ecosystem == "pypi":
        return str((data.get("info") or {}).get("version") or "")
    return str(data.get("tag_name") or "").lstrip("v")
