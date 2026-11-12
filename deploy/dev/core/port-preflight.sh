#!/usr/bin/env bash
# dev.sh up 在构建之前查宿主端口：网关 TCP 和隧道 UDP。compose 要到绑定端口时才报冲突（“Bind for 0.0.0.0:18682
# failed”），那时核心镜像已经构建完，排队加构建白等一轮。
# 被占 = 别的 compose 项目在跑的容器发布了这个端口，或本机任何地址（0.0.0.0、127.0.0.1、::）上已经有 socket。
# 本项目自己的容器发布的不算：栈在跑时再 up 照常。DOCKER_HOST 指向远端时只查 Docker 发布的端口，本机 socket 和那台无关。
# 用法：port-preflight.sh <compose 项目> <网关 TCP 端口> <隧道 UDP 端口>
# 有冲突时逐个打印哪个端口、谁占着、往后第一个空的端口，退出 1。
set -euo pipefail

project=${1:?compose 项目} gw=${2:?网关端口} tunnel=${3:?隧道端口}

local_docker() {
  case "${DOCKER_HOST:-}" in
    ssh://* | tcp://*) return 1 ;;
  esac
}

# 本机在这个端口上的 socket（任何地址）。
host_socket() { # <端口> <tcp|udp>
  local p=$1 proto=$2
  if command -v ss >/dev/null; then
    if [[ $proto == tcp ]]; then ss -Hltn "sport = :$p"; else ss -Hlun "sport = :$p"; fi
  elif [[ $proto == tcp ]]; then
    lsof -nP -iTCP:"$p" -sTCP:LISTEN
  else
    lsof -nP -iUDP:"$p"
  fi
}

# 谁占着这个端口；空着返回 1。
holder() { # <端口> <tcp|udp>
  local p=$1 proto=$2 line
  line=$(docker ps --format '{{.Label "com.docker.compose.project"}}|{{.Names}}|{{.Ports}}' |
    grep -E "[:]$p->[0-9]+/$proto" | head -1 || true)
  if [[ -n $line ]]; then
    [[ ${line%%|*} == "$project" ]] && return 1
    line=${line#*|}
    echo "容器 ${line%%|*}"
    return 0
  fi
  local_docker && (( can_check_host )) || return 1
  line=$(host_socket "$p" "$proto" | head -1 || true)
  [[ -n $line ]] || return 1
  echo "本机已有 socket：$line"
}

# ss 和 lsof 都没有时只查了 Docker 发布的端口，如实说一句，不拦。
can_check_host=1
if local_docker && ! command -v ss >/dev/null && ! command -v lsof >/dev/null; then
  can_check_host=0
  echo "注意：没有 ss 也没有 lsof，起栈前只查了 Docker 发布的端口，本机别的进程占着端口要到绑定时才知道" >&2
fi

status=0
check() { # <端口> <tcp|udp> <环境变量>
  local p=$1 proto=$2 var=$3 who next hint
  hint="往后 20 个也都被占，换个远一点的 $var"
  who=$(holder "$p" "$proto") || return 0
  for next in $(seq $((p + 1)) $((p + 20))); do
    holder "$next" "$proto" >/dev/null || { hint="换一个，比如 $var=$next"; break; }
  done
  echo "端口 $p/$proto 被占：$who。$hint" >&2
  status=1
}
check "$gw" tcp WONTON_DEV_PORT
check "$tunnel" udp WONTON_DEV_TUNNEL_PORT
exit "$status"
