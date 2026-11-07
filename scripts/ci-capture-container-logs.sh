#!/usr/bin/env bash
# 跟着 Docker 测试起的容器记日志：每个容器一份 <目录>/<容器名>-<短 ID>.log，带时间戳。测试结束时容器会被删掉，
# 失败的现场只剩这里（rust.yml 失败时把目录作为 artifact 上传；重跑另起一份，覆盖不掉）。
# 只记 testcontainers 起的（tuwunel 等）和测试按名字建的 wonton-*（核心、插件）：同一台机器上主人的服务不记。
#
#   scripts/ci-capture-container-logs.sh <目录>     一直跑到被杀（作业结束时 runner 会收掉）
set -uo pipefail
dir="${1:?用法：$0 <目录>}"
mkdir -p "$dir"
docker events --filter type=container --filter event=start \
  --format '{{.Actor.ID}} {{.Actor.Attributes.name}} {{index .Actor.Attributes "org.testcontainers.managed-by"}}' |
  while read -r id name managed; do
    [[ $managed == testcontainers || $name == wonton-* ]] || continue
    docker logs -f -t "$id" >"$dir/$name-${id:0:12}.log" 2>&1 &
  done
