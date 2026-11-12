"""“ViewModel 项”的定义：壳要渲染的字段和分支、壳要发的操作、壳要调的核心函数。

- 操作：`Action` 的每个分支（壳经 `ClientCore::dispatch` 发）。分支里的参数不算：Kotlin 常按位置传参，名字不出现。
- 函数：导出的对象方法、命名构造函数和自由函数。主构造函数 `new` 在壳里写成类型名，不算。
- 输出：从导出函数的返回类型（各个 ViewModel、订阅推的批次等）出发，顺着字段和分支里的类型能走到的
  结构体和枚举；结构体的每个字段是一项；枚举里带数据的分支各是一项，不带数据的分支合起来是一项。只作为操作参数的结构体是输入，不算。

不算的分支：向前兼容的兜底分支（`Other` 和带 `#[serde(other)]` 的）。Swift 的 switch 要穷尽，必须写出它；
Kotlin 的 when 用 else 接住，不写名字。两边都处理了，只是写法不同。

不算的类型（连同它们的字段和分支，也不往下走）：
- 错误类别：`uniffi::Error` 和名字以 Failure、Problem、Error 结尾的类型。类别由核心给、文字在壳里写，
  两边各自决定哪些类别单独写文字，不要求一一对应。
- 通用的工具类型：见 UTILITY_TYPES。
- 平台专属和其他有意只在一边的：白名单里按类型整个列出。
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Set

from .rust import Exports, Member, TypeDef
from .shells import Shell, lower_camel

ERROR_SUFFIX_RE = re.compile(r"(Failure|Problem|Error)$")
# 只是值的载体，两边都交给通用的转换函数（颜色转成平台的颜色类型），字段不是界面上的一项。
UTILITY_TYPES = {"Rgb"}
ACTION_TYPE = "Action"
CATCH_ALL = "Other"


@dataclass
class Item:
    key: str  # 白名单和报告里用的名字，如 HomeViewModel.status、Action.PairHome、ClientCore.tabs
    owner: str  # 所属的类型或对象；自由函数为空字符串
    kind: str  # field | enum | variant | action | function
    path: str
    line: int
    # 在壳里找的写法，找到任何一个就算用到：`.名字` 是成员（Swift 枚举分支），`类型.名字` 是限定名，其余是标识符。
    swift: List[str]
    kotlin: List[str]


def excluded_type(definition: TypeDef, allow_types: Set[str]) -> bool:
    return (
        definition.kind == "error"
        or bool(ERROR_SUFFIX_RE.search(definition.name))
        or definition.name in UTILITY_TYPES
        or definition.name in allow_types
    )


def _walk(exports: Exports, allow_types: Set[str], follow: Callable[[TypeDef, Member], bool]) -> Set[str]:
    """从导出函数的返回类型出发，经 follow 放行的字段和分支能走到的结构体和枚举。"""
    types = exports.types
    queue = [name for fn in exports.functions if fn.owner not in allow_types for name in fn.returns]
    seen: Set[str] = set()
    while queue:
        name = queue.pop()
        definition = types.get(name)
        if name in seen or definition is None or definition.kind == "object":
            continue
        if name == ACTION_TYPE or excluded_type(definition, allow_types):
            continue
        seen.add(name)
        for member in definition.members:
            if follow(definition, member):
                queue.extend(member.refs)
    return seen


def output_types(exports: Exports, allow_types: Set[str]) -> List[TypeDef]:
    seen = _walk(exports, allow_types, lambda definition, member: True)
    return [exports.types[name] for name in sorted(seen)]


def _enum_items(definition: TypeDef) -> Iterable[Item]:
    variants = [m for m in definition.members if not (m.catch_all or m.name == CATCH_ALL)]
    if definition.name == ACTION_TYPE:
        for member in variants:
            key = f"{ACTION_TYPE}.{member.name}"
            yield Item(key, ACTION_TYPE, "action", definition.path, member.line, [f".{lower_camel(member.name)}"], [key])
        return
    # 不带数据的分支合起来算一项：Swift 的 switch 要穷尽，每个分支都写出来；Kotlin 的 when 常把几个并进 else。
    # 按分支比会把这种写法差别全报成“只有 iOS 用到”。Kotlin 里提到枚举名就算用到。
    plain = [f".{lower_camel(m.name)}" for m in variants if not m.has_data]
    if plain:
        yield Item(definition.name, definition.name, "enum", definition.path, definition.line, plain, [definition.name])
    # 带数据的分支各算一项：数据要在分支里取出来渲染，并进 else 就是没显示。不区分 Swift 的 `case .running:`
    # （只为穷尽）和取数据的写法：Kotlin 的 `is ChatAccount.SignedIn` 也常常不取数据，只认一边会更不对称。
    for member in variants:
        if member.has_data:
            key = f"{definition.name}.{member.name}"
            yield Item(key, definition.name, "variant", definition.path, member.line, [f".{lower_camel(member.name)}"], [key])


def _record_items(definition: TypeDef) -> Iterable[Item]:
    for member in definition.members:
        name = lower_camel(member.name)
        key = f"{definition.name}.{member.name}"
        # 只认成员访问（x.name、\\.name、$0.name）：光看标识符会撞上同名的局部变量。壳里不用 Kotlin 的
        # with/run 不带点地读核心的结构体，也不按位置解构它们。
        yield Item(key, definition.name, "field", definition.path, member.line, [f".{name}"], [f".{name}"])


def _type_items(definition: TypeDef) -> Iterable[Item]:
    return _record_items(definition) if definition.kind == "record" else _enum_items(definition)


def _function_items(exports: Exports, allow_types: Set[str]) -> Iterable[Item]:
    for fn in exports.functions:
        if fn.owner in allow_types or (fn.constructor and fn.name == "new"):
            continue
        name = lower_camel(fn.name)
        key = f"{fn.owner}.{fn.name}" if fn.owner else fn.name
        yield Item(key, fn.owner, "function", fn.path, fn.line, [name], [name])


def collect_items(exports: Exports, allow_types: Set[str]) -> List[Item]:
    items: List[Item] = []
    action = exports.types.get(ACTION_TYPE)
    if action is not None:
        items.extend(_type_items(action))
    for definition in output_types(exports, allow_types):
        items.extend(_type_items(definition))
    items.extend(_function_items(exports, allow_types))
    return items


def contexts(exports: Exports) -> Dict[str, Set[str]]:
    """每个类型在壳的源文件里会以什么名字出现：类型名本身、指向它的字段和分支的名字、返回它的函数名。

    Swift 的 `.file`、`.default` 到处都是（FileManager.default），真读 Transfer.file 的文件一般声明了 Transfer
    类型的参数，或者写了 `.transfers` 这样的上层字段。
    """
    found: Dict[str, Set[str]] = {name: {name} for name in exports.types}
    for fn in exports.functions:
        for ref in fn.returns & found.keys():
            found[ref].add(lower_camel(fn.name))
    for definition in exports.types.values():
        for member in definition.members:
            for ref in member.refs & found.keys():
                found[ref] |= {member.name, lower_camel(member.name)}
    return found


def _patterns(item: Item, swift: bool) -> List[str]:
    return item.swift if swift else item.kotlin


def used_in(item: Item, shell: Shell, swift: bool) -> bool:
    return any(shell.finds(pattern) for pattern in _patterns(item, swift))


def _member_used(definition: TypeDef, member: Member, shell: Shell, swift: bool) -> bool:
    if swift or definition.kind == "record":
        return shell.finds(f".{lower_camel(member.name)}")
    return shell.finds(f"{definition.name}.{member.name}")


def reachable(exports: Exports, allow_types: Set[str], shell: Shell, swift: bool) -> Set[str]:
    """壳实际走得到的类型：只顺着这个壳用到的字段和分支往下走。

    按名字找会撞上同名的东西（Swift 的 `.ready` 可能是别的枚举的分支），一项的所属类型这个壳根本走不到时，
    同名的引用就不算。
    """
    return _walk(exports, allow_types, lambda definition, member: _member_used(definition, member, shell, swift))


@dataclass
class Usage:
    shells: Dict[str, Set[str]]  # 每一项在哪些壳里用到
    nested: Set[str]  # 所属类型只有一个壳走得到的项：缺口已经报在上层那个字段或分支上
    doubtful: Set[str]  # 只有一个壳用到，但那边的引用所在的文件看不出和所属类型有关，可能是同名的别的东西


def usage(exports: Exports, allow_types: Set[str], items: List[Item], shells: Dict[str, Shell]) -> Usage:
    """字段和分支除了名字对得上，还要求所属类型在这个壳里走得到。"""
    used: Dict[str, Set[str]] = {item.key: set() for item in items}
    reach: Dict[str, Set[str]] = {}
    for name, shell in shells.items():
        swift = name == "iOS"
        reach[name] = reachable(exports, allow_types, shell, swift)
        for item in items:
            top_level = item.kind in ("action", "function")
            if (top_level or item.owner in reach[name]) and used_in(item, shell, swift):
                used[item.key].add(name)
    in_all = set.intersection(*reach.values())
    one_shell = [item for item in items if len(used[item.key]) == 1 and item.kind not in ("action", "function")]
    nested = {item.key for item in one_shell if item.owner not in in_all}
    doubtful = {
        item.key
        for item in one_shell
        if item.key not in nested
        for shell_name in used[item.key]
        if not _certain(item, exports, shells[shell_name], shell_name == "iOS", reach[shell_name])
    }
    return Usage(used, nested, doubtful)


def _certain(item: Item, exports: Exports, shell: Shell, swift: bool, reach: Set[str]) -> bool:
    """判“只有一边用到”之前，再严格查一遍用到的那边，只用来压掉警告、不用来判“没用到”。

    引用要和所属类型的名字、上层字段名或取它的函数名在同一个文件里（Swift 里常见 model.viewModel.x，文件里
    不一定出现这些名字，所以不能拿它判“没用到”）。这个壳走得到的别的类型也有同名成员时（Swift 的 `.bot` 可能是
    TabRef.bot），文件里还得直接提到所属类型。
    """
    names = contexts(exports)
    shared = {
        lower_camel(member.name)
        for name in reach - {item.owner}
        for member in exports.types[name].members
    }
    for pattern in _patterns(item, swift):
        if not shell.finds(pattern, names[item.owner]):
            continue
        ambiguous = pattern.startswith(".") and pattern[1:] in shared
        if not ambiguous or shell.finds(pattern, {item.owner}):
            return True
    return False
