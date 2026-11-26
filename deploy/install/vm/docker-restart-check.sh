#!/bin/sh
# 在装好 Wonton 的主机上以 root 执行：重启 Docker 守护进程，断言原来在跑的 Compose 服务、核心自己建的入口/连接器
# 容器都回来、有健康检查的都健康，之后核心和它们都没有再重启、退出码是 0。不通过时打印每个容器的状态和 Docker 记下的
# HasBeenManuallyStopped，退出码 1。
#
#   docker-restart-check.sh [restart|stop-start]
#     restart     systemctl restart docker（默认）
#     stop-start  systemctl stop docker docker.socket，停 10 秒，再 start
#
# WONTON_PROJECT（默认 wonton）、WONTON_INSTANCE（几套核心共用一台 Docker 时按实例标签挑，容器信息正好是
# wonton-<实例>--*）、WONTON_CHECK_TIMEOUT（等服务回来的秒数，默认 360：Tuwunel 的 start_period 是 5 分钟）、
# WONTON_CHECK_SETTLE（回来后再看多少秒没人重启，默认 150：核心连不上 Matrix 时约 100 秒崩一次）。
set -eu

project=${WONTON_PROJECT:-wonton}
instance=${WONTON_INSTANCE:-}
timeout=${WONTON_CHECK_TIMEOUT:-360}
settle=${WONTON_CHECK_SETTLE:-150}
mode=${1:-restart}
label=com.docker.compose.project=$project

say() { echo "[$(date -u +%H:%M:%S)] $*"; }

# 要盯的容器：项目里原来在跑的服务（一次性的 hydra-migrate 不算），加上核心自己建的入口和连接器。
# 两样都用容器名，没了的时候也还叫得出来。
containers() {
  {
    docker ps --filter "label=$label" --format '{{.Names}}'
    docker ps --filter "label=wonton.managed-by=core" ${instance:+--filter "label=wonton.instance=$instance"} \
      --format '{{.Names}}'
  } | sort -u
}

# running，且有健康检查的已经 healthy。容器没了或起不来都返回 1。
ready() {
  docker inspect -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$1" 2>/dev/null |
    grep -Eq '^running (healthy|none)$'
}

# HasBeenManuallyStopped 只在守护进程写的 config.v2.json 里，API 不给；unless-stopped 的容器它为 true 时，守护进程
# 重启后不拉起。
report() {
  root=$(docker info -f '{{.DockerRootDir}}')
  for container in $containers; do
    id=$(docker inspect -f '{{.Id}}' "$container" 2>/dev/null) || true
    if [ -z "$id" ]; then
      echo "$container missing"
      continue
    fi
    manual=$(sed -n 's/.*"HasBeenManuallyStopped":\([a-z]*\).*/\1/p' "$root/containers/$id/config.v2.json" 2>/dev/null) || manual=missing
    docker inspect -f "$container {{.State.Status}} health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}} exit={{.State.ExitCode}} restarts={{.RestartCount}} started={{.State.StartedAt}} policy={{.HostConfig.RestartPolicy.Name}} stop_signal={{.Config.StopSignal}} manually_stopped=$manual" "$id" || echo "$container inspect 失败"
  done
}

fail() {
  say "不通过：$*"
  report
  exit 1
}

[ "$(id -u)" = 0 ] || { echo "要用 root 执行" >&2; exit 2; }
containers=$(containers)
[ -n "$containers" ] || fail "项目 $project 里没有在跑的容器"
for container in $containers; do
  ready "$container" || fail "重启前 $container 就没就绪"
done
say "重启前在跑：$(echo "$containers" | tr '\n' ' ')"
report

case "$mode" in
  restart) say "systemctl restart docker"; systemctl restart docker ;;
  stop-start)
    say "systemctl stop docker docker.socket"
    systemctl stop docker docker.socket
    sleep 10
    say "systemctl start docker"
    systemctl start docker
    ;;
  *) echo "用法：$0 [restart|stop-start]" >&2; exit 2 ;;
esac

deadline=$(($(date +%s) + timeout))
for container in $containers; do
  until ready "$container"; do
    [ "$(date +%s)" -lt "$deadline" ] || fail "$timeout 秒内 $container 没回到 running/healthy"
    sleep 2
  done
done
say "都回来了，再看 $settle 秒有没有人重启"

# 快照在都回来之后取：Docker 起容器时会重置 StartedAt，拿重启前的 StartedAt 比必然不等；之后它一变就是又重启过。
# RestartCount 同理会被守护进程清零，只看前后两份快照是否相同。
snapshot() {
  for container in $containers; do
    docker inspect -f '{{.Name}} {{.Id}} {{.State.Status}} {{.RestartCount}} {{.State.StartedAt}} {{.State.ExitCode}}' "$container" 2>/dev/null || echo "$container missing"
  done
}
recovered=$(snapshot)
sleep "$settle"
for container in $containers; do
  ready "$container" || fail "$container 回来后又掉了（退出码 $(docker inspect -f '{{.State.ExitCode}}' "$container" 2>/dev/null || echo missing)）"
done
now=$(snapshot)
[ "$recovered" = "$now" ] || fail "$settle 秒里有容器重启过（${recovered} → ${now}）"
for container in $containers; do
  exit_code=$(docker inspect -f '{{.State.ExitCode}}' "$container")
  [ "$exit_code" = 0 ] || fail "$container 退出码是 $exit_code"
done
say "通过"
report
