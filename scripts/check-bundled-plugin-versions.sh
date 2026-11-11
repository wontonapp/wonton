#!/usr/bin/env bash
# 随核心镜像带的插件包（plugins/<名>/，见 deploy/dev/core/Dockerfile）改了文件，清单的 x-wonton.version 必须跟着变。
# 核心按版本号决定要不要升已经装着的系统插件，同号不升；旧记录里的清单新核心又可能读不懂（magpie 两份不同的 0.6.0，
# 旧的缺了后来必填的 providers.test，装着旧版的机器上模型网关读不懂、也升不上去）。
#
#   scripts/check-bundled-plugin-versions.sh [基准提交]
#
# 基准默认是和 origin/main 的分叉点；就在 main 上时和上一个提交比。
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

base="${1:-}"
if [[ -z $base ]]; then
  base="$(git merge-base HEAD origin/main)"
  [[ $base != "$(git rev-parse HEAD)" ]] || base="$(git rev-parse HEAD^)"
fi

# 只认 x-wonton 下面那一级的 version，服务里的同名字段不算。
version() { awk '/^x-wonton:/ { in_x = 1; next } in_x && /^[^ #]/ { exit } in_x && /^  version:/ { print $2; exit }'; }

failed=0
for compose in plugins/*/docker-compose.yml; do
  dir="${compose%/docker-compose.yml}"
  git diff --quiet "$base" -- "$dir" && continue
  git cat-file -e "$base:$compose" 2>/dev/null || continue
  old="$(git show "$base:$compose" | version)"
  new="$(version <"$compose")"
  if [[ $old == "$new" ]]; then
    echo "$dir 的文件改了，x-wonton.version 还是 $new：升一下版本号，装着旧版的机器才会升级" >&2
    failed=1
  fi
done
exit "$failed"
