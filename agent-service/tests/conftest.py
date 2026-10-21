"""pytest 全局装置：在任何应用模块导入前提供 config.json 测试沙箱。

app.config 在导入时执行冷启动 bootstrap（沿 cwd 向上按 workspace marker
查找 config.json），直接跑 pytest 时仓库根未必存在 config.json（本机配置，
gitignore 覆盖、不入库）。本装置在 conftest 导入期（早于一切测试模块）把
cwd 切到临时 workspace（含 marker 与最小 config.json），保证首个
`import app.*` 触发的模块级 bootstrap 总能成功落在这个受控 cwd 上；
个别测试再通过 monkeypatch.chdir / 显式 --config 按需覆盖查找结果。
"""

import os
import tempfile
from pathlib import Path

_SANDBOX = Path(tempfile.mkdtemp(prefix="praxis-tests-"))
# workspace marker：与 src-tauri/src/app_config.rs 约定一致
# （两形态：目录直接含 tauri.conf.json，或目录含 src-tauri/tauri.conf.json）
(_SANDBOX / "src-tauri").mkdir()
(_SANDBOX / "src-tauri" / "tauri.conf.json").write_text("{}", encoding="utf-8")
# 最小 config.json：全部缺省 → PraxisRuntimeConfig 默认值（host/port/info/空 dir/mock 关）
(_SANDBOX / "config.json").write_text("{}", encoding="utf-8")

os.chdir(_SANDBOX)
