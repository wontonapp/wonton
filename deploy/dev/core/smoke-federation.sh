#!/usr/bin/env bash
# 跨服务器好友聊天的两栈端到端（#26 C6，ADR-0050、ADR-0051、ADR-0090）：两套开发栈、两台核心经 iroh 配对，
# 一台的客户端核心发消息，另一台的客户端核心看到同一句明文；中间把对方的核心暂停，验证恢复后补到并解出。
#
# 断言在 crates/wonton-client/examples/cross_server_chat_e2e.rs，这里负责起栈、出码、拿令牌、跑驱动，退出时
# 连数据卷一起拆两栈并放名额。main 上 C2/C3/C4 还没合时部分步骤会红，这正是本脚本的用途。
#
# 用法：deploy/dev/core/smoke-federation.sh            （在仓库根目录跑，要 Docker 和两个验证栈名额）
# 两个名额名和项目名默认 people-fed26-a、people-fed26-b；端口从 WONTON_FED_BASE_PORT（默认 18480）往后数四个。
# 默认不装 Hermes（WONTON_DEV_DEFAULT_PLUGINS 置空），跑完的每步 PASS/FAIL 和两份核心日志留在
# WONTON_FED_STATE（默认 mktemp）：WONTON_FED_KEEP=1 时留着栈让 WONTON_FED_STATE 跟着留着。
set -euo pipefail
root="$(git rev-parse --show-toplevel 2>/dev/null)" || root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root"
slot="$HOME/wonton-dev/stack-slot.sh"
dev="$root/deploy/dev/core/dev.sh"
driver_example=cross_server_chat_e2e

project_a="${WONTON_FED_PROJECT_A:-people-fed26-a}"
project_b="${WONTON_FED_PROJECT_B:-people-fed26-b}"
base="${WONTON_FED_BASE_PORT:-18480}"
port_a="$base"
tunnel_a=$((base + 2))
port_b=$((base + 1))
tunnel_b=$((base + 3))
domain_a="$project_a.wonton.localhost"
domain_b="$project_b.wonton.localhost"
state="${WONTON_FED_STATE:-$(mktemp -d "${TMPDIR:-/tmp}/fed-e2e.XXXXXX")}"
export WONTON_FED_STATE="$state"
mkdir -p "$state/a" "$state/b" "$state/a/client" "$state/b/client"

# 1k 的 /opt/wonton 是 root 700：dev-guard 的 `cd -P` 进不去，会把安装目录解析成空、拒绝一切写命令。
# 显式指到本栈的数据目录（开发栈的安装目录本来就是它；容器里的 WONTON_INSTALL_DIR 也由 compose 设成它）。
WONTON_INSTALL_DIR="$state/a/data"
export WONTON_INSTALL_DIR
mkdir -p "$state/a/data" "$state/b/data"

env_a=(
  "WONTON_DEV_PROJECT=$project_a"
  "WONTON_DEV_PORT=$port_a"
  "WONTON_DEV_TUNNEL_PORT=$tunnel_a"
  "WONTON_DEV_DOMAIN=$domain_a"
  "WONTON_DEV_DATA=$state/a/data"
  "WONTON_DEV_DEFAULT_PLUGINS="
  "WONTON_PLUGIN_NETWORK_POOL=10.230.1.0/24"
)
env_b=(
  "WONTON_DEV_PROJECT=$project_b"
  "WONTON_DEV_PORT=$port_b"
  "WONTON_DEV_TUNNEL_PORT=$tunnel_b"
  "WONTON_DEV_DOMAIN=$domain_b"
  "WONTON_DEV_DATA=$state/b/data"
  "WONTON_DEV_DEFAULT_PLUGINS="
  "WONTON_PLUGIN_NETWORK_POOL=10.230.2.0/24"
)

held_a=0
held_b=0
stopped=0

stack() { # stack a|b <dev.sh 的参数…>
  local side="$1"
  shift
  case "$side" in
    a) env "${env_a[@]}" "$dev" "$@" ;;
    b) env "${env_b[@]}" "$dev" "$@" ;;
  esac
}

logs() {
  docker logs "$project_a" >"$state/core-a.full.log" 2>&1 || true
  docker logs "$project_b" >"$state/core-b.full.log" 2>&1 || true
  docker logs --tail 80 "$project_a" >"$state/core-a.log" 2>&1 || true
  docker logs --tail 80 "$project_b" >"$state/core-b.log" 2>&1 || true
  # 联邦的红通常在 Tuwunel 一侧（握手、签名），核心日志只有它转回来的 502。
  docker logs "$project_a-tuwunel-1" >"$state/tuwunel-a.log" 2>&1 || true
  docker logs "$project_b-tuwunel-1" >"$state/tuwunel-b.log" 2>&1 || true
}

stop_stacks() {
  (( stopped )) && return 0
  stopped=1
  docker unpause "$project_a" "$project_b" >/dev/null 2>&1 || true
  if [[ -n "${WONTON_FED_KEEP:-}" ]]; then
    echo "保留两栈：${env_a[*]} \"$dev\" reset；$slot release $project_a（B 同理）" >&2
    return 0
  fi
  logs
  stack a reset >/dev/null 2>&1 || true
  stack b reset >/dev/null 2>&1 || true
  (( held_a )) && "$slot" release "$project_a" >/dev/null 2>&1 || true
  (( held_b )) && "$slot" release "$project_b" >/dev/null 2>&1 || true
  purge "$project_a"
  purge "$project_b"
}

