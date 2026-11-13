"""壳的源码和 UniFFI 的命名规则：Rust 的名字在 Swift、Kotlin 里写成什么，壳里有没有写到。

命名照 uniffi 0.32 的生成器（底下是 heck）：字段、函数和 Swift 的枚举分支用 lowerCamelCase。Kotlin 里带数据
的枚举生成 sealed class，分支保持 UpperCamelCase，壳里总是写成 `枚举名.分支名`（不按分支导入），按这个限定名找；
光找分支名会撞上字符串和别的常量（"TCP"）。不带数据的枚举怎么算见 items.py。
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from typing import Dict, List, Optional, Set

# heck 的分词：小写到大写、连续大写到“大写加小写”处断开，数字跟着前面的字母不断开。
WORD_RE = re.compile(r"[A-Z]+[0-9]*(?=[A-Z][a-z])|[A-Z]?[a-z0-9]+|[A-Z]+[0-9]*|[0-9]+")
IDENT_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
# 插值里的表达式是真用到；`://` 是网址，不是注释。
LINE_COMMENT_RE = re.compile(r"(?<!:)//[^\n]*")
# 普通字符串字面量要去掉：里面的 .名字 会撞上成员匹配（"app.wonton.network" 的 .wonton 撞 ServerKind.wonton）；
# 插值表达式留下（INTERP_KEEP_RE），那才是真用到（#503 核对）。
STRING_RE = re.compile(r'"(?:[^"\\\n]|\\.)*"')
INTERP_KEEP_RE = re.compile(r"\\\(([^)]*)\)|\$\{([^}]*)\}")
MEMBER_RE = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)")
IMPORT_RE = re.compile(r"^\s*(?:import|package)\b[^\n]*", re.M)
QUALIFIED_RE = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)")

# 测试、UI 测试和只在 Debug 构建里的代码不算壳用到：它们引用了，用户在界面上也看不到。
SKIPPED_PARTS = {"WontonTests", "WontonUITests", "Generated", "test", "androidTest", "androidHostTest", "debug", "build"}


def words(name: str) -> List[str]:
    out: List[str] = []
    for chunk in name.split("_"):
        out.extend(WORD_RE.findall(chunk))
    return [word for word in out if word]


def lower_camel(name: str) -> str:
    parts = words(name)
    if not parts:
        return name
    return parts[0].lower() + "".join(part[:1].upper() + part[1:].lower() for part in parts[1:])


@dataclass
class Source:
    """一个源文件里出现的名字。"""

    tokens: Set[str]  # 标识符，不含 import 行
    members: Set[str]  # `.名字` 里的名字
    qualified: Set[str]  # `类型.名字`
    context: Set[str]  # 标识符，含 import 行：文件提到了哪些类型


@dataclass
class Shell:
    name: str
    files: List[Source]

    def finds(self, pattern: str, context: Optional[Set[str]] = None) -> bool:
        """有没有一个文件既有 pattern、又提到 context 里的任一名字（context 为空时不要求）。

        `.名字` 找成员，`类型.名字` 找限定名，其余找标识符。
        """
        for source in self.files:
            if pattern.startswith("."):
                hit = pattern[1:] in source.members
            elif "." in pattern:
                hit = pattern in source.qualified
            else:
                hit = pattern in source.tokens
            if hit and (not context or not context.isdisjoint(source.context | source.members)):
                return True
        return False


def _strip(text: str) -> str:
    """去掉注释：注释里写到的名字不算用到。"""
    text = BLOCK_COMMENT_RE.sub(" ", text)
    return LINE_COMMENT_RE.sub("", text).replace("`", "")


def _strip_strings(text: str) -> str:
    """去掉字符串字面量，插值里的表达式留下（Swift 的 \\(x.y)、Kotlin 的 ${x.y} 里的字段是真用到）：
    普通字符串里的 .名字 会撞上成员匹配（"app.wonton.network" 的 .wonton 撞 ServerKind 的 .wonton，#503 核对）。"""
    return STRING_RE.sub(lambda m: " " + " ".join(a or b for a, b in INTERP_KEEP_RE.findall(m.group(0))), text)


def _source_files(root: str, base: str, suffix: str) -> List[str]:
    found: List[str] = []
    for directory, dirs, names in os.walk(os.path.join(root, base)):
        dirs[:] = sorted(d for d in dirs if d not in SKIPPED_PARTS)
        found.extend(os.path.join(directory, name) for name in sorted(names) if name.endswith(suffix))
    return found


def read_source(text: str) -> Source:
    text = _strip(text)
    # import 和 package 行里的包名（androidx.compose.runtime）会撞上同名字段，只拿来看文件提到了哪些类型。
    code = IMPORT_RE.sub("", text)
    code = _strip_strings(code)
    return Source(
        set(IDENT_RE.findall(code)),
        set(MEMBER_RE.findall(code)),
        {f"{owner}.{member}" for owner, member in QUALIFIED_RE.findall(code)},
        set(IDENT_RE.findall(text)),
    )


def read_shell(root: str, name: str, base: str, suffix: str) -> Shell:
    files: List[Source] = []
    for path in _source_files(root, base, suffix):
        with open(path, encoding="utf-8") as handle:
            files.append(read_source(handle.read()))
    return Shell(name, files)


def read_shells(root: str) -> Dict[str, Shell]:
    return {
        "iOS": read_shell(root, "iOS", "apps/ios", ".swift"),
        "Android": read_shell(root, "Android", "apps/compose", ".kt"),
    }
