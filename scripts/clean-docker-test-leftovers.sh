#!/usr/bin/env bash
set -euo pipefail

# 作业被杀或客户端超时后，Docker 仍可能完成建容器；残留资源会撞网段、拖慢 Docker。
# 20 分钟和 PID 判断给同机其他作业留出时间，避免删掉仍在使用的资源。
min_age_minutes="${WONTON_LEFTOVER_MIN_AGE_MINUTES:-20}"
if [[ ! "$min_age_minutes" =~ ^[0-9]+$ ]]; then
  printf 'WONTON_LEFTOVER_MIN_AGE_MINUTES 必须是非负整数\n' >&2
  exit 2
fi
min_age_seconds=$((min_age_minutes * 60))
now="$(date +%s)"
# docker_network.rs 的资源前缀是 `wonton-nettest-<pid>-<test>`（容器名 `…--core`、核心自建网络 `…_core`），
# 其余是老前缀：pintest/gwtest 是这个测试改名前的，gosttest、tusdtest 和 lefta/b/c 分别是
# docker_egress、tusd 和 docker_leftovers 的。都按 PID 判断：进程还活着的不动，死了超过 20 分钟才清。
pattern='^wonton-(pintest|gwtest|gosttest|tusdtest|left[abc])([0-9]+)(-|$)'
nettest_pattern='^wonton-nettest-([0-9]+)(-|$)'
# 改名前的旧核心网络（`wonton-gwtest<pid>_<…>_core`）PID 后是 `_`，两条通用正则都够不着，单独列出；
# 上一代 CI 的残留清完即可删这条。
old_core_network_pattern='^wonton-gwtest([0-9]+)_core$'
stale() {
  local name="$1" kind="$2" pid created created_at
  if [[ "$name" =~ $pattern ]]; then
    pid="${BASH_REMATCH[2]}"
  elif [[ "$name" =~ $nettest_pattern ]]; then
    pid="${BASH_REMATCH[1]}"
  elif [[ "$kind" == network && "$name" =~ $old_core_network_pattern ]]; then
    pid="${BASH_REMATCH[1]}"
  else
    return 1
  fi
  [ -d "/proc/$pid" ] && return 1
  if ! created="$(docker "$kind" inspect --format '{{.Created}}' "$name" 2>/dev/null)"; then
    printf '检查资源失败，跳过：%s\n' "$name" >&2
    return 1
  fi
  if ! created_at="$(date -d "${created% *}" +%s 2>/dev/null)"; then
    printf '解析创建时间失败，跳过：%s\n' "$name" >&2
    return 1
  fi
  (( now - created_at > min_age_seconds ))
}

remove_testcontainer_batch() {
  local -a batch=("$@")
  local name
  ((${#batch[@]} > 0)) || return 0
  if docker rm "${batch[@]}" >/dev/null 2>&1; then
    containers_removed=$((containers_removed + ${#batch[@]}))
  else
    for name in "${batch[@]}"; do
      if docker rm "$name" >/dev/null 2>&1; then
        containers_removed=$((containers_removed + 1))
      else
        printf '删除 testcontainers 遗留失败，继续：%s\n' "$name" >&2
      fi
    done
  fi
}

containers_removed=0
testcontainer_batch=()
testcontainers="$(docker ps -a \
  --filter label=org.testcontainers.managed-by=testcontainers \
  --filter status=created \
  --format '{{.Names}}\t{{.CreatedAt}}')"
while IFS=$'\t' read -r name created; do
  [[ -n "$name" ]] || continue
  if ! created_at="$(date -d "${created% *}" +%s 2>/dev/null)"; then
    printf '解析 testcontainers 创建时间失败，跳过：%s\n' "$name" >&2
    continue
  fi
  (( now - created_at > min_age_seconds )) || continue
  testcontainer_batch+=("$name")
  if ((${#testcontainer_batch[@]} == 50)); then
    remove_testcontainer_batch "${testcontainer_batch[@]}"
    testcontainer_batch=()
  fi
done <<<"$testcontainers"
remove_testcontainer_batch "${testcontainer_batch[@]}"

containers="$(docker ps -a --format '{{.Names}}')"
while IFS= read -r name; do
  [[ -n "$name" ]] || continue
  stale "$name" container || continue
  if docker rm -f "$name" >/dev/null 2>&1; then
    containers_removed=$((containers_removed + 1))
  else
    printf '删除容器失败，继续：%s\n' "$name" >&2
  fi
done <<<"$containers"

networks_removed=0
networks="$(docker network ls --format '{{.Name}}')"
while IFS= read -r name; do
  [[ -n "$name" ]] || continue
  stale "$name" network || continue
  if docker network rm "$name" >/dev/null 2>&1; then
    networks_removed=$((networks_removed + 1))
  else
    printf '删除网络失败，继续：%s\n' "$name" >&2
  fi
done <<<"$networks"

# registry_mirror 的测试推到一次性仓库再拉回来的镜像（`127.0.0.1:<端口>/<实例名>/fixture`，没有标签）不随测试删掉，
# 一台 runner 上攒了 500 多个。实例名是 `wt<十六进制 PID><随机数>`，拆不出 PID，只按建好的时间判断；有容器在用的
# `docker rmi` 不加 -f 会拒绝，不动。
fixture_pattern='^127\.0\.0\.1:[0-9]+/wt[0-9a-f]+/fixture$'
images_removed=0
images="$(docker images --no-trunc --format '{{.ID}}|{{.Repository}}|{{.CreatedAt}}')"
while IFS='|' read -r id repo created; do
  [[ "$repo" =~ $fixture_pattern ]] || continue
  if ! created_at="$(date -d "${created% *}" +%s 2>/dev/null)"; then
    printf '解析镜像创建时间失败，跳过：%s\n' "$repo" >&2
    continue
  fi
  (( now - created_at > min_age_seconds )) || continue
  if docker rmi "$id" >/dev/null 2>&1; then
    images_removed=$((images_removed + 1))
  else
    printf '删除测试镜像失败，继续：%s\n' "$repo" >&2
  fi
done <<<"$images"

printf '清理完成：删除容器 %d 个，网络 %d 个，测试镜像 %d 个\n' "$containers_removed" "$networks_removed" "$images_removed"
