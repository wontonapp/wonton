"""Swift、Kotlin、Rust 源码的小词法分析器，只为界面文字检查服务。

只关心标识符、标点，以及字符串和字符字面量（字面量只保留静态文字，转义解码，插值去掉）。
注释整段跳过；插值里的表达式继续按代码扫描，因为里面还可能有字符串。
三种语言的差别全部放在 Dialect 里：开头的引号怎么认，字符字面量和生命周期怎么分。
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Callable, List, NamedTuple, Optional, Tuple

IDENT_RE = re.compile(r"[^\W\d]\w*")
NUMBER_RE = re.compile(r"\d\w*")
SWIFT_OPEN_RE = re.compile(r'(#*)("""|")')
RUST_RAW_OPEN_RE = re.compile(r'b?r(#*)"')
UNICODE_ESCAPE_RE = re.compile(r"u(?:\{([0-9a-fA-F]{1,8})\}|([0-9a-fA-F]{4}))")
MAX_CODEPOINT = 0x10FFFF


@dataclass(frozen=True)
class Opening:
    """字符串开头的形状。

    skip 是前缀（Rust 的 r、b、br）加开头引号的长度；hashes 是 Swift 的 #"…"# 和 Rust 的 r#"…"# 的 # 个数。
    interp 是插值写法：swift 是 \\(…)，kotlin 是 ${…} 和 $名字，none 没有。
    """

    skip: int
    hashes: int
    triple: bool
    escapes: bool
    newlines: bool
    interp: str


@dataclass
class Literal:
    line: int
    chars: List[Tuple[str, int]]

    def text(self) -> str:
        return "".join(ch for ch, _ in self.chars)


@dataclass
class Token:
    kind: str  # ident | num | punct | str | char
    text: str  # ident、num、punct 的原文；字面量为空串
    line: int
    lit: Optional[Literal] = None


def _open_swift(src: str, i: int) -> Optional[Opening]:
    m = SWIFT_OPEN_RE.match(src, i)
    if not m:
        return None
    triple = m.group(2) == '"""'
    return Opening(
        skip=len(m.group(0)),
        hashes=len(m.group(1)),
        triple=triple,
        escapes=True,
        newlines=triple,
        interp="swift",
    )


def _open_kotlin(src: str, i: int) -> Optional[Opening]:
    if src.startswith('"""', i):
        return Opening(3, 0, triple=True, escapes=False, newlines=True, interp="kotlin")
    if src.startswith('"', i):
        return Opening(1, 0, triple=False, escapes=True, newlines=False, interp="kotlin")
    return None


def _open_rust(src: str, i: int) -> Optional[Opening]:
    m = RUST_RAW_OPEN_RE.match(src, i)
    if m:
        return Opening(len(m.group(0)), len(m.group(1)), False, False, True, "none")
    if src.startswith('b"', i):
        return Opening(2, 0, False, True, True, "none")
    if src.startswith('"', i):
        return Opening(1, 0, False, True, True, "none")
    return None


CHAR_OPENING = Opening(skip=0, hashes=0, triple=False, escapes=True, newlines=False, interp="none")


class Dialect(NamedTuple):
    opening: Callable[[str, int], Optional[Opening]]
    char_literals: bool
    lifetimes: bool  # Rust：'a 是生命周期或标签，不是字符字面量


DIALECTS = {
    "swift": Dialect(_open_swift, char_literals=False, lifetimes=False),
    "kotlin": Dialect(_open_kotlin, char_literals=True, lifetimes=False),
    "rust": Dialect(_open_rust, char_literals=True, lifetimes=True),
}


def tokenize(src: str, lang: str) -> List[Token]:
    """返回 token 列表。字符串的 token 位置早于它的插值里的 token。"""
    lexer = _Lexer(src, DIALECTS[lang])
    lexer.code(closer=None)
    return lexer.tokens


