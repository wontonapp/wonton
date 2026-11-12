#!/usr/bin/env bash
# 隔离的 ADR-0051 端到端分享冒烟：两台真云吞 + 一台「别家」Matrix + 多人在线插件。
#
# 全部在一条 internal:true 的隔离网段里，用自己起的权威 DNS、Pebble ACME 和测试 CA；
# 不碰公网 DNS、Let's Encrypt 正式环境和真域名。要名额、要 Docker。
#
# 用法：deploy/dev/core/smoke-sharing.sh            （在仓库根目录跑）
# 依赖：事先编好两个探针二进制到 WONTON_SHARE_SMOKE_BIN_DIR（默认 $HOME/wonton-dev/share-smoke-bin）
#   cargo build -p wonton-core --example fr_share_smoke
#   cargo build -p wonton-tunnel --example share-smoke-relay
set -euo pipefail
# 分流机上没有 .git（只 rsync 了工作区），退回脚本自己的位置算仓库根。
root="$(git rev-parse --show-toplevel 2>/dev/null)" || root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root"

name=share-smoke
project="wonton-$name"
fixture="deploy/dev/core/share-smoke"
bin_dir="${WONTON_SHARE_SMOKE_BIN_DIR:-$HOME/wonton-dev/share-smoke-bin}"
state="$(mktemp -d "${TMPDIR:-/tmp}/$name.XXXXXX")"
started=0

cleanup() {
  local status=$?
  if (( started )) && (( status != 0 )); then
    # 失败时把中继和核心的末尾日志一起打出来，日志只有这一份。
    docker compose -p "$project" -f "$fixture/compose.yaml" logs --tail 60 relay core-owner core-friend >&2 || true
  fi
  if (( started )); then
    if [[ -n "${SHARE_SMOKE_KEEP:-}" ]]; then
      echo "保留栈：docker compose -p $project -f $fixture/compose.yaml down -v --remove-orphans" >&2
    else
      docker compose -p "$project" -f "$fixture/compose.yaml" down -v --remove-orphans >/dev/null 2>&1 || true
      "$HOME/wonton-dev/stack-slot.sh" release "$name" >/dev/null 2>&1 || true
    fi
  fi
  rm -rf "$state" 2>/dev/null ||
    docker run --rm -v "$state:$state" --entrypoint sh "$SHARE_SMOKE_CERTBOT_IMAGE" \
      -c "rm -rf '$state'" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for binary in fr_share_smoke share-smoke-relay; do
  [[ -x "$bin_dir/$binary" ]] || { echo "缺少 $bin_dir/$binary，先按文件头的命令编译" >&2; exit 2; }
done

"$HOME/wonton-dev/stack-slot.sh" acquire --project "$project" "$name"
started=1
subnet="$("$HOME/wonton-dev/stack-slot.sh" subnet "$name")"
prefix="${subnet%.0/24}"

export SHARE_SMOKE_PROJECT="$project"
export SHARE_SMOKE_STATE="$state"
export SHARE_SMOKE_PREFIX="$prefix"
export SHARE_SMOKE_BIN_DIR="$bin_dir"
# 实例名只能小写字母和数字、最长 16 个字符：短前缀加随机数，两个核心各自再接一个字母。
export SHARE_SMOKE_INSTANCE="ss$RANDOM"
slot="${prefix##*.}"
export SHARE_SMOKE_IDENTITY_PREFIX="11.213.$slot"
export SHARE_SMOKE_PLUGIN_POOL="10.213.$slot.0/24"
# 必须是含分享实现的镜像（dev.sh share-smoke 会编好它）。
export SHARE_SMOKE_CORE_IMAGE="${WONTON_SHARE_SMOKE_CORE_IMAGE:-wonton-core:share-smoke}"
export SHARE_SMOKE_TUWUNEL_IMAGE="ghcr.io/matrix-construct/tuwunel:v1.9.4@sha256:0e86c6164d6c9f5b60291938eb61ad8395021c37fe58d47a963ea7edddbedaec"
export SHARE_SMOKE_PEBBLE_IMAGE="ghcr.io/letsencrypt/pebble:2.8.0@sha256:d9080f68f6cb6af8d82134ab26de0aaaf312ac9cba42aecc6d3aede6cb63007b"
export SHARE_SMOKE_CERTBOT_IMAGE="certbot/certbot:v5.1.0@sha256:bb72eb98a32ecc5dcef8eeeb8fb6a2a8c50d4d93e2a2c3542f8cb679840c990c"
export SHARE_SMOKE_PYTHON_IMAGE="python:3.13.3-alpine3.21@sha256:452682e4648deafe431ad2f2391d726d7c52f0ff291be8bd4074b10379bb89ff"

# 状态：两套核心目录、好友名单、测试密钥和证书（都在仓库外）。
"$bin_dir/fr_share_smoke" seed "$state"
export SHARE_SMOKE_JWT_PUBLIC="$(cat "$state/matrix.pub")"
cat > "$state/pebble.json" <<JSON
{"pebble": {"listenAddress": "0.0.0.0:14000", "managementListenAddress": "0.0.0.0:15000",
  "certificate": "$state/pebble.crt", "privateKey": "$state/pebble.key",
  "httpPort": 5002, "tlsPort": 5001, "ocspResponderURL": "",
  "externalAccountBindingRequired": false}}
JSON
: > "$state/paired-userinfo"

# 插件清单引用本地标签：隔离网段没有 registry，带摘要的 Hub 名字仍会去拉。
docker tag "$SHARE_SMOKE_PYTHON_IMAGE" wonton-share-multiplayer:smoke

compose=(docker compose -p "$project" -f "$fixture/compose.yaml")

# 证书签发前只启动 DNS 和 ACME；TLS 前门需要签发后的证书。
"${compose[@]}" up -d dns pebble
# 读取根证书的同时检查 ACME directory，等待服务就绪。
root_ok=0
for _ in $(seq 1 60); do
  if "${compose[@]}" exec -T dns python3 /fixture/fixture.py root >/dev/null 2>&1; then root_ok=1; break; fi
  sleep 2
done
if (( !root_ok )); then
  echo "取 Pebble 根证书失败，Pebble 日志：" >&2
  "${compose[@]}" logs pebble >&2 || true
  exit 1
fi

if ! "${compose[@]}" run --rm --no-deps -T -e "REQUESTS_CA_BUNDLE=$state/pebble.crt" \
  -v "$state:$state" --entrypoint sh entrance -ec "
    certbot certonly --non-interactive --agree-tos -m smoke@share-smoke.test \\
      --server https://pebble.test:14000/dir \\
      --config-dir $state/certs --work-dir $state/work --logs-dir $state/logs \\
      --cert-name share-smoke --manual --preferred-challenges dns \\
      --manual-auth-hook 'python3 /fixture/fixture.py auth' \\
      --manual-cleanup-hook 'python3 /fixture/fixture.py cleanup' \\
      -d share-smoke.test -d dns.test -d pebble.test -d relay.test \\
      -d owner.test -d friend.test -d external.test -d mismatch.test"; then
  "${compose[@]}" logs pebble >&2
  exit 1
fi

if ! "${compose[@]}" up -d --remove-orphans --wait --wait-timeout 300; then
  "${compose[@]}" logs >&2
  exit 1
fi

"${compose[@]}" --profile driver run --rm -T driver
