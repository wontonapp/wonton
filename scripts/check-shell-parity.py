#!/usr/bin/env python3
"""两个壳的对齐检查：核心经 UniFFI 导出的 ViewModel 项，iOS 和 Android 是不是都用到了。

“项”的定义见 scripts/shell_parity/items.py。只有一个壳用到的项报 GitHub 警告注解，不让 CI 失败：
一边先做、另一边跟上是常事，警告提醒别忘了。有意只在一边的写进仓库根目录的 .shell-parity-allowlist。
两边都没用到的只计数（可能是给测试或以后用的）。所属类型只有一个壳走得到的项不再单报，缺口报在上层；
用到的那边看起来是同名的别的东西的（见 items.usage）也只计数。

按名字在壳的源码里找（去掉注释和 import，不看测试和 Debug 代码）。同名的别的东西会让一项看起来“用到了”，
所以这个检查偏向漏报；它不知道的用法（比如经字符串取字段）会误报。

退出码：读不懂 Rust 源码时为 1（导出的写法变了，要改解析）；警告多少都为 0。
--list both|iOS|Android|nested|doubtful|neither 打印对应的项，调试定义时用。
"""

from __future__ import annotations

import argparse
import os
import sys
from typing import Dict, List

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from shell_parity.allowlist import ALLOWLIST_NAME, Entry, parse_allowlist  # noqa: E402
from shell_parity.items import Item, Usage, collect_items, usage  # noqa: E402
from shell_parity.rust import Exports, read_exports  # noqa: E402
from shell_parity.shells import read_shells  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRATES = ["crates/wonton-protocol/src", "crates/wonton-client/src"]
SHELLS = ("iOS", "Android")
BUCKETS = ("both", "iOS", "Android", "nested", "doubtful", "neither")


def bucket(item: Item, result: Usage) -> str:
    used = result.shells[item.key]
    if len(used) == len(SHELLS):
        return "both"
    if not used:
        return "neither"
    if item.key in result.nested:
        return "nested"
    return "doubtful" if item.key in result.doubtful else next(iter(used))


def read_allowlist(root: str) -> List[Entry]:
    path = os.path.join(root, ALLOWLIST_NAME)
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as handle:
        entries, problems = parse_allowlist(handle.read())
    for line, problem in problems:
        print(f"::warning file={ALLOWLIST_NAME},line={line}::{problem}")
    return entries


def check_allowlist(entries: List[Entry], exports: Exports) -> None:
    """白名单里已经不存在的项报警告。两边都用到了的不报：Swift 的枚举分支按名字找，同名的会让它看起来用到了。"""
    keys = {item.key for item in collect_items(exports, set())} | set(exports.types)
    for entry in entries:
        if entry.key not in keys:
            print(f"::warning file={ALLOWLIST_NAME},line={entry.line}::{entry.key} 已经不是导出的 ViewModel 项，删掉这一行")


def warn(item: Item, shell: str) -> str:
    other = "Android" if shell == "iOS" else "iOS"
    spelling = " 或 ".join(item.kotlin if other == "Android" else item.swift)
    return (
        f"::warning file={item.path},line={item.line},title=只有 {shell} 用到::"
        f"{item.key} 只有 {shell} 用到，{other} 的壳里没有找到 {spelling}。"
        f"有意只在一边的，写进 {ALLOWLIST_NAME} 并写明原因"
    )


def write_summary(counts: Dict[str, int], lines: List[str], doubtful: List[str]) -> None:
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write("### 两个壳的对齐\n\n| 两边都用 | 只有 iOS | 只有 Android | 上层已报 | 可能同名 | 都没用 |\n|---|---|---|---|---|---|\n")
        handle.write("| " + " | ".join(str(counts[name]) for name in BUCKETS) + " |\n\n")
        # 一个步骤的警告注解在页面上只显示前几条，完整的列表放在这里。
        handle.write("".join(f"- {line}\n" for line in lines))
        if doubtful:
            handle.write("\n<details><summary>只有一边找到、但可能是同名的别的东西，没报警告</summary>\n\n")
            handle.write("".join(f"- `{key}`\n" for key in doubtful))
            handle.write("\n</details>\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--root", default=ROOT)
    parser.add_argument("--list", choices=BUCKETS)
    args = parser.parse_args()

    exports = read_exports(args.root, CRATES)
    if exports.unparsed:
        print("这些 uniffi 的 derive 后面没认出类型声明，先改 scripts/shell_parity/rust.py：", file=sys.stderr)
        print("\n".join(exports.unparsed), file=sys.stderr)
        return 1
    if "Action" not in exports.types or not any(name.endswith("ViewModel") for name in exports.types):
        print("没读到 Action 或任何 ViewModel：导出的写法变了，先改 scripts/shell_parity/rust.py", file=sys.stderr)
        return 1
    entries = read_allowlist(args.root)
    check_allowlist(entries, exports)
    allowed = {entry.key for entry in entries}
    items = [item for item in collect_items(exports, allowed) if item.key not in allowed]
    result = usage(exports, allowed, items, read_shells(args.root))

    counts = {name: 0 for name in BUCKETS}
    summary: List[str] = []
    doubtful: List[str] = []
    for item in items:
        where = bucket(item, result)
        counts[where] += 1
        if where == "doubtful":
            doubtful.append(item.key)
        if args.list == where:
            print(f"{item.kind:8} {item.key}  ({item.path}:{item.line})")
        if where in SHELLS and not args.list:
            print(warn(item, where))
            summary.append(f"`{item.key}` 只有 {where} 用到（{item.path}:{item.line}）")
    print(
        f"共 {len(items)} 项：两边都用 {counts['both']}，只有 iOS {counts['iOS']}，只有 Android {counts['Android']}，"
        f"上层已报 {counts['nested']}，可能同名 {counts['doubtful']}，都没用 {counts['neither']}"
    )
    write_summary(counts, summary, doubtful)
    return 0


if __name__ == "__main__":
    sys.exit(main())