class _Lexer:
    def __init__(self, src: str, dialect: Dialect) -> None:
        self.src = src
        self.dialect = dialect
        self.i = 0
        self.line = 1
        self.tokens: List[Token] = []

    def done(self) -> bool:
        return self.i >= len(self.src)

    def ch(self, k: int = 0) -> str:
        j = self.i + k
        return self.src[j] if j < len(self.src) else ""

    def at(self, s: str) -> bool:
        return self.src.startswith(s, self.i)

    def adv(self, k: int = 1) -> None:
        self.line += self.src.count("\n", self.i, self.i + k)
        self.i += k

    def emit(self, kind: str, text: str) -> None:
        self.tokens.append(Token(kind, text, self.line))
        self.adv(len(text))

    def code(self, closer: Optional[str]) -> None:
        """扫描代码直到 closer（插值的 ) 或 }）且没有未配对的括号；closer 为 None 时扫到文件末尾。"""
        depth = 0
        while not self.done():
            c = self.ch()
            if c.isspace():
                self.adv()
            elif self.at("//"):
                self.skip_line()
            elif self.at("/*"):
                self.skip_block()
            elif closer is not None and depth == 0 and c == closer:
                self.adv()
                return
            else:
                if c in "([{":
                    depth += 1
                elif c in ")]}" and depth > 0:
                    depth -= 1
                self.token()

    def skip_line(self) -> None:
        end = self.src.find("\n", self.i)
        self.adv((len(self.src) if end == -1 else end) - self.i)

    def skip_block(self) -> None:
        depth = 0
        while not self.done():
            if self.at("/*"):
                depth += 1
                self.adv(2)
            elif self.at("*/"):
                depth -= 1
                self.adv(2)
                if depth == 0:
                    return
            else:
                self.adv()

    def token(self) -> None:
        opening = self.dialect.opening(self.src, self.i)
        if opening is not None:
            self.string(opening)
            return
        if self.ch() == "'" and self.dialect.char_literals:
            if not self.dialect.lifetimes or self.ch(1) == "\\" or self.ch(2) == "'":
                self.char_literal()
            else:
                self.adv()  # 生命周期或标签：只跳过引号，名字按标识符读
            return
        for pattern, kind in ((IDENT_RE, "ident"), (NUMBER_RE, "num")):
            m = pattern.match(self.src, self.i)
            if m:
                self.emit(kind, m.group())
                return
        self.emit("punct", "==" if self.at("==") else self.ch())

    def string(self, opening: Opening) -> None:
        # 字符串的 token 先占住位置，它插值里的 token 排在后面，这样 Text( 后面紧跟的就是字符串本身。
        index = len(self.tokens)
        self.tokens.append(None)  # type: ignore[arg-type]
        line = self.line
        chars: List[Tuple[str, int]] = []
        self.adv(opening.skip)
        self.body(opening, chars)
        self.tokens[index] = Token("str", "", line, Literal(line, chars))

    def char_literal(self) -> None:
        index = len(self.tokens)
        self.tokens.append(None)  # type: ignore[arg-type]
        line = self.line
        chars: List[Tuple[str, int]] = []
        self.adv()
        while not self.done() and self.ch() not in ("'", "\n"):
            if self.ch() == "\\":
                self.escape(CHAR_OPENING, chars)
            else:
                chars.append((self.ch(), self.line))
                self.adv()
        if self.ch() == "'":
            self.adv()
        self.tokens[index] = Token("char", "", line, Literal(line, chars))

    def body(self, op: Opening, chars: List[Tuple[str, int]]) -> None:
        quote = '"""' if op.triple else '"'
        closer = quote + "#" * op.hashes
        escape_intro = "\\" + "#" * op.hashes
        while not self.done():
            if self.at(closer):
                if op.triple and op.hashes == 0:
                    # Kotlin 的 """a"""" 末尾多出来的引号属于内容，最后三个才是结束。
                    run = 0
                    while self.ch(run) == '"':
                        run += 1
                    chars.extend(('"', self.line) for _ in range(run - 3))
                    self.adv(run)
                else:
                    self.adv(len(closer))
                return
            c = self.ch()
            if c == "\n" and not op.newlines:
                return  # 单行字符串没有闭合，停在行尾
            if op.escapes and self.at(escape_intro):
                self.escape(op, chars)
            elif op.interp == "kotlin" and c == "$" and self.kotlin_template(chars):
                continue
            else:
                chars.append((c, self.line))
                self.adv()

    def escape(self, op: Opening, chars: List[Tuple[str, int]]) -> None:
        line = self.line
        self.adv(1 + op.hashes)
        if op.interp == "swift" and self.ch() == "(":
            self.adv()
            self.code(closer=")")
            chars.append((" ", line))
            return
        m = UNICODE_ESCAPE_RE.match(self.src, self.i)
        if m:
            self.adv(len(m.group()))
            value = int(m.group(1) or m.group(2), 16)
            chars.append((chr(value) if value <= MAX_CODEPOINT else " ", line))
            return
        # 其他转义（\n、\"、\\ 等）不算文字，记成空格。
        chars.append((" ", line))
        self.adv()

    def kotlin_template(self, chars: List[Tuple[str, int]]) -> bool:
        """在 $ 处：${…} 或 $名字 是插值，静态文字里记成空格；其他的 $ 是普通字符。"""
        if self.ch(1) == "{":
            line = self.line
            self.adv(2)
            self.code(closer="}")
            chars.append((" ", line))
            return True
        m = IDENT_RE.match(self.src, self.i + 1)
        if m:
            chars.append((" ", self.line))
            self.adv(1 + len(m.group()))
            return True
        return False
