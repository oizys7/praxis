"""校验 agent-service/prompts/*.yaml 均可被安全解析。

由 scripts/check.mjs 以仓库根目录为 cwd 调用：
    python scripts/check_prompts.py

退出码：0 = 全部通过；1 = 存在缺失 / 解析失败 / 结构不合法。
"""

from __future__ import annotations

import glob
import sys

import yaml


def main() -> int:
    files = sorted(glob.glob("agent-service/prompts/*.yaml"))
    if not files:
        print("FAIL: 未找到任何提示词 YAML 文件（agent-service/prompts/）")
        return 1

    for path in files:
        try:
            # prompts.py 在运行时以 UTF-8 读取，这里保持一致
            with open(path, encoding="utf-8") as f:
                data = yaml.safe_load(f)
        except (OSError, yaml.YAMLError) as exc:
            print(f"FAIL: {path}: {exc}")
            return 1
        if not isinstance(data, dict):
            print(f"FAIL: {path} 根节点必须是映射（dict）")
            return 1
        print(f"OK   {path}")

    print(f"已校验 {len(files)} 个 YAML 文件")
    return 0


if __name__ == "__main__":
    sys.exit(main())
