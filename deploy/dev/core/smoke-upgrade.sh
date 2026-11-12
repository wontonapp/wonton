#!/usr/bin/env bash
# 核心一键升级的端到端冒烟（ADR-0025）：本机一个临时仓库当渠道，用 install.sh 装旧版本 → 发新版本，经接口检查、升级，
# 核心被换掉后接着报告 → 发健康检查不过的版本，升级后自动回滚 → 用 WONTON_CORE_PIN 手动退回到最早的版本。
#
#   deploy/dev/core/smoke-upgrade.sh
#
# 在要测的主机上直接执行，不用 root：install.sh 跑在一个 docker CLI 容器里，那里面是 root。compose 项目名是常量，
# 所以这台主机上不能有真装的云吞（有就停下）。结束时清理全部容器、卷、镜像和目录；KEEP=1 保留现场。
# 本机需要 bash、docker、python3、curl、script。CORE_BASE=<镜像> 跳过构建，用现成的核心镜像。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
prefix=core-upgrade-smoke
port="${SMOKE_PORT:-15480}"
tunnel_port="${SMOKE_TUNNEL_PORT:-15482}"
registry_port="${SMOKE_REGISTRY_PORT:-15055}"
# 127.0.0.0/8 的仓库不用 TLS，Docker 默认放行。
channel="127.0.0.1:$registry_port/wonton-core"
domain=wonton.localhost
registry_image=public.ecr.aws/docker/library/registry:2
cli_image=public.ecr.aws/docker/library/docker:29.9.2-cli
work="$(mktemp -d "${TMPDIR:-/tmp}/$prefix-XXXXXX")"
install_dir="$work/install"
data_dir="$work/data"

ok() { echo "ok：$*"; }
fail() {
  echo "失败：$*" >&2
  exit 1
}
expect() {
  if [[ "$2" == "$3" ]]; then ok "$1"; else fail "$1：期望 $3，实际 $2"; fi
}
json() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value.get(key) if isinstance(value, dict) else None
print("" if value is None else value)' "$1"
}

cleanup() {
  local code=$?
  if [[ -n "${KEEP:-}" ]]; then
    echo "保留现场：$work，项目 wonton，仓库容器 $prefix-registry"
    return "$code"
  fi
  docker update --restart=no wonton-updater >/dev/null 2>&1 || true
  docker rm -f wonton-updater >/dev/null 2>&1 || true
  if [[ -f "$install_dir/compose.yaml" ]]; then
    docker run --rm --network none -v /var/run/docker.sock:/var/run/docker.sock -v "$install_dir:$install_dir" \
      --entrypoint docker-compose "$channel:one" -f "$install_dir/compose.yaml" down -v -t 3 >/dev/null 2>&1 || true
  fi
  docker rm -f "$prefix-registry" "$prefix-install" >/dev/null 2>&1 || true
  [[ -n "$egress_network_existed" ]] || docker network rm wonton-egress >/dev/null 2>&1 || true
  docker images --format '{{.Repository}}:{{.Tag}}' | grep -E "^($prefix|127\.0\.0\.1:$registry_port)" |
    xargs -r docker rmi -f >/dev/null 2>&1 || true
  docker run --rm -v "$(dirname "$work"):/parent" --entrypoint rm "$cli_image" -rf "/parent/$(basename "$work")" \
    >/dev/null 2>&1 || true
  return "$code"
}
egress_network_existed="$(docker network ls -q --filter name=^wonton-egress$)"
trap cleanup EXIT

if [[ -n "$(docker ps -aq --filter label=com.docker.compose.project=wonton)" ]] ||
  docker inspect wonton-updater >/dev/null 2>&1; then
  fail "这台主机上有项目名是 wonton 的容器或者 wonton-updater，冒烟会碰到它们，换一台主机"
fi

base="${CORE_BASE:-}"
if [[ -z "$base" ]]; then
  base="$prefix-base:latest"
  echo "==> 构建核心镜像"
  nice -n 19 docker buildx build --load --target core -t "$base" -f "$repo/deploy/dev/core/Dockerfile" \
    --build-arg CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" "$repo" >/dev/null
fi

docker run -d --name "$prefix-registry" -p "127.0.0.1:$registry_port:5000" "$registry_image" >/dev/null

