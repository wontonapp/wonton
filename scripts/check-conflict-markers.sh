#!/usr/bin/env bash
# 合并冲突标记门禁：残留的 <<<<<<< 、>>>>>>> 和分隔线 ======= 不能进主干。
# Markdown 不查单独一行 =======：它可能是合法的 setext 标题，语法上和冲突分隔线分不开；
# 真实的冲突总带 <<<<<<< 和 >>>>>>>，这两样照样挡得住。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

status=0

# git grep 的退出码：0 有匹配，1 没匹配，大于 1 是自身出错——出错必须失败，不能当成没查到。
scan() {
  local pattern="$1"
  shift
  local out rc=0
  out="$(git grep -I -n -e "$pattern" -- . "$@")" || rc=$?
  case "$rc" in
    0) printf '%s\n' "$out" >&2; return 1 ;;
    1) return 0 ;;
    *) echo "git grep 查 ${pattern} 出错（退出码 ${rc}），检查失败" >&2; return 2 ;;
  esac
}

scan '^<<<<<<< ' || status=1
scan '^>>>>>>> ' || status=1
scan '^=======$' ':(exclude)*.md' || status=1

if (( status == 0 )); then
  echo "冲突标记检查通过"
fi
exit "$status"
