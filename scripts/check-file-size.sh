#!/usr/bin/env bash
# 源码文件行数门禁：超过上限即失败。
# 例外写进仓库根目录的 .size-allowlist，每行一条：
#   <路径> | <原因> | <到期日 YYYY-MM-DD>
# 到期后例外失效，CI 重新失败。
# vendor/ 下是原样导入的第三方源码（见 vendor/README.md），不按我们的上限拆。
#
# 给了参数就只查这些文件（`lefthook.yml` 的 pre-commit 传暂存的文件），不给就查全部跟踪的源码文件。
set -euo pipefail

limit="${WONTON_MAX_FILE_LINES:-800}"
allowlist=".size-allowlist"
today="$(date -u +%F)"
status=0

trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

is_allowed() {
  local target="$1" line path reason expires
  [[ -f "$allowlist" ]] || return 1
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$(trim "$line")" || "$line" == \#* ]] && continue
    IFS='|' read -r path reason expires <<<"$line"
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

emit_files() {
  if (($# > 0)); then
    printf '%s\0' "$@"
  else
    git ls-files -z -- \
      '*.rs' '*.swift' '*.kt' '*.kts' '*.ts' '*.tsx' '*.js' '*.mjs' '*.py' '*.go' '*.sh' \
      ':(exclude)**/generated/**' ':(exclude)vendor/**'
  fi
}

while IFS= read -r -d '' file; do
  # 参数里可能有已删除的文件；vendor 和生成的文件在两种模式下都跳过。
  [[ -f "$file" ]] || continue
  case "$file" in vendor/* | generated/* | */generated/*) continue ;; esac
  lines="$(wc -l <"$file" | tr -d ' ')"
  if (( lines > limit )) && ! is_allowed "$file"; then
    echo "超过 ${limit} 行：$file（${lines} 行）" >&2
    status=1
  fi
done < <(emit_files "$@")

if (( status == 0 )); then
  echo "文件行数检查通过（上限 ${limit} 行）"
fi
exit "$status"
