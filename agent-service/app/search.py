"""外部检索的接入点与降级路径。

只做「发现层」：检索由外部 API 完成，本项目不爬、不建索引、不存语料——
技术内容时效性极强，自建全网库会变成过时知识的放大器，而覆盖度永远追不上
任意学习目标。知识库属于独立项目，这里只留调用点。

未配置提供方时 current_provider() 返回 None，生成行为与没有本模块完全一致。
"""

from __future__ import annotations

import logging
from typing import Protocol, runtime_checkable

import httpx

logger = logging.getLogger(__name__)

_TAVILY_ENDPOINT = "https://api.tavily.com/search"
_TIMEOUT_SECONDS = 10.0
#: Tavily 单次最多返回的条数上限（服务端限制）
_MAX_RESULTS_LIMIT = 20


@runtime_checkable
class SearchProvider(Protocol):
    """检索提供方：返回 [{title, url, snippet}] 形态的候选素材。"""

    async def search(self, query: str, *, limit: int = 3) -> list[dict[str, str]]: ...


class TavilyProvider:
    """Tavily Search，只用 basic 深度。

    basic 是 1 credit/次，advanced 是 2 credits；我们要的是「可注入的素材段落」
    而不是「更全的检索」，没有理由付双倍。只取 content 片段、不取 raw_content
    全文——正文需要某篇文档的完整代码时，那是一次单独的 extract 决策。
    """

    def __init__(self, api_key: str) -> None:
        self._api_key = api_key

    async def search(self, query: str, *, limit: int = 3) -> list[dict[str, str]]:
        async with httpx.AsyncClient(timeout=_TIMEOUT_SECONDS) as client:
            response = await client.post(
                _TAVILY_ENDPOINT,
                headers={"Authorization": f"Bearer {self._api_key}"},
                json={
                    "query": query,
                    "max_results": max(1, min(limit, _MAX_RESULTS_LIMIT)),
                    "search_depth": "basic",
                },
            )
        if response.status_code >= 400:
            # 上抛而不是静默空结果：调用方（nodes._research_snippets）会记 warning
            # 并继续生成，但日志里能看出是「被限流了」而不是「确实没内容」
            raise RuntimeError(f"Tavily 检索返回 {response.status_code}")

        snippets: list[dict[str, str]] = []
        for item in response.json().get("results") or []:
            snippet = str(item.get("content") or "").strip()
            if not snippet:
                # 没有正文片段的条目对「注入素材」毫无用处，直接跳过
                continue
            snippets.append(
                {
                    "title": str(item.get("title") or "").strip(),
                    "url": str(item.get("url") or "").strip(),
                    "snippet": snippet,
                }
            )
        return snippets


def current_provider() -> SearchProvider | None:
    """按配置构造检索提供方；未配置 / 缺 key / 未知提供方一律 None。"""
    from app.config import settings

    provider = (settings.search_provider or "").strip().lower()
    if not provider:
        return None
    if provider == "tavily":
        api_key = (settings.search_api_key or "").strip()
        if not api_key:
            logger.info("search.provider=tavily 但未填 search.apiKey，按未启用处理")
            return None
        return TavilyProvider(api_key)
    logger.warning("未知的检索提供方 %s，按未启用处理", provider)
    return None
