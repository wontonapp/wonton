"""从 Rust 源码里读出经 UniFFI 导出给壳的类型和函数。

读源码而不读生成的 Swift、Kotlin 绑定：绑定要先编译整个 wonton-client（matrix-sdk 在内）才生成得出来，
不进仓库；导出的写法在源码里很统一（derive 的 uniffi::Record、uniffi::Enum、uniffi::Object，
以及 #[uniffi::export] 的 impl 和自由函数），按行读就够。
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set

DERIVE_RE = re.compile(r"uniffi::(Record|Enum|Object|Error)\b")
TYPE_DECL_RE = re.compile(r"^\s*pub\s+(struct|enum)\s+(\w+)")
FIELD_RE = re.compile(r"^\s*pub\s+(\w+)\s*:\s*(.*)$")
VARIANT_RE = re.compile(r"^\s*([A-Z]\w*)\s*(\{|\(|,|$|=)")
EXPORT_RE = re.compile(r"^\s*#\[uniffi::export(\(.*\))?\]")
IMPL_RE = re.compile(r"^\s*impl\s+(\w+)\s*\{")
FN_RE = re.compile(r"^\s*pub\s+(?:async\s+)?fn\s+(\w+)")
TYPE_NAME_RE = re.compile(r"\b([A-Z][A-Za-z0-9]*)\b")
CONSTRUCTOR_RE = re.compile(r"#\[uniffi::constructor")


@dataclass
class Member:
    name: str
    line: int
    refs: Set[str] = field(default_factory=set)
    has_data: bool = False
    catch_all: bool = False  # 带 #[serde(other)]：接住比 App 新的核心给的取值


@dataclass
class TypeDef:
    name: str
    kind: str  # record | enum | object | error
    path: str
    line: int
    members: List[Member] = field(default_factory=list)


@dataclass
class ExportedFn:
    owner: str  # 所属的对象；自由函数为空字符串
    name: str
    path: str
    line: int
    returns: Set[str]
    constructor: bool


@dataclass
class Exports:
    types: Dict[str, TypeDef]
    functions: List[ExportedFn]
    unparsed: List[str]  # 有 uniffi 的 derive、却没认出后面的类型声明的位置


def strip_comment(line: str) -> str:
    index = line.find("//")
    return line if index < 0 else line[:index]


def brace_delta(line: str) -> int:
    return line.count("{") + line.count("(") - line.count("}") - line.count(")")


def _type_refs(text: str) -> Set[str]:
    return set(TYPE_NAME_RE.findall(text))


def _parse_body(lines: List[str], start: int, kind: str) -> List[Member]:
    """start 是 `pub struct/enum Name {` 所在行的下标。返回字段（结构体）或分支（枚举）。"""
    members: List[Member] = []
    depth = 0
    current: Optional[Member] = None
    catch_all = False
    for index in range(start, len(lines)):
        code = strip_comment(lines[index])
        if depth == 1 and "serde(other)" in code:
            catch_all = True
        if depth == 1 and not code.strip().startswith("#"):
            matcher = FIELD_RE if kind == "struct" else VARIANT_RE
            match = matcher.match(code)
            if match:
                current = Member(match.group(1), index + 1)
                current.has_data = kind == "enum" and match.group(2) in ("{", "(")
                current.catch_all, catch_all = catch_all, False
                members.append(current)
                code = code[match.end(1):]
        if current is not None and depth >= 1:
            current.refs |= _type_refs(code)
        depth += brace_delta(code) if index > start else code.count("{")
        if index > start and depth <= 0:
            break
    return members


def _parse_types(path: str, lines: List[str], types: Dict[str, TypeDef], unparsed: List[str]) -> None:
    """derive 和类型声明之间只能有属性（可以跨行，如 #[expect(...)]）；对不上的记进 unparsed。"""
    pending: Optional[str] = None
    derive_line = 0
    attr_depth = 0
    for index, raw in enumerate(lines):
        code = strip_comment(raw).strip()
        derive = DERIVE_RE.search(code) if "derive" in code else None
        if derive:
            pending, derive_line = derive.group(1).lower(), index + 1
            attr_depth = max(code.count("[") - code.count("]"), 0)
            continue
        if pending is None or not code:
            continue
        if attr_depth > 0 or code.startswith("#"):
            attr_depth = max(attr_depth + code.count("[") - code.count("]"), 0)
            continue
        decl = TYPE_DECL_RE.match(code)
        if decl:
            members = _parse_body(lines, index, decl.group(1))
            types[decl.group(2)] = TypeDef(decl.group(2), pending, path, index + 1, members)
        else:
            unparsed.append(f"{path}:{derive_line}")
        pending = None


