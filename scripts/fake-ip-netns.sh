#!/usr/bin/env bash
# 代理 TUN 假地址（198.18.0.0/15）的建连测试（`crates/wonton-tunnel/tests/tunnel/fake_ip.rs`）。
# 这组测试要 198.18.0.1 能绑、服务端自己的地址手机到不了，普通 CI 给不出来；这里在无特权的用户 + 网络命名空间
# （`unshare -rn`，不要 sudo，不碰宿主网络）里搭好再跑：
#   - 198.18.0.1 加在命名空间的回环上，当作代理 TUN 的假地址；
#   - 服务端绑 10.255.0.1，客户端绑 127.0.0.3，nftables 丢掉客户端直接发给服务端的包：像核心跑在容器里，
#     经 QNT 报上来的容器地址手机到不了，只能经发布的端口（测试里的转发）到。
# 只支持 Linux，要 unshare、ip、nft，内核要允许无特权用户命名空间。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

cargo="${WONTON_CARGO:-cargo}"
binary="$($cargo test -p wonton-tunnel --test tunnel --no-run 2>&1 | tee /dev/stderr \
  | sed -n 's/.*Executable .*(\(.*\))$/\1/p; s/.*Executable \([^ ]*\)$/\1/p' | tail -1)"
[[ -x "$binary" ]] || { echo "找不到 tunnel 测试二进制" >&2; exit 1; }

exec unshare -rn sh -euc '
  ip link set lo up
  ip addr add 198.18.0.1/15 dev lo
  ip addr add 10.255.0.1/32 dev lo
  nft -f - <<RULES
table inet wonton_fake_ip {
  chain out {
    type filter hook output priority 0;
    ip saddr 127.0.0.3 ip daddr 10.255.0.1 drop
  }
}
RULES
  WONTON_FAKE_IP_NETNS=1 exec "$0" fake_ip:: --test-threads=1 "$@"
' "$binary" "$@"
