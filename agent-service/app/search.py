"""外部检索的接入点与降级路径。

只做「发现层」：检索由外部 API 完成，本项目不爬、不建索引、不存语料——
技术内容时效性极强，自建全网库会变成过时知识的放大器，而覆盖度永远追不上
任意学习目标。知识库属于独立项目，这里只留调用点。

未配置提供方时 current_provider() 返回 None，生成行为与没有本模块完全一致。
"""

from __future__ import annotations

import logging
from typing import Protocol, runtime_checkable

logger = logging.getLogger(__name__)


@runtime_checkable
class SearchProvider(Protocol):
    """检索提供方：返回 [{title, url, snippet}] 形态的候选素材。"""

    async def search(
        self, query: str, *, limit: int = 3
    ) -> list[dict[str, str]]: ...


def current_provider() -> SearchProvider | None:
    """读取配置里的检索提供方；未配置或尚未接入实现时返回 None。

    本轮刻意不接真实实现：没有 Key 就无法验证，接一个不能验证的网络依赖
    只会增加失败面。接入时只改本函数，其余调用方无需改动。
    """
    from app.config import settings

    provider = (settings.search_provider or "").strip().lower()
    if not provider:
        return None
    logger.info("检索提供方配置为 %s，但实现尚未接入，按未启用处理", provider)
    return None
