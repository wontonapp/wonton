"""有意只在一个壳里用的项的白名单 .shell-parity-allowlist：每行 `项 | 原因`。

项写类型名（整个类型连同字段、分支都不查，也不顺着它往下走）、`类型.成员`、`对象.方法`或自由函数名。
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Tuple

ALLOWLIST_NAME = ".shell-parity-allowlist"


@dataclass
class Entry:
    key: str
    reason: str
    line: int


def parse_allowlist(text: str) -> Tuple[List[Entry], List[Tuple[int, str]]]:
    entries: List[Entry] = []
    problems: List[Tuple[int, str]] = []
    for number, raw in enumerate(text.split("\n"), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [part.strip() for part in line.split("|")]
        if len(parts) != 2 or not parts[0] or not parts[1]:
            problems.append((number, "格式应为 项 | 原因"))
            continue
        entries.append(Entry(parts[0], parts[1], number))
    return entries, problems
