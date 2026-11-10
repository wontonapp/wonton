#!/usr/bin/env python3
"""界面文字检查（ADR-0043，防止再写死）。

检查三件事：源码字符串里的中文、日文和全角字符（规则一）；Swift、Kotlin 里直接传给界面接口（系统控件、设计系统组件、
文字类参数）的写死文字和 TabText.choose、tabText 的调用（规则二）；
四种语言的字符串目录是否完整（规则三）。例外写在仓库根目录的 .ui-text-allowlist。

--print-violating 列出现在有问题的文件（忽略白名单），用来生成白名单的初始内容。
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone
from typing import Dict, List, Set

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from ui_text import catalogs, components, source  # noqa: E402
from ui_text.allowlist import ALLOWLIST_NAME, check_entries, parse_allowlist  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ANDROID_EN_SUFFIX = "/res/values/strings.xml"


def list_tracked_files(root: str) -> List[str]:
    out = subprocess.run(["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True).stdout
    # 删掉但还没提交的文件还留在索引里；它在工作区已经不存在，读不到。
    return [p for p in out.decode("utf-8").split("\0") if p and os.path.isfile(os.path.join(root, p))]


def read(root: str, path: str) -> str:
    with open(os.path.join(root, path), encoding="utf-8") as f:
        return f.read()


def collect_problems(root: str, files: List[str]) -> Dict[str, List[source.Problem]]:
    problems: Dict[str, List[source.Problem]] = defaultdict(list)
    design = components.find_components(files, lambda path: read(root, path))
    for path in files:
        lang = source.source_lang(path)
        if lang is not None:
            problems[path] += source.scan_source(read(root, path), lang, design.get(lang, ()))
        if path.startswith("apps/ios/") and path.endswith(".xcstrings"):
            problems[path] += catalogs.check_xcstrings(read(root, path))
    for path, found in _android_problems(root, files).items():
        problems[path] += found
    return problems


def _android_problems(root: str, files: List[str]) -> Dict[str, List[source.Problem]]:
    """每个带 values/strings.xml 的 res 目录检查一次；问题记在对应语言的文件上。"""
    tracked: Set[str] = set(files)
    result: Dict[str, List[source.Problem]] = {}
    for en_path in files:
        if not (en_path.startswith("apps/compose/") and en_path.endswith(ANDROID_EN_SUFFIX)):
            continue
        res_dir = en_path[: -len("values/strings.xml")]
        paths = {"en": en_path}
        others: Dict[str, object] = {}
        for lang, dirname in catalogs.ANDROID_DIRS.items():
            if lang == "en":
                continue
            rel = res_dir + dirname + "/strings.xml"
            paths[lang] = rel
            others[lang] = read(root, rel) if rel in tracked else None
        per_lang = catalogs.android_problems(read(root, en_path), others)
        for lang, found in per_lang.items():
            result.setdefault(paths[lang], []).extend(found)
    return result


def violating_paths(problems: Dict[str, List[source.Problem]]) -> Set[str]:
    return {path for path, found in problems.items() if found}


def report_lines(root: str, problems: Dict[str, List[source.Problem]], today: str) -> List[str]:
    violating = violating_paths(problems)
    allowlist_file = os.path.join(root, ALLOWLIST_NAME)
    text = read(root, ALLOWLIST_NAME) if os.path.isfile(allowlist_file) else ""
    entries, format_problems = parse_allowlist(text)
    # 暂停检查的文件（source.CHECK_RUST）当作还有问题，条目留着，恢复检查时照常生效。
    violating |= {entry.path for entry in entries if source.paused(entry.path)}
    exempt, entry_problems = check_entries(entries, root, today, violating)
    found = [(path, line, msg) for path, items in problems.items() if path not in exempt for line, msg in items]
    found += [(ALLOWLIST_NAME, line, msg) for line, msg in format_problems + entry_problems]
    return [f"{path}:{line}: {msg}" for path, line, msg in sorted(found)]


def main() -> int:
    parser = argparse.ArgumentParser(description="界面文字检查（ADR-0043）")
    parser.add_argument("--print-violating", action="store_true", help="列出有问题的文件，忽略白名单")
    args = parser.parse_args()
    problems = collect_problems(ROOT, list_tracked_files(ROOT))
    if args.print_violating:
        print("\n".join(sorted(violating_paths(problems))))
        return 0
    today = datetime.now(timezone.utc).date().isoformat()
    lines = report_lines(ROOT, problems, today)
    for line in lines:
        print(line, file=sys.stderr)
    if lines:
        return 1
    print("界面文字检查通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
