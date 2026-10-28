#!/usr/bin/env bash
# 隧道实地测试：服务端经 SSH 部署到一台 Linux 主机，客户端在本机运行，测量直连与中继的比例和延迟。
# 结论写进 docs/adr/0004-iroh-tunnel.md 的“验证结果”。
#
#   field-test.sh up <ssh 主机> [运行小时数，默认 2]
#       在主机上构建并启动测试用上游 HTTP 服务和隧道服务端，只放行本机客户端。两个进程到时自动退出，
#       不会留下常驻进程。服务端 ID 和中继地址存在本机状态目录里。
#   field-test.sh probe [tunnel-client probe 的参数，例如 --trials 10 --relay-only]
#       在本机运行客户端测量。先把本机切到要测的网络（例如手机热点），再执行；不需要 SSH。
#   field-test.sh down <ssh 主机>
#       提前停止主机上的进程，并删除主机上的测试目录。
#
# 主机需要 python3、rsync 和用户级 rustup（~/.cargo/bin/cargo）。国内主机设 WONTON_CARGO_MIRROR=rsproxy，
# 经 rsproxy.cn 下载工具链和依赖。本机状态（含客户端私钥）在 ~/.wonton-field-test，不进仓库。
set -euo pipefail

repo="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
state="${WONTON_FIELD_TEST_DIR:-$HOME/.wonton-field-test}"
port="${WONTON_FIELD_TEST_PORT:-47000}"

usage() {
  sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# 构建本机客户端，输出可执行文件路径。
client_bin() {
  cargo build --release --locked --quiet --manifest-path "$repo/Cargo.toml" \
    -p wonton-tunnel --example tunnel-client >&2
  echo "$repo/target/release/examples/tunnel-client"
}

up() {
  local host="$1" hours="${2:-2}" client client_id output server_id relay
  mkdir -p "$state"
  chmod 700 "$state"
  client="$(client_bin)"
  client_id="$("$client" --key "$state/client.key" id)"
  ssh "$host" mkdir -p wonton-field-test/src
  # 按各级 .gitignore 排除被忽略的文件，本地密钥和数据（如 deploy/dev/agent/.env、.secrets/）不传到远端。
  rsync -az --delete --exclude .git --filter=':- .gitignore' "$repo/" "$host:wonton-field-test/src/"
  output="$(ssh "$host" bash -s -- "$hours" "$client_id" "$port" "${WONTON_CARGO_MIRROR:-}" <<'REMOTE'
set -euo pipefail
hours="$1" client_id="$2" port="$3" mirror="$4"
cd ~/wonton-field-test
mirror_args=()
if [[ "$mirror" == rsproxy ]]; then
  export RUSTUP_DIST_SERVER=https://rsproxy.cn RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup
  mirror_args=(--config 'source.crates-io.replace-with="rsproxy"'
    --config 'source.rsproxy.registry="sparse+https://rsproxy.cn/index/"')
fi
(cd src && ~/.cargo/bin/cargo ${mirror_args[@]+"${mirror_args[@]}"} build --release --locked \
  --quiet -p wonton-tunnel --example tunnel-server) >&2
mkdir -p www run
echo "hello from $(hostname)" > www/index.txt
head -c 4194304 /dev/urandom > www/4m.bin
setsid nohup timeout "${hours}h" python3 -m http.server 18080 --bind 127.0.0.1 \
  --directory www > run/http.log 2>&1 &
echo $! > run/http.pid
setsid nohup timeout "${hours}h" src/target/release/examples/tunnel-server --key run/server.key \
  --port "$port" --upstream 127.0.0.1:18080 --allow "$client_id" > run/server.log 2>&1 &
echo $! > run/server.pid
for _ in $(seq 60); do
  grep -q -- '--relay' run/server.log && break
  sleep 1
done
cat run/server.log
REMOTE
)"
  echo "$output"
  server_id="$(sed -n 's/^服务端 endpoint ID：//p' <<<"$output")"
  relay="$(sed -n 's/^客户端可加参数：--relay //p' <<<"$output")"
  if [[ -z "$server_id" || -z "$relay" ]]; then
    echo "服务端没有在 60 秒内连上中继，见上面的日志" >&2
    exit 1
  fi
  printf 'host=%s\nserver_id=%s\nrelay=%s\n' "$host" "$server_id" "$relay" >"$state/server.env"
  echo "服务端已启动，${hours} 小时后自动退出。切到要测的网络后运行：$0 probe"
}

probe() {
  local client
  [[ -f "$state/server.env" ]] || { echo "先运行 $0 up <ssh 主机>" >&2; exit 1; }
  # shellcheck source=/dev/null
  source "$state/server.env"
  client="$(client_bin)"
  "$client" --key "$state/client.key" probe --server "$server_id" --relay "$relay" "$@"
}

down() {
  local host="$1"
  ssh "$host" 'cd ~/wonton-field-test 2>/dev/null || exit 0
    kill $(cat run/*.pid 2>/dev/null) 2>/dev/null || true
    sleep 1
    cd ~ && rm -rf ~/wonton-field-test'
  rm -f "$state/server.env"
  echo "已停止并清理 $host"
}

case "${1:-}" in
  up) [[ $# -ge 2 ]] || usage; up "${@:2}" ;;
  probe) probe "${@:2}" ;;
  down) [[ $# -eq 2 ]] || usage; down "$2" ;;
  *) usage ;;
esac
