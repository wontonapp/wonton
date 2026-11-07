#!/usr/bin/env bash
# crates/ 和测试 docker build 的 Dockerfile 里写到的镜像引用，都要列在 deploy/ci/docker-test-images.txt：
# 要拉的照写，测试数据前面加 `!`。没有 Docker Hub 的 CI 机器只按那份清单预先拿到镜像，漏写的 Docker 测试在那里会拉取失败。
#
#   scripts/check-docker-test-images.sh          检查
#   scripts/check-docker-test-images.sh --pull   只打印要拉的镜像（推镜像的脚本用）
#
# 按文本找引用：带注册表主机名的、按摘要固定的、`IMAGE: &str` 常量里的 Docker Hub 镜像。
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
list=deploy/ci/docker-test-images.txt

entries() { grep -vE '^\s*(#|$)' "$list"; }
if [[ ${1:-} == --pull ]]; then
  entries | grep -v '^!'
  exit 0
fi

ref='[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*'
found=$({
  grep -rhoE "(public\.ecr\.aws|ghcr\.io|quay\.io|docker\.io)/$ref(:[A-Za-z0-9._-]+)?(@sha256:[0-9a-f]{64})?" crates
  grep -hoE "^FROM( --platform=[^ ]+)? [^ ]+" deploy/dev/acp-bridge/Dockerfile deploy/dev/core/Dockerfile | awk '{print $NF}' | grep /
  grep -rhoE "\"$ref:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}\"" crates | tr -d '"'
  grep -rhE 'IMAGE: &str = "' crates | grep -oE "\"$ref:[A-Za-z0-9._-]+\"" | tr -d '"'
} | grep -E ':|@' | sort -u)

known=$(entries | sed 's/^!//' | sort -u)
missing=$(comm -23 <(printf '%s\n' "$found") <(printf '%s\n' "$known"))
if [[ -n $missing ]]; then
  echo "这些镜像引用不在 $list 里（测试会拉的照写，只是测试数据的前面加 !）：" >&2
  printf '  %s\n' $missing >&2
  exit 1
fi
