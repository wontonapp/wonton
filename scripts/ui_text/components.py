"""从仓库里读出设计系统组件（ADR-0026）的名字，给规则二用。

名单不写死：组件会增删改名，写死的名单会悄悄漏掉新组件。
- Kotlin：设计系统目录（ui/design/，不含生成的）里的，和别处以 Wonton 开头的 fun、class，
  第一个参数是 String 的才算，这样 WontonIconButton(icon, …) 这类第一个参数不是文字的不会进来。
- Swift：Design/ 目录里的 View，和别处以 Wonton 开头的 View；Swift 的组件多用带标签的参数，
  那部分由标签规则管，这里只管 WontonX("…") 这种不带标签的第一个参数。
  另外是 View 扩展里以 wonton 开头、第一个参数是 `_ x: String` 的修饰符，如 .wontonPushedScreen("…")。
"""

from __future__ import annotations

import re
from typing import Callable, Dict, Iterable, List, Set, Tuple

from .source import source_lang

KOTLIN_FIRST_STRING_RE = re.compile(
    r"(?m)^[ \t]*(?:@Composable[ \t]+)?(?:(?:public|internal|data)[ \t]+)*(?:fun|class)[ \t]+([A-Z]\w*)"
    r"[ \t]*\(\s*(?:va[lr]\s+)?\w+\s*:\s*String\??\s*[,)=]"
)
SWIFT_VIEW_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:public|internal)[ \t]+)?struct[ \t]+(\w+)(?:<[^{]*?>)?\s*:\s*[^{]*\bView\b"
)
SWIFT_MODIFIER_RE = re.compile(r"\bfunc[ \t]+(wonton\w*)\(\s*_\s+\w+\s*:\s*String\??\s*[,)=]")

# (写法, 名字)，写法同 source.TEXT_APIS："调用" 是 名字(，"修饰" 是 .名字(。
Api = Tuple[str, str]


def _design_dir(path: str, lang: str) -> bool:
    if lang == "kotlin":
        return "/ui/design/" in path
    return "/Design/" in path and "/Generated/" not in path


def find_components(files: Iterable[str], read: Callable[[str], str]) -> Dict[str, Set[Api]]:
    """返回每种语言的设计系统组件接口。"""
    found: Dict[str, Set[Api]] = {"swift": set(), "kotlin": set()}
    readers = {"kotlin": kotlin_components, "swift": swift_components}
    for path in files:
        lang = source_lang(path)
        if lang in readers:
            found[lang] |= readers[lang](read(path), _design_dir(path, lang))
    return found


def kotlin_components(text: str, design_dir: bool) -> Set[Api]:
    names: List[str] = KOTLIN_FIRST_STRING_RE.findall(text)
    return {("调用", n) for n in names if design_dir or n.startswith("Wonton")}


def swift_components(text: str, design_dir: bool) -> Set[Api]:
    views = {("调用", n) for n in SWIFT_VIEW_RE.findall(text) if design_dir or n.startswith("Wonton")}
    return views | {("修饰", n) for n in SWIFT_MODIFIER_RE.findall(text)}
