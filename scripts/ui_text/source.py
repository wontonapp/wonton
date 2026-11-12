"""源码的范围和规则一、规则二（ADR-0043 的“防止再写死”）。

规则一：字符串或字符字面量里有汉字、假名、CJK 符号或全角字符，报错（注释不算）。
规则二：Swift 和 Kotlin 里，直接传给界面接口（系统控件、设计系统组件、文字类的参数）的字符串字面量像界面文字时，报错；
TabText.choose(…)、tabText(…) 是迁移前按语言挑文字的旧写法，调用本身就报错。
"""

from __future__ import annotations

import re
from typing import Collection, Dict, List, Optional, Set, Tuple

from .lexer import Token, tokenize

# 客户端核心的 Rust 源码暂时不查规则一：核心的 Fluent 目录（ADR-0043）进 main 之前，新写的 Rust 文字没有地方放，
# 查了只能往白名单里加。Fluent 目录进 main 后改成 True，crates/ 的白名单条目随之重新生效。
CHECK_RUST = False
RUST_PREFIXES = ("crates/wonton-client/src/", "crates/wonton-protocol/src/")

# 直接收字符串字面量作为第一个参数（或参数的值）的界面接口。新增接口只改这里；设计系统组件从仓库读（components.py）。
# ("调用", 名字)：名字( ；("修饰", 名字)：.名字( ；("命名", 名字)：名字 = ；("标签", 名字)：(或, 之后的 名字:
TEXT_APIS: Dict[str, List[Tuple[str, str]]] = {
    "swift": [
        *[("调用", n) for n in ("Text", "Button", "Label", "Toggle", "TextField", "SecureField",
                               "Section", "Picker", "Menu", "Link")],
        *[("修饰", n) for n in ("navigationTitle", "accessibilityLabel", "accessibilityHint", "help",
                               "alert", "confirmationDialog")],
        *[("标签", n) for n in ("title", "message", "label", "subtitle", "detail", "placeholder", "prompt")],
    ],
    "kotlin": [
        ("调用", "Text"),
        *[("命名", n) for n in ("text", "label", "contentDescription", "title", "placeholder", "message",
                               "subtitle", "detail")],
    ],
}

# 迁移前按系统语言二选一的写法：(限定名或空, 函数名)。
LEGACY_PICKERS: Dict[str, List[Tuple[str, str]]] = {
    "swift": [("TabText", "choose")],
    "kotlin": [("", "tabText")],
}

# 汉字、假名、CJK 符号和标点、全角形式，以及扩展的汉字区。
CJK_RANGES = (
    (0x3000, 0x303F),
    (0x3040, 0x309F),
    (0x30A0, 0x30FF),
    (0x31F0, 0x31FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0xFF00, 0xFFEF),
    (0x20000, 0x2FA1F),
)

Problem = Tuple[int, str]  # (行号, 中文说明)


def source_lang(path: str) -> Optional[str]:
    """返回文件用哪种语言检查，不在范围内返回 None。"""
    parts = path.split("/")
    name = parts[-1]
    if path.startswith("apps/ios/") and path.endswith(".swift"):
        return None if any("Tests" in p for p in parts) else "swift"
    if path.startswith("apps/compose/") and path.endswith(".kt"):
        if "/src/" not in "/" + path or re.search(r"/src/(test|androidTest)", "/" + path):
            return None
        if "/ui/icons/" in "/" + path or "/ui/design/generated/" in "/" + path:
            return None
        return "kotlin"
    if CHECK_RUST and path.startswith(RUST_PREFIXES) and path.endswith(".rs"):
        if name == "tests.rs" or name.endswith("_tests.rs") or "tests" in parts[:-1]:
            return None
        return "rust"
    return None


def paused(path: str) -> bool:
    """暂时不查、但白名单条目要留着的文件（见 CHECK_RUST）。"""
    return not CHECK_RUST and path.startswith(RUST_PREFIXES) and path.endswith(".rs")


def cut_test_module(text: str) -> str:
    """Rust 的测试都放在文件末尾：从第一行 #[cfg(test)] 起整段去掉，行号保持不变。"""
    lines = text.split("\n")
    for index, line in enumerate(lines):
        if "#[cfg(test)]" in line:
            return "\n".join(lines[:index])
    return text


def is_cjk(ch: str) -> bool:
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in CJK_RANGES)


def _snippet(text: str) -> str:
    shown = text[:30].replace("\n", "\\n")
    return shown + ("…" if len(text) > 30 else "")


def _cjk_problems(tokens: List[Token]) -> Tuple[List[Problem], set]:
    problems: List[Problem] = []
    flagged = set()
    for tok in tokens:
        if tok.kind not in ("str", "char") or tok.lit is None:
            continue
        bad = [line for ch, line in tok.lit.chars if is_cjk(ch)]
        if bad:
            flagged.add(id(tok.lit))
            problems.append((bad[0], f"字符串里有中文、日文或全角字符「{_snippet(tok.lit.text())}」，界面文字要进字符串目录"))
    return problems, flagged


