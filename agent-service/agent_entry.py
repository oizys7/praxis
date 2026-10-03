"""PyInstaller 打包入口：sidecar 生产模式的启动引导。

与开发模式（python -m app.main，带 --reload）的区别：
- 不带 reload（生产子进程由 Rust 桌面壳管理生命周期）；
- uvicorn.run 直接传 app 对象而非 "app.main:app" 字符串
  （PyInstaller 打包后模块布局变化，import 字符串不可靠）。

动态端口约定：Rust 壳拉起本进程前设置 AGENT_SERVICE_PORT 环境变量，
settings.port 会读取它；未设置时回退默认 8000（与开发模式一致）。
"""

import uvicorn

from app.config import settings
from app.main import app


def main() -> None:
    """以生产参数启动 FastAPI 应用。"""
    uvicorn.run(app, host=settings.host, port=settings.port, log_level="info")


if __name__ == "__main__":
    main()
