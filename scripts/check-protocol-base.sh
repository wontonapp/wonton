#!/usr/bin/env bash
# 改了 crates/wonton-protocol 的分支，main 在分支切出（merge-base）之后又改过协议，就失败：两边各自按旧协议写，
# 合在一起才坏，PR 上的 CI 看不出来。没改协议的分支直接通过。
# 用法：check-protocol-base.sh <base 分支名> <分支头提交>；在 CI 的浅克隆里会把两边的历史补齐（不拉文件内容）。
set -euo pipefail

base=${1:?用法：check-protocol-base.sh <base 分支名> <分支头提交>}
head=${2:?用法：check-protocol-base.sh <base 分支名> <分支头提交>}
path=crates/wonton-protocol

fetch=(git fetch --quiet --no-tags --filter=blob:none)
if [[ $(git rev-parse --is-shallow-repository) == true ]]; then
  fetch+=(--unshallow)
fi
"${fetch[@]}" origin "+refs/heads/$base:refs/remotes/origin/$base" "$head"

mb=$(git merge-base "$head" "origin/$base")
if git diff --quiet "$mb" "$head" -- "$path"; then
  echo "没改 $path，不用查。"
  exit 0
fi
moved=$(git log --format='%h %s' "$mb..origin/$base" -- "$path")
if [[ -n $moved ]]; then
  echo "::error::先同步 main（协议在你分支之后变过）：git fetch origin && git rebase origin/$base，解完冲突再推。"
  echo "分支切出之后 $base 上改过 $path 的提交："
  echo "$moved"
  exit 1
fi
echo "$path 的改动基于最新的 $base。"
