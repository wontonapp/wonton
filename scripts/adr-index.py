#!/usr/bin/env python3
"""按编号列出 docs/adr 下的各篇（编号、主题、文件），主题取各篇的一级标题。

ADR 的目录不再手写成表：几个 PR 同时往表尾加一行，回回冲突在同一处。编号和主题只写在各篇的文件名和一级标题里，
要看全表时运行这个脚本。--check 检查约定：文件名 NNNN-xxx.md，一级标题“# ADR-NNNN ……”，编号和文件名一致；
同一个编号的第二篇只能是验证结果（标题“# ADR-NNNN 的验证”）。"""
import os
import re
import sys

ADR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "docs", "adr")
NAME = re.compile(r"^(\d{4})-[a-z0-9-]+\.md$")


def entries(root=ADR):
    """[(编号, 文件名, 一级标题)]，按文件名排序；不是 NNNN-xxx.md 的文件（README 等）不算。"""
    out = []
    for name in sorted(os.listdir(root)):
        m = NAME.match(name)
        if not m:
            continue
        with open(os.path.join(root, name), encoding="utf-8") as f:
            out.append((m.group(1), name, f.readline().rstrip("\n")))
    return out


def problems(items):
    out, seen = [], {}
    for num, name, title in items:
        if not title.startswith(f"# ADR-{num} "):
            out.append(f"{name}：一级标题要以“# ADR-{num} ”开头，现在是“{title}”")
        if num in seen and title != f"# ADR-{num} 的验证":
            out.append(f"{name}：编号 {num} 已被 {seen[num]} 用了，换下一个空号")
        seen.setdefault(num, name)
    return out


def main(argv):
    items = entries()
    if "--check" in argv:
        bad = problems(items)
        for line in bad:
            print(f"::error::{line}")
        return 1 if bad else 0
    print("| 编号 | 主题 |\n| --- | --- |")
    for num, name, title in items:
        print(f"| [{num}]({name}) | {title.removeprefix(f'# ADR-{num} ')} |")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