def _signature(lines: List[str], index: int) -> str:
    """从 `pub fn` 那一行起拼到函数体的 `{` 或声明末尾的 `;`。"""
    parts: List[str] = []
    for line in lines[index:index + 30]:
        code = strip_comment(line)
        parts.append(code)
        if code.rstrip().endswith("{") or code.rstrip().endswith(";"):
            break
    return " ".join(parts)


def _return_refs(signature: str) -> Set[str]:
    arrow = signature.find("->")
    return set() if arrow < 0 else _type_refs(signature[arrow + 2:])


def _parse_exports(path: str, lines: List[str], functions: List[ExportedFn]) -> None:
    index = 0
    while index < len(lines):
        match = EXPORT_RE.match(lines[index])
        if not match:
            index += 1
            continue
        if match.group(1) and "with_foreign" in match.group(1):
            index += 1  # 壳实现、核心调用的回调接口，方向相反，不算壳要用的东西
            continue
        index = _parse_export_target(path, lines, index + 1, functions)


def _parse_export_target(path: str, lines: List[str], index: int, functions: List[ExportedFn]) -> int:
    # 跳过 export 和 impl/fn 之间的文档注释和其他属性。
    while index < len(lines) and not (IMPL_RE.match(lines[index]) or FN_RE.match(lines[index])):
        index += 1
    if index >= len(lines):
        return index
    fn = FN_RE.match(lines[index])
    if fn:
        functions.append(ExportedFn("", fn.group(1), path, index + 1, _return_refs(_signature(lines, index)), False))
        return index + 1
    owner = IMPL_RE.match(lines[index]).group(1)  # type: ignore[union-attr]
    depth = 0
    constructor = False
    for cursor in range(index, len(lines)):
        code = strip_comment(lines[cursor])
        if depth == 1:
            if CONSTRUCTOR_RE.search(code):
                constructor = True
            fn = FN_RE.match(code)
            if fn:
                returns = _return_refs(_signature(lines, cursor))
                functions.append(ExportedFn(owner, fn.group(1), path, cursor + 1, returns, constructor))
                constructor = False
        depth += code.count("{") - code.count("}")
        if cursor > index and depth <= 0:
            return cursor + 1
    return len(lines)


def _skip_dir(path: str) -> bool:
    parts = path.split(os.sep)
    return "tests" in parts or path.endswith(("_tests.rs", "/tests.rs"))


def read_exports(root: str, crate_dirs: List[str]) -> Exports:
    types: Dict[str, TypeDef] = {}
    functions: List[ExportedFn] = []
    unparsed: List[str] = []
    for crate_dir in crate_dirs:
        for directory, _, names in os.walk(os.path.join(root, crate_dir)):
            for name in sorted(names):
                full = os.path.join(directory, name)
                rel = os.path.relpath(full, root)
                if not name.endswith(".rs") or _skip_dir(rel):
                    continue
                with open(full, encoding="utf-8") as handle:
                    lines = handle.read().split("\n")
                _parse_types(rel, lines, types, unparsed)
                _parse_exports(rel, lines, functions)
    return Exports(types, functions, unparsed)
