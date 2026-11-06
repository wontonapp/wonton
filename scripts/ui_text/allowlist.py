"""界面文字例外白名单 .ui-text-allowlist 的解析和检查，格式和 .size-allowlist 一样：路径 | 原因 | 到期日。

有效的例外让文件的问题不报错；无效的（格式、日期、过期、路径不存在、文件已经没有问题）本身报错，
而且不豁免文件，所以文件里的问题照常报出来。
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from datetime import date
from typing import List, Set, Tuple

ALLOWLIST_NAME = ".ui-text-allowlist"
DATE_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


@dataclass
class Entry:
    path: str
    line: int
    expires: str


def parse_allowlist(text: str) -> Tuple[List[Entry], List[Tuple[int, str]]]:
    entries: List[Entry] = []
    problems: List[Tuple[int, str]] = []
    for number, raw in enumerate(text.split("\n"), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [part.strip() for part in line.split("|")]
        if len(parts) != 3 or not parts[0]:
            problems.append((number, "格式应为 路径 | 原因 | 到期日 YYYY-MM-DD"))
            continue
        entries.append(Entry(parts[0], number, parts[2]))
    return entries, problems


def check_entries(
    entries: List[Entry], root: str, today: str, violating: Set[str]
) -> Tuple[Set[str], List[Tuple[int, str]]]:
    """返回 (有效豁免的路径, 白名单本身的问题)。today 是 YYYY-MM-DD。"""
    exempt: Set[str] = set()
    problems: List[Tuple[int, str]] = []
    seen: Set[str] = set()
    for entry in entries:
        problem = _entry_problem(entry, root, today, violating, seen)
        if problem is None:
            exempt.add(entry.path)
        else:
            problems.append((entry.line, problem))
        seen.add(entry.path)
    return exempt, problems


def _entry_problem(entry: Entry, root: str, today: str, violating: Set[str], seen: Set[str]):
    """这一条有问题返回说明，没问题返回 None。"""
    if entry.path in seen:
        return f"「{entry.path}」在白名单里重复出现"
    if not os.path.isfile(os.path.join(root, entry.path)):
        return f"路径不存在：{entry.path}"
    if not DATE_RE.fullmatch(entry.expires):
        return "到期日缺失或格式不对，应为 YYYY-MM-DD"
    try:
        date.fromisoformat(entry.expires)
    except ValueError:
        return f"到期日不是合法日期：{entry.expires}"
    if entry.expires < today:
        return f"例外已过期（到期 {entry.expires}），请抽取写死的文字后删掉这一行，或者重新评估"
    if entry.path not in violating:
        return "已经没有写死的文字了，从白名单里删掉这一行"
    return None
