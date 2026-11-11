#!/usr/bin/env bash
# UniFFI 把文档注释原样写进生成的 Swift 和 Kotlin 块注释。这两种语言的块注释可以嵌套，注释里的 /* 会开一个
# 永远不闭合的注释、*/ 会提前结束注释，生成的绑定都编译不过（#49）。生成绑定的 crate 的文档注释里不许出现它们。
set -euo pipefail

if grep -rnE '^[[:space:]]*//[/!].*(/\*|\*/)' crates/wonton-protocol/src crates/wonton-client/src; then
  echo "上面这些文档注释里有 /* 或 */，会让生成的 Swift 和 Kotlin 绑定编译失败，换个写法" >&2
  exit 1
fi