# 不带插件：冒烟只看核心换版本，不要下载 Hermes。bad 的健康检查永远不过，别的子命令照常。
context="$work/context"
mkdir -p "$context"
cat >"$context/bad-core" <<'EOF'
#!/bin/sh
[ "$1" != healthcheck ] || exit 1
exec /usr/local/bin/real-core "$@"
EOF
chmod 755 "$context/bad-core"
publish() {
  local tag="$1" version="$2" extra="${3:-}"
  docker build -q -t "$channel:$tag" --label "org.opencontainers.image.version=$version" -f - "$context" >/dev/null <<EOF
FROM $base
RUN rm -rf /usr/share/wonton/system-plugins/* /usr/share/wonton/default-plugins/*
$extra
EOF
  docker push -q "$channel:$tag" >/dev/null
  # 渠道就是 stable 这个 tag。
  docker tag "$channel:$tag" "$channel:stable"
  docker push -q "$channel:stable" >/dev/null
  docker image inspect --format '{{index .RepoDigests 0}}' "$channel:$tag" | sed 's/.*@//'
}

# install.sh 最后一直显示配对码等手机，不会自己退出：看到“装好了”就停掉它。
wait_installed() {
  for _ in $(seq 1 240); do
    if docker logs "$prefix-install" 2>&1 | grep -q 装好了; then
      docker rm -f "$prefix-install" >/dev/null
      return 0
    fi
    [[ "$(docker inspect --format '{{.State.Running}}' "$prefix-install" 2>/dev/null)" == true ]] || break
    sleep 3
  done
  docker logs "$prefix-install" 2>&1 | tail -30
  fail "install.sh 没有装好"
}

run_install() {
  docker run -d --name "$prefix-install" "$@" -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$install_dir:$install_dir" -v "$data_dir:$data_dir" -v "$repo/deploy/install/install.sh:/install.sh:ro" \
    -e WONTON_INSTALL_DIR="$install_dir" -e WONTON_DATA_DIR="$data_dir" -e WONTON_PORT="$port" \
    -e WONTON_TUNNEL_PORT="$tunnel_port" -e WONTON_BASE_DOMAIN="$domain" "$cli_image" sh /install.sh >/dev/null
  wait_installed
}

# 安装目录和数据目录是 root 的。
root_read() {
  docker run --rm -v "$work:$work" --entrypoint cat "$cli_image" "$1"
}

request() {
  local method="$1" path="$2"
  shift 2
  curl -sS --noproxy '*' --max-time 20 --resolve "$domain:$port:127.0.0.1" -X "$method" \
    -H "Authorization: Bearer $token" "$@" "http://$domain:$port/_wonton/api/core/upgrade$path"
}
api() {
  local method="$1"
  shift
  request "$method" "" "$@"
}
check() {
  request POST /check --fail-with-body -H 'Content-Type: application/json' -d '{"manual":true}' >/dev/null
}

# 等到检查结果是这个 digest 可以升级。
wait_offer() {
  local digest="$1" state
  for _ in $(seq 1 60); do
    check
    state="$(api GET | json check.offer.digest)" || state=
    [[ "$state" == "$digest" ]] && return 0
    sleep 3
  done
  fail "等不到新版本 $digest：$(api GET)"
}

# 发起升级，等到结果。核心断开时连不上是正常的。打印经过的阶段。
upgrade() {
  local digest="$1" reply state phase last=''
  reply="$(api POST -H 'Content-Type: application/json' -d "{\"digest\":\"$digest\"}" -w '\n%{http_code}')"
  expect "发起升级回 202" "${reply##*$'\n'}" 202 >&2
  for _ in $(seq 1 400); do
    if reply="$(api GET 2>/dev/null)" && state="$(json run.state.state <<<"$reply" 2>/dev/null)"; then
      phase="$(json run.state.phase <<<"$reply")"
      if [[ "$state/$phase" != "$last" ]]; then
        echo "    $state $phase" >&2
        last="$state/$phase"
      fi
      [[ -z "$state" || "$state" == in_progress ]] || {
        printf '%s' "$reply"
        return 0
      }
    elif [[ "$last" != 连不上 ]]; then
      echo "    连不上核心" >&2
      last=连不上
    fi
    sleep 3
  done
  fail "升级 20 分钟没有结果：$(docker logs --tail 30 wonton-updater 2>&1)"
}

env_value() { root_read "$install_dir/.env" | sed -n "s/^$1=//p"; }
env_image() { env_value WONTON_CORE_IMAGE; }

mkdir -p "$install_dir" "$data_dir"
one="$(publish one 0.9.0-smoke.1)"
echo "==> install.sh 装 0.9.0-smoke.1"
run_install -e WONTON_CORE_IMAGE="$channel:stable"
token="$(root_read "$data_dir/core/owner-token")"
status="$(api GET)"
expect "当前版本" "$(json current.version <<<"$status")" 0.9.0-smoke.1
expect "当前 digest" "$(json current.digest <<<"$status")" "$one"
expect ".env 按 digest 写" "$(env_image)" "$channel@$one"

two="$(publish two 0.9.0-smoke.2)"
echo "==> 发布 0.9.0-smoke.2，经接口检查、升级"
wait_offer "$two"
result="$(upgrade "$two")"
[[ "$(json run.state.state <<<"$result")" == completed ]] || {
  docker logs --tail 30 wonton-updater 2>&1
  fail "升级没有完成：$result"
}
ok "升级完成"
status="$(api GET)"
expect "新核心报告新版本" "$(json current.digest <<<"$status")" "$two"
expect ".env 换了镜像" "$(env_image)" "$channel@$two"
expect "升级器不会再被拉起" "$(docker inspect --format '{{.HostConfig.RestartPolicy.Name}} {{.State.Running}}' wonton-updater)" "no false"
# 检查是异步的：发起后轮询到它出结果为止（不加固定 sleep）。
check_state() {
  local state
  for _ in $(seq 30); do
    check
    state="$(json check.state <<<"$(api GET)" 2>/dev/null || true)"
    [[ "$state" == up_to_date ]] && { echo up_to_date; return 0; }
    sleep 2
  done
  echo "${state:-无结果}"
}
expect "检查结果是最新" "$(check_state)" up_to_date
root_read "$data_dir/core/upgrade/backup/$one/.complete" >/dev/null || fail "没有 $one 的备份"
ok "留下了旧版本的备份"

bad_extra='RUN mv /usr/local/bin/wonton-core /usr/local/bin/real-core
COPY bad-core /usr/local/bin/wonton-core'
bad="$(publish bad 0.9.0-smoke.3 "$bad_extra")"
echo "==> 发布健康检查不过的 0.9.0-smoke.3，升级后要自动回滚"
wait_offer "$bad"
result="$(upgrade "$bad")"
[[ "$(json run.state.state <<<"$result")" == rolled_back ]] || {
  docker logs --tail 30 wonton-updater 2>&1
  fail "没有回滚：$result"
}
ok "结果是已回滚"
expect "原因是不健康" "$(json run.state.problem.reason <<<"$result")" unhealthy
status="$(api GET)"
expect "回到回滚前的版本" "$(json current.digest <<<"$status")" "$two"
expect ".env 也回去了" "$(env_image)" "$channel@$two"
expect "核心健康" "$(docker inspect --format '{{.State.Health.Status}}' wonton-core)" healthy

echo "==> WONTON_CORE_PIN 手动退回到 0.9.0-smoke.1"
# install.sh 从 /dev/tty 读确认，要一个真的终端。
pin_command="docker run --name $prefix-install -it -v /var/run/docker.sock:/var/run/docker.sock -v $install_dir:$install_dir \
  -v $data_dir:$data_dir -v $repo/deploy/install/install.sh:/install.sh:ro -e WONTON_INSTALL_DIR=$install_dir \
  -e WONTON_DATA_DIR=$data_dir -e WONTON_CORE_PIN=$one $cli_image sh /install.sh"
printf 'n\n' | script -qfec "$pin_command" /dev/null >/dev/null 2>&1 && fail "没有确认也退回了"
docker rm -f "$prefix-install" >/dev/null
expect "没确认就什么都没动" "$(json current.digest <<<"$(api GET)")" "$two"
printf 'y\n' | script -qfec "$pin_command" /dev/null >/dev/null 2>&1 &
sleep 3
wait_installed
status="$(api GET)"
expect "退回到最早的版本" "$(json current.digest <<<"$status")" "$one"
expect ".env 按旧 digest 写" "$(env_image)" "$channel@$one"
expect "还跟着渠道" "$(env_value WONTON_CORE_SOURCE)" "$channel:stable"
expect "App 不再显示旧的结果" "$(json run <<<"$status")" ""

echo "全部通过"
