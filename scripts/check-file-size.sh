#!/usr/bin/env bash
# 源码文件行数门禁：只数代码行，不算空行和只有注释的行。
# 超过 800 行只警告（该按职责拆的信号），超过 2000 行失败。
# 只有注释的行指去掉首尾空白后以注释符开头的行：`#`（sh、py），`//` 和 `/* … */` 块里的行（其余语言）。
# 不做完整的语法分析：代码后面跟的注释、字符串里的注释符都算代码行。
# 文案文件（.xcstrings、strings*.xml、.ftl）也算在内：键按前缀分文件（ADR-0043），装不下就在前缀上再拆细。
# 确需超过 2000 行时写进仓库根目录的 .size-allowlist，每行一条：
#   <路径> | <原因> | <到期日 YYYY-MM-DD>
# 例外只免掉 2000 行的失败，800 行的警告照常打印；到期后例外失效，CI 重新失败。
# vendor/ 下是原样导入的第三方源码（见 vendor/README.md），不按我们的上限拆。
#
# 给了参数就只查这些文件（`lefthook.yml` 的 pre-commit 传暂存的文件），不给就查全部跟踪的源码文件。
set -euo pipefail

warn_limit="${WONTON_WARN_FILE_LINES:-800}"
fail_limit="${WONTON_MAX_FILE_LINES:-2000}"
allowlist=".size-allowlist"
today="$(date -u +%F)"
status=0
warned=0

trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

is_allowed() {
  local target="$1" line path expires
  [[ -f "$allowlist" ]] || return 1
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$(trim "$line")" || "$line" == \#* ]] && continue
    IFS='|' read -r path _ expires <<<"$line"
    path="$(trim "$path")"
    expires="$(trim "${expires:-}")"
    [[ "$path" == "$target" ]] || continue
    if [[ -z "$expires" ]]; then
      echo "例外缺少到期日：$target" >&2
      return 1
    fi
    if [[ "$expires" < "$today" ]]; then
      echo "例外已过期：$target（到期 $expires）" >&2
      return 1
    fi
    return 0
  done <"$allowlist"
  return 1
}

# 数代码行。style 为 hash 时 `#` 开头的行是注释；为 c 时 `//` 开头的行和 `/* … */` 块里的行是注释。
count_code_lines() {
  local style="$1" file="$2"
  awk -v style="$style" '
    {
      line = $0
      sub(/^[ \t\r]+/, "", line)
      sub(/[ \t\r]+$/, "", line)
    }
    in_block {
      if (index(line, "*/")) in_block = 0
      next
    }
    line == "" { next }
    style == "hash" && substr(line, 1, 1) == "#" { next }
    style == "c" && substr(line, 1, 2) == "//" { next }
    style == "c" && substr(line, 1, 2) == "/*" {
      if (!index(substr(line, 3), "*/")) in_block = 1
      next
    }
    { count++ }
    END { print count + 0 }
  ' "$file"
}

comment_style() {
  case "$1" in
    *.sh | *.py) echo hash ;;
    *) echo c ;;
  esac
}

report_warning() {
  local file="$1" lines="$2"
  local message="$file 有 ${lines} 行代码（不算空行和注释），超过 ${warn_limit}，按职责拆"
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "::warning file=${file}::${message}"
  else
    echo "警告：$message" >&2
  fi
}

emit_files() {
  if (($# > 0)); then
    printf '%s\0' "$@"
  else
    git ls-files -z -- \
      '*.rs' '*.swift' '*.kt' '*.kts' '*.ts' '*.tsx' '*.js' '*.mjs' '*.py' '*.go' '*.sh' \
      '*.xcstrings' '*.xml' '*.ftl' \
      ':(exclude)**/generated/**' ':(exclude)vendor/**'
  fi
}

while IFS= read -r -d '' file; do
  # 参数里可能有已删除的文件；vendor 和生成的文件在两种模式下都跳过。
  [[ -f "$file" ]] || continue
  case "$file" in vendor/* | generated/* | */generated/*) continue ;; esac
  lines="$(count_code_lines "$(comment_style "$file")" "$file")"
  if ((lines > fail_limit)) && ! is_allowed "$file"; then
    echo "超过 ${fail_limit} 行代码：$file（${lines} 行，不算空行和注释），按职责拆开" >&2
    status=1
  elif ((lines > warn_limit)); then
    report_warning "$file" "$lines"
    warned=$((warned + 1))
  fi
done < <(emit_files "$@")

if ((status == 0)); then
  echo "文件行数检查通过（${warned} 个文件超过 ${warn_limit} 行代码只警告，上限 ${fail_limit} 行）"
fi
exit "$status"
