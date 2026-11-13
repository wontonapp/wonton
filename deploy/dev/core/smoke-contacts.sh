#!/usr/bin/env bash
# 跨机 Bot 联系人（ADR-0050）的隔离端到端：借分享冒烟的夹具（smoke-sharing.sh：两台真云吞经 iroh 配对、各自的
# Tuwunel、隔离网段和测试 CA），两边各装一个回声 Bot，依次走：不是联系人时不理 → 请求、对方在“等你确认”里看到并同意
# → 两边的 Bot 互相私聊、点名对方 → A 建群拉 B 的人和两边的 Bot、B 的人点名 Bot 得回 → B 整台停着时 A 在群里发的消息
# 在 B 回来后补到 → 删除后又不理 → 请求被拒绝 → 请求后取消。要名额、要 Docker。
#
# 夹具里的 Tuwunel 不经核心的 iroh 转发联邦（那要正式安装渲染的联邦配置，见 entry-cloud 的冒烟），而是信任测试 CA、
# 经夹具的 TLS 前门互相联邦（share-smoke/contacts.yaml）。配对、对方在不在线由 iroh 给出，Matrix 事件和私聊走前门。
# 别的栈的插件网络占了按名额算的地址段时，设 WONTON_SHARE_SMOKE_PLUGIN_POOL 换一段私有 /24。
#
# 用法：deploy/dev/core/smoke-contacts.sh            （在仓库根目录跑）
# 依赖：
#   - 回声 Bot 的镜像 wonton-echo-bot:dev（见 plugins/examples/echo-bot/README.md）；
#   - 本检出的核心镜像，WONTON_SHARE_SMOKE_CORE_IMAGE 指定（docker build -f deploy/dev/core/Dockerfile --target dev）；
#   - WONTON_SHARE_SMOKE_BIN_DIR 里编好的三个二进制：
#     cargo build -p wonton-core --example fr_contacts_smoke --example fr_share_smoke
#     cargo build -p wonton-tunnel --example share-smoke-relay
#
# 两个回声 Bot 在同一个房间里会互相回声停不下来，所以私聊和群聊一次只留一个 Bot 在跑：脚本停掉说话一方的
# 容器，测试用它的账号（新设备）说话，说完退出房间（群聊里不退出）再把容器起回来。
set -euo pipefail

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  root="$(git rev-parse --show-toplevel 2>/dev/null)" || root="$(cd "$(dirname "$0")/../../.." && pwd)"
  [[ -n "$(docker image ls -q wonton-echo-bot:dev)" ]] || { echo "缺少镜像 wonton-echo-bot:dev" >&2; exit 2; }
  exec env SHARE_SMOKE_NAME=contacts-smoke SHARE_SMOKE_DRIVER=fr_contacts_smoke \
    SHARE_SMOKE_COMPOSE_EXTRA="$root/deploy/dev/core/share-smoke/contacts.yaml" \
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
  local side="$1" user="$2" want="$3" env
  for _ in $(seq 1 60); do
    # 重建时有一会儿没有容器：读不到就接着等，不然“要没有”会在容器不在时误判成已经好了。
    if env="$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$(echo_bot "$side" 2>/dev/null)" 2>/dev/null)"; then
      if grep '^WONTON_MATRIX_ALLOWED_USERS=' <<<"$env" | grep -q -- "$user"; then
        [[ "$want" == yes ]] && { allowed "$side" || true; return 0; }
      else
        [[ "$want" == no ]] && { allowed "$side" || true; return 0; }
      fi
    fi
    sleep 1
  done
  echo "$side 的名单没有变成 $want $user" >&2
  return 1
}

healthy() {
  for _ in $(seq 1 90); do
    [[ "$(docker inspect -f '{{.State.Health.Status}}' "$(echo_bot "$1")")" == healthy ]] && return 0
    sleep 1
  done
  echo "$1 的回声 Bot 没有恢复健康" >&2
  return 1
}

# 等 B 栈的一个服务恢复健康（stop 之后 start 回来用）。
stack_healthy() {
  local service="$1" id
  for _ in $(seq 1 120); do
    id="$("${compose[@]}" ps -aq "$service" | head -n 1)"
    [[ -n "$id" ]] && [[ "$(docker inspect -f '{{.State.Health.Status}}' "$id")" == healthy ]] && return 0
    sleep 1
  done
  echo "B 栈的 $service 没有恢复健康" >&2
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

# 群聊：两边的人先和对方的 Bot 结成联系人，A 建群拉 B 的人和两边的 Bot（都在线，还没有消息，不会互回）。
step group-contacts
wait_allowed o '@smokeowner:friend.test' yes
wait_allowed f '@smokeowner:owner.test' yes
healthy o
healthy f
step group-room
step group-join

# 回话的留 A 的 Bot：停掉 B 的，B 的人点名 A 的 Bot，回声从 A 那边回过来。
docker stop "$(echo_bot f)" >/dev/null
step group-reply

# B 整台停着，A 的人在群里说话；B 的人回来后那句补到（A 的 Bot 开着，它自己的回声也一起补）。
"${compose[@]}" stop core-friend tuwunel-friend
docker stop "$(echo_bot f)" >/dev/null
step group-offline
"${compose[@]}" start core-friend tuwunel-friend
stack_healthy core-friend
stack_healthy tuwunel-friend
step group-backfill

# 两个都起回来：B 的第一次同步吃掉积下的事件，之后才起 A 的，两边不会一起开回声循环。名单变化时停着的 Bot
# 不会重建（要等下次启动才带上新名单），所以删联系人之前两个都得起着。
docker start "$(echo_bot f)" >/dev/null
healthy f
docker start "$(echo_bot o)" >/dev/null
healthy o
step remove
wait_allowed o '@echo:friend.test' no
wait_allowed f '@echo:owner.test' no
healthy f
step ignored
step reject
step cancel
echo "PASS: cross-server bot contacts: request, approve, chat with mentions both ways, group with a bot reply, offline backfill, remove, reject, cancel"
