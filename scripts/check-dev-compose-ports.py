#!/usr/bin/env python3
"""开发栈端口门禁：deploy/dev/ 下 compose 的 ports 必须把宿主地址绑在 127.0.0.1（或 [::1]）。

规则和例外见 scripts/dev_ports/ports.py。CI 的结构门禁和 scripts/check.sh 都跑这个。
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    from dev_ports.ports import EXCEPTIONS, check_repo
except ImportError as e:
    print(f"端口检查跑不了：{e}", file=sys.stderr)
    sys.exit(2)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main() -> int:
    violations = check_repo(ROOT, EXCEPTIONS)
    if not violations:
        print("开发栈端口检查通过")
        return 0
    print(f"开发栈端口检查不通过（{len(violations)} 处）：宿主地址只许 127.0.0.1 / [::1]", file=sys.stderr)
    for v in violations:
        print(f"  {v.describe()}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
