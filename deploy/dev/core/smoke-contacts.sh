#!/usr/bin/env bash
# 跨机 Bot 联系人（ADR-0050）的隔离端到端：借分享冒烟的夹具（smoke-sharing.sh：两台真云吞经 iroh 配对、各自的
# Tuwunel、隔离网段和测试 CA），两边各装一个回声 Bot，依次走：不是联系人时不理 → 请求、对方在“等你确认”里看到并同意
# → 两边的 Bot 互相私聊、点名对方 → 删除后又不理 → 请求被拒绝 → 请求后取消。要名额、要 Docker。
#
# 用法：deploy/dev/core/smoke-contacts.sh            （在仓库根目录跑）
# 依赖：
#   - 回声 Bot 的镜像 wonton-echo-bot:dev（见 plugins/examples/echo-bot/README.md）；
#   - 本检出的核心镜像，WONTON_SHARE_SMOKE_CORE_IMAGE 指定（docker build -f deploy/dev/core/Dockerfile --target dev）；
#   - WONTON_SHARE_SMOKE_BIN_DIR 里编好的三个二进制：
#     cargo build -p wonton-core --example fr_contacts_smoke --example fr_share_smoke
#     cargo build -p wonton-tunnel --example share-smoke-relay
#
# 两个回声 Bot 在同一个房间里会互相回声停不下来，所以私聊那两步一次只留收消息的那个 Bot 在跑：脚本停掉说话一方的
# 容器，测试用它的账号（新设备）说话，说完退出房间再把容器起回来。
set -euo pipefail

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  root="$(git rev-parse --show-toplevel 2>/dev/null)" || root="$(cd "$(dirname "$0")/../../.." && pwd)"
  [[ -n "$(docker image ls -q wonton-echo-bot:dev)" ]] || { echo "缺少镜像 wonton-echo-bot:dev" >&2; exit 2; }
  exec env SHARE_SMOKE_NAME=contacts-smoke SHARE_SMOKE_DRIVER=fr_contacts_smoke \
    SHARE_SMOKE_SCENARIO="$root/deploy/dev/core/smoke-contacts.sh" bash "$root/deploy/dev/core/smoke-sharing.sh"
fi

# 下面由 smoke-sharing.sh 在栈起来之后 source：compose 数组和 SHARE_SMOKE_* 都在。
step() {
  echo "== $1"
  SHARE_SMOKE_STEP="$1" "${compose[@]}" --profile driver run --rm -T driver
}

# 一台核心装的回声 Bot 的容器（o 是 owner.test，f 是 friend.test）。
echo_bot() {
  local id
  id="$(docker ps -aq --filter "label=wonton.instance=${SHARE_SMOKE_INSTANCE}$1" --filter name=echo | head -n 1)"
  [[ -n "$id" ]] || { echo "找不到 ${SHARE_SMOKE_INSTANCE}$1 的回声 Bot 容器" >&2; return 1; }
  echo "$id"
}

# Bot 的名单里有谁（核心渲染进容器的 WONTON_MATRIX_ALLOWED_USERS）。
allowed() {
  docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$(echo_bot "$1")" |
    grep '^WONTON_MATRIX_ALLOWED_USERS=' | sed 's/^WONTON_MATRIX_ALLOWED_USERS=/  名单 /'
}

# 名单变了核心会重建 Bot 的容器：等新容器里的名单带上（或去掉）对方。
wait_allowed() {
  local side="$1" user="$2" want="$3" i
  for i in $(seq 1 60); do
    if docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$(echo_bot "$side")" 2>/dev/null |
      grep '^WONTON_MATRIX_ALLOWED_USERS=' | grep -q -- "$user"; then
      [[ "$want" == yes ]] && { allowed "$side"; return 0; }
    else
      [[ "$want" == no ]] && { allowed "$side"; return 0; }
    fi
    sleep 1
  done
  echo "$side 的名单没有变成 $want $user" >&2
  return 1
}

healthy() {
  local i
  for i in $(seq 1 90); do
    [[ "$(docker inspect -f '{{.State.Health.Status}}' "$(echo_bot "$1")")" == healthy ]] && return 0
    sleep 1
  done
  echo "$1 的回声 Bot 没有恢复健康" >&2
  return 1
}

step setup
step ignored
step approve
wait_allowed o '@echo:friend.test' yes
wait_allowed f '@echo:owner.test' yes
healthy o
healthy f

docker stop "$(echo_bot o)" >/dev/null
step chat-a
docker stop "$(echo_bot f)" >/dev/null
docker start "$(echo_bot o)" >/dev/null
healthy o
step chat-b
docker start "$(echo_bot f)" >/dev/null
healthy f

step remove
wait_allowed o '@echo:friend.test' no
wait_allowed f '@echo:owner.test' no
healthy f
step ignored
step reject
step cancel
echo "PASS: cross-server bot contacts: request, approve, chat with mentions both ways, remove, reject, cancel"
