#!/bin/sh
# 在装好 Wonton 的主机上以 root 执行：重启 Docker 守护进程，断言 Compose 项目里原来在跑的服务都回来、有健康检查的
# 都健康，核心之后不再重启。不通过时打印每个服务的状态和 Docker 记下的 HasBeenManuallyStopped，退出码 1。
#
#   docker-restart-check.sh [restart|stop-start]
#     restart     systemctl restart docker（默认）
#     stop-start  systemctl stop docker docker.socket，停 10 秒，再 start
#
# WONTON_PROJECT（默认 wonton）、WONTON_CHECK_TIMEOUT（等服务回来的秒数，默认 360：Tuwunel 的 start_period 是 5 分钟）、
# WONTON_CHECK_SETTLE（回来后再看多少秒核心没有重启，默认 150：核心连不上 Matrix 时约 100 秒崩一次）。
set -eu

project=${WONTON_PROJECT:-wonton}
timeout=${WONTON_CHECK_TIMEOUT:-360}
settle=${WONTON_CHECK_SETTLE:-150}
mode=${1:-restart}
label=com.docker.compose.project=$project

say() { echo "[$(date -u +%H:%M:%S)] $*"; }

# 原来在跑的服务：一次性的（hydra-migrate）不算。
running_services() {
  docker ps --filter "label=$label" --format '{{.Label "com.docker.compose.service"}}' | sort
}

container_of() {
  docker ps -aq --filter "label=$label" --filter "label=com.docker.compose.service=$1" | head -n 1
}

# running，且有健康检查的已经 healthy。
ready() {
  docker inspect -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$1" 2>/dev/null |
    grep -Eq '^running (healthy|none)$'
}

# HasBeenManuallyStopped 只在守护进程写的 config.v2.json 里，API 不给；unless-stopped 的容器它为 true 时，守护进程
# 重启后不拉起。
report() {
  root=$(docker info -f '{{.DockerRootDir}}')
  for service in $services; do
    id=$(docker inspect -f '{{.Id}}' "$(container_of "$service")")
    manual=$(sed -n 's/.*"HasBeenManuallyStopped":\([a-z]*\).*/\1/p' "$root/containers/$id/config.v2.json")
    docker inspect -f "$service {{.State.Status}} health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}} exit={{.State.ExitCode}} started={{.State.StartedAt}} finished={{.State.FinishedAt}} restarts={{.RestartCount}} policy={{.HostConfig.RestartPolicy.Name}} stop_signal={{.Config.StopSignal}} manually_stopped=$manual" "$id"
  done
}

fail() {
  say "不通过：$*"
  report
  exit 1
}

[ "$(id -u)" = 0 ] || { echo "要用 root 执行" >&2; exit 2; }
services=$(running_services)
[ -n "$services" ] || fail "Compose 项目 $project 里没有在跑的服务"
for service in $services; do
  ready "$(container_of "$service")" || fail "重启前 $service 就没就绪"
done
say "重启前在跑：$(echo "$services" | tr '\n' ' ')"
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
for service in $services; do
  until ready "$(container_of "$service")"; do
    [ "$(date +%s)" -lt "$deadline" ] || fail "$timeout 秒内 $service 没回到 running/healthy"
    sleep 2
  done
done
say "都回来了，再看 $settle 秒核心有没有重启"

core=$(container_of core)
before=$(docker inspect -f '{{.RestartCount}} {{.State.StartedAt}}' "$core")
sleep "$settle"
after=$(docker inspect -f '{{.RestartCount}} {{.State.StartedAt}}' "$core")
[ "$before" = "$after" ] || fail "核心在 $settle 秒里重启了：$before → $after"
for service in $services; do
  ready "$(container_of "$service")" || fail "$service 回来后又掉了"
done
say "通过"
report
