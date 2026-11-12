#!/usr/bin/env python3
"""界面文字检查（ADR-0043，防止再写死）。

检查三件事：源码字符串里的中文、日文和全角字符（规则一）；Swift、Kotlin 里直接传给界面接口（系统控件、设计系统组件、
文字类参数）的写死文字和 TabText.choose、tabText 的调用（规则二）；
四种语言的字符串目录是否完整（规则三），以及每个键在不在按前缀该在的文件里、有没有在别的文件里也有一份。
例外写在仓库根目录的 .ui-text-allowlist。

--print-violating 列出现在有问题的文件（忽略白名单），用来生成白名单的初始内容。
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone
from typing import Dict, List, Set, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from ui_text import catalogs, components, paths, source  # noqa: E402
from ui_text.allowlist import ALLOWLIST_NAME, check_entries, parse_allowlist  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


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
    found: Dict[str, List[Tuple[str, List[Tuple[str, int]]]]] = {"ios": [], "android": [], "fluent": []}
    for path in files:
        lang = source.source_lang(path)
        if lang is not None:
            problems[path] += source.scan_source(read(root, path), lang, design.get(lang, ()))
        kind = paths.file_kind(path)
        catalog = path.startswith("apps/ios/") and path.endswith(".xcstrings")
        if kind is None and lang is None and not catalog:
            continue
        text = read(root, path)
        if catalog:
            problems[path] += catalogs.check_xcstrings(text)
            if kind is not None:
                found["ios"].append((path, catalogs.xcstring_keys(text)))
            continue
        if kind is None:
            continue
        keys = catalogs.android_keys(text) if kind[0] == "android" else catalogs.ftl_keys(text)
        found[kind[0]].append((path, keys))
    for platform, items in found.items():
        _merge(problems, catalogs.layout_problems(items, platform))
    _merge(problems, _android_pairs(root, files, found["android"]))
    _merge(problems, _fluent_pairs(root, found["fluent"]))
    return problems


def _merge(into: Dict[str, List[source.Problem]], extra: Dict[str, List[source.Problem]]) -> None:
    for path, found in extra.items():
        into[path] += found


def _android_pairs(root: str, files: List[str], items: List[Tuple[str, List[Tuple[str, int]]]]) -> Dict[str, List[source.Problem]]:
    """同一份文件在四种语言之间比缺键、多余键、占位符和平复数；问题记在对应语言的文件上。"""
    tracked: Set[str] = set(files)
    result: Dict[str, List[source.Problem]] = {}
    for module, stem in sorted({(_android_module(path), paths.file_kind(path)[2]) for path, _ in items}):
        en_path = paths.android_path(module, "en", stem)
        others = {
            lang: read(root, paths.android_path(module, lang, stem))
            if paths.android_path(module, lang, stem) in tracked
            else None
            for lang in paths.ANDROID_DIRS
            if lang != "en"
        }
        en_text = read(root, en_path) if en_path in tracked else ""
        for lang, found in catalogs.android_problems(en_text, others).items():
            result.setdefault(paths.android_path(module, lang, stem), []).extend(found)
    return result


def _android_module(path: str) -> str:
    return path.rsplit("/", 2)[0]


def _fluent_pairs(root: str, items: List[Tuple[str, List[Tuple[str, int]]]]) -> Dict[str, List[source.Problem]]:
    """同一份 Fluent 文件在四种语言之间比缺键和多余键。"""
    by_stem: Dict[str, Dict[str, str]] = defaultdict(dict)
    for path, _ in items:
        kind = paths.file_kind(path)
        by_stem[kind[2]][kind[1]] = read(root, path)
    result: Dict[str, List[source.Problem]] = {}
    for stem, texts in sorted(by_stem.items()):
        texts.setdefault("en", "")
        for path, found in catalogs.ftl_missing(stem, texts).items():
            result.setdefault(path, []).extend(found)
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