# 不像界面文字的单个词：小写开头的标识符（gear、showLabels、tab_bar）、点分的 SF Symbol 名（chevron.left）、
# 全大写加下划线的常量名、# 开头的格式示例（#RRGGBB），以及词中间带 . : / @ 的地址、路径和网址（mirror.example.com:5000）。
# 英文界面文字要么有空格，要么是大写开头的词（Done、Retry）。
NOT_TEXT_WORD_RE = re.compile(
    r"[a-z][A-Za-z0-9_]*|[a-z0-9]+(?:[.\-][a-z0-9]+)+|[A-Z0-9]+(?:_[A-Z0-9]+)+|#\S*|\S*(?:\w[.:/@]|//)\S*"
)
# Compose 动画接口的 label = 是给调试工具看的名字，不是界面文字。
KOTLIN_DEBUG_LABEL_CALLS = re.compile(r"animate\w*|updateTransition|rememberInfiniteTransition|Crossfade|AnimatedContent")
# printf 风格的占位符（%@、%d、%1$s、%lld），去掉后没有字母就是纯格式串。
FORMAT_SPEC_RE = re.compile(r"%(?:\d+\$)?[-+ #0]*\d*(?:\.\d+)?(?:hh|h|ll|l|q|z|t|j)?[@dDuUxXoOfFeEgGcCsSaAp]")


def looks_like_text(text: str) -> bool:
    """字面量像不像给人看的界面文字。插值在词法分析时已经换成空格。宁可漏，不误报。"""
    stripped = FORMAT_SPEC_RE.sub(" ", text).strip()
    if not any(ch.isalpha() for ch in stripped):
        return False
    return not NOT_TEXT_WORD_RE.fullmatch(stripped)


def _tok(tokens: List[Token], j: int) -> Optional[Token]:
    return tokens[j] if 0 <= j < len(tokens) else None


def _enclosing_call(tokens: List[Token], k: int) -> Optional[str]:
    """tokens[k] 所在的那层括号前面的函数名；不在括号里返回 None。"""
    depth = 0
    for j in range(k - 1, -1, -1):
        text = tokens[j].text if tokens[j].kind == "punct" else ""
        if text == ")":
            depth += 1
        elif text == "(":
            if depth == 0:
                name = _tok(tokens, j - 1)
                return name.text if name is not None and name.kind == "ident" else None
            depth -= 1
    return None


def _debug_label(tokens: List[Token], k: int) -> bool:
    if tokens[k].text != "label":
        return False
    call = _enclosing_call(tokens, k)
    return call is not None and KOTLIN_DEBUG_LABEL_CALLS.fullmatch(call) is not None


def _api_hit(tokens: List[Token], k: int, apis: Collection[Tuple[str, str]]) -> Optional[Tuple[str, int]]:
    """tokens[k] 开头是界面接口时，返回 (接口写法, 文字参数的下标)。"""
    head, after = _tok(tokens, k), _tok(tokens, k + 1)
    if head is None or after is None:
        return None
    if head.kind == "ident" and after.kind == "punct":
        if after.text == "(" and ("调用", head.text) in apis:
            return f"{head.text}(", k + 2
        if after.text == "=" and ("命名", head.text) in apis and not _debug_label(tokens, k):
            return f"{head.text} =", k + 2
        before = _tok(tokens, k - 1)
        if after.text == ":" and ("标签", head.text) in apis and before is not None and before.text in ("(", ","):
            return f"{head.text}:", k + 2
    if head.kind == "punct" and head.text == "." and after.kind == "ident":
        paren = _tok(tokens, k + 2)
        if paren is not None and paren.text == "(" and ("修饰", after.text) in apis:
            return f".{after.text}(", k + 3
    return None


def _api_problems(tokens: List[Token], apis: Set[Tuple[str, str]], flagged: set) -> List[Problem]:
    problems: List[Problem] = []
    for k in range(len(tokens)):
        hit = _api_hit(tokens, k, apis)
        if hit is None:
            continue
        label, arg = hit
        lit = tokens[arg].lit if arg < len(tokens) and tokens[arg].kind == "str" else None
        if lit is None or id(lit) in flagged or not looks_like_text(lit.text()):
            continue
        problems.append((lit.line, f"界面接口 {label} 直接收到写死的文字「{_snippet(lit.text())}」，改成字符串资源"))
    return problems


def _legacy_problems(tokens: List[Token], lang: str) -> List[Problem]:
    """TabText.choose(…)、tabText(…) 的调用（不含定义本身）。"""
    problems: List[Problem] = []
    for owner, name in LEGACY_PICKERS[lang]:
        for k, tok in enumerate(tokens):
            nxt, prev = _tok(tokens, k + 1), _tok(tokens, k - 1)
            if tok.kind != "ident" or tok.text != name or nxt is None or nxt.text != "(":
                continue
            if owner:
                qualifier = _tok(tokens, k - 2)
                if prev is None or prev.text != "." or qualifier is None or qualifier.text != owner:
                    continue
            elif prev is not None and prev.text in ("fun", "func", "."):
                continue
            shown = f"{owner}.{name}" if owner else name
            problems.append((tok.line, f"{shown}(…) 是按语言二选一的旧写法，界面文字改成字符串资源"))
    return problems


def scan_source(text: str, lang: str, components: Collection[Tuple[str, str]] = ()) -> List[Problem]:
    """检查一个源码文件，返回 (行号, 说明) 列表。components 是这种语言的设计系统组件（components.py）。"""
    src = cut_test_module(text) if lang == "rust" else text
    tokens = tokenize(src, lang)
    problems, flagged = _cjk_problems(tokens)
    if lang != "rust":
        problems += _api_problems(tokens, set(TEXT_APIS[lang]) | set(components), flagged)
        problems += _legacy_problems(tokens, lang)
    return problems