# dev.sh reset 之后还剩下的同名资源（栈起了一半、reset 中途失败时会有）：按 compose 项目标签删掉。
purge() {
  docker ps -aq --filter "label=com.docker.compose.project=$1" | xargs -r docker rm -f >/dev/null 2>&1 || true
  docker volume ls -q --filter "label=com.docker.compose.project=$1" |
    xargs -r docker volume rm >/dev/null 2>&1 || true
  docker network ls -q --filter "label=com.docker.compose.project=$1" |
    xargs -r docker network rm >/dev/null 2>&1 || true
}

cleanup() {
  local status=$?
  stop_stacks
  [[ -n "${WONTON_FED_KEEP:-}" ]] || rm -rf "$state"
  exit "$status"
}
trap cleanup EXIT

# 先编好驱动再出码：码 10 分钟就过期，而排 cargo 名额可能更久。出码之后只剩一次增量编译，几百毫秒。
nice -n 19 "$HOME/wonton-dev/cargo-slot.sh" cargo build -p wonton-client \
  --example "$driver_example" --target-dir "$root/target"

# 名额按名字排：先占 A 再占 B，任何一个拿不到都只放已经拿到的。
"$slot" acquire --project "$project_a" "$project_a"
held_a=1
WONTON_DEV_CORE_SUBNET="$("$slot" subnet "$project_a")"
export WONTON_DEV_CORE_SUBNET
stack a up
"$slot" acquire --project "$project_b" "$project_b"
held_b=1
WONTON_DEV_CORE_SUBNET="$("$slot" subnet "$project_b")"
export WONTON_DEV_CORE_SUBNET
stack b up
unset WONTON_DEV_CORE_SUBNET

# 容器里 `*.wonton.localhost` 由 Docker 的 DNS 解析成 `::1`/`127.0.0.1`，Tuwunel 会把发往对方的联邦请求
# 发回自己，核心的出站 CONNECT 代理根本收不到。把对方域名写进本栈 Tuwunel 的 extra_hosts 指到本栈核心：
# 解析成功的地址只用来建连接，真正去哪由核心按 CONNECT 里的名字转；只重建 Tuwunel，不动网络。
peer_host() { # <side> <对方的域名>
  local project ip
  if [[ "$1" == a ]]; then project="$project_a"; else project="$project_b"; fi
  ip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"${project}_core\").IPAddress}}" "$project")"
  export WONTON_DEV_TUWENEL_PEER="$2:$ip"
  stack "$1" up
}
peer_host a "$domain_b"
peer_host b "$domain_a"
unset WONTON_DEV_TUWENEL_PEER

code() { # code a|b
  local file="$state/$1.code" code
  stack "$1" pair >"$file" 2>&1
  code="$(grep -o 'wonton://[^[:space:]]*' "$file" | tail -n 1)"
  [[ -n "$code" ]] || { echo "$1 没出码，见 $file" >&2; exit 1; }
  printf '%s\n' "$code" >"$file"
}
echo "A http://$domain_a:$port_a  B http://$domain_b:$port_b"

code a
code b
stack a token >"$state/a.token"
stack b token >"$state/b.token"
set +e
# 直接跑编好的二进制：再经 cargo 会重新排队领名额，而配对码 10 分钟就过期。
"$root/target/debug/examples/$driver_example" \
  --a-dir "$state/a/client" --b-dir "$state/b/client" \
  --a-code-file "$state/a.code" --b-code-file "$state/b.code" \
  --a-api "http://$domain_a:$port_a" --b-api "http://$domain_b:$port_b" \
  --a-token-file "$state/a.token" --b-token-file "$state/b.token" \
  --b-core "$project_b" 2>&1 | tee "$state/driver.log"

driver=${PIPESTATUS[0]}
set -e

stop_stacks
trap - EXIT
if (( driver != 0 )); then
  for file in "$state/core-a.log" "$state/core-b.log"; do
    echo "== ${file##*/} 末尾" >&2
    tail -60 "$file" >&2 || true
  done
  # C2/C3 的红要看联邦投递：把两份完整日志里和联邦、8448 有关的行摘出来。
  for file in "$state"/core-*.full.log; do
    echo "== ${file##*/} 里的联邦行" >&2
    grep -E 'federat|8448|8008|handshake|peer|signature|not paired|public' "$file" |
      tail -40 >&2 || true
  done
  for file in "$state"/tuwunel-*.log; do
    echo "== ${file##*/} 里的握手/联邦行" >&2
    grep -iE 'tls|cert|handshake|federat|8448|502|fail' "$file" | tail -40 >&2 || true
  done
fi
counts="$(awk '/^STEP .*: PASS/{p++} /^STEP .*: FAIL/{f++} END{print p+0, f+0}' "$state/driver.log")"
[[ -n "${WONTON_FED_KEEP:-}" ]] || rm -rf "$state"
read -r passed failed <<<"$counts"
echo "PASS=$passed FAIL=$failed"
exit "$driver"
