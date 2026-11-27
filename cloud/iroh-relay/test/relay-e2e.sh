#!/usr/bin/env bash
# 部署件端到端验证：官方固定版本 iroh-relay（--dev，TLS 由 nginx 终结）后面，
# 一对只经这个中继、禁直连的 iroh 端点跑 30 秒下载测吞吐，再用错令牌验证被拒。
#
# 依赖：docker、openssl、curl、cargo、jq。所有状态在 mktemp 目录里，退出时清理。
# 用法：
#   cloud/iroh-relay/test/relay-e2e.sh                 # 首次会下载官方二进制并编译上游 transfer 示例
#   MIN_MBPS=1 cloud/iroh-relay/test/relay-e2e.sh      # 抬高吞吐门槛（默认限速 256 KiB/s ≈ 2 Mbps）
#   KEEP=1 cloud/iroh-relay/test/relay-e2e.sh          # 保留临时目录看日志
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck source=../release.env
. "$HERE/release.env"

MIN_MBPS=${MIN_MBPS:-1.5}
TRANSFER_DURATION=${TRANSFER_DURATION:-30}
QAD_PORT=${QAD_PORT:-7842}   # 客户端默认连 7842，测试里不能换
HTTPS_PORT=${HTTPS_PORT:-443}
pick_port() {
    python3 -c 'import socket
s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}
HTTP_PORT=${HTTP_PORT:-$(pick_port)}
METRICS_PORT=${METRICS_PORT:-$(pick_port)}
TLS_PORT=${TLS_PORT:-$(pick_port)}
HTTP_REDIRECT_PORT=${HTTP_REDIRECT_PORT:-$(pick_port)}
NGINX_IMAGE=${NGINX_IMAGE:-nginx:1.29-alpine}
CACHE=${CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/wonton-iroh-relay-test}
TMP=$(mktemp -d /tmp/wonton-iroh-relay-e2e.XXXXXX)
NGINX_NAME=wonton-iroh-relay-e2e-nginx-$$
TOKEN=$(openssl rand -hex 32)
BAD_TOKEN=$(openssl rand -hex 32)
RELAY_URL="https://localhost:$TLS_PORT"
TRANSFER=$CACHE/transfer
RELAY_BIN=$CACHE/iroh-relay
PROVIDER_PID=""
NGINX_STARTED=0

fail_hard() {
    echo "FAIL: $*" >&2
    exit 1
}

pass() {
    echo "PASS: $*"
}

cleanup() {
    local rc=$?
    if [[ -n ${PROVIDER_PID-} ]]; then kill "$PROVIDER_PID" 2>/dev/null || true; fi
    if [[ -n ${RELAY_PID-} ]]; then kill "$RELAY_PID" 2>/dev/null || true; fi
    if ((NGINX_STARTED)); then docker rm -f "$NGINX_NAME" >/dev/null 2>&1 || true; fi
    pkill -f "$RELAY_BIN --dev" 2>/dev/null || true
    if ((rc == 0)) && [[ ${KEEP:-0} != 1 ]]; then
        rm -rf "$TMP"
    else
        echo "临时目录保留在 $TMP"
    fi
}
trap cleanup EXIT

need() {
    command -v "$1" >/dev/null || fail_hard "缺 $1"
}

# 1. 部署件必须在。缺哪个报哪个（这是写部署件之前的红）。
missing=()
for f in config.toml.example relay.env.example wonton-iroh-relay.service install.sh uninstall.sh \
    nginx/relay.conf.template nginx/relay.bootstrap.conf.template nginx/cert-deploy-hook.sh.template; do
    [[ -e $HERE/$f ]] || missing+=("$f")
done
if ((${#missing[@]})); then
    fail_hard "部署件缺 ${#missing[@]} 个：${missing[*]}"
fi

for c in docker openssl curl cargo jq sha256sum ss; do need "$c"; done

# DOMAIN 必填：没给、给了非法值都要在 root 检查之前退出；合法值通过（都在子 shell 里跑，不污染测试）。
if env -u DOMAIN bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>"$TMP/no-domain.err"; then
    fail_hard "没给 DOMAIN 也过了"
fi
grep -q '缺 DOMAIN' "$TMP/no-domain.err" || fail_hard "缺 DOMAIN 的报错文案不对：$(cat "$TMP/no-domain.err")"
if DOMAIN='bad_domain' bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>/dev/null; then
    fail_hard "非法 DOMAIN 也过了"
fi
DOMAIN=relay.example.com bash -c '. "$1"; require_domain' _ "$HERE/install.sh" ||
    fail_hard "合法 DOMAIN 被拒"
pass "DOMAIN 必填（缺/非法都退出，合法通过）"

# 准入判定用例：缺 [access] 节时上游默认 everyone，不能误报成 shared_token。
# shellcheck source=../install.sh
. "$HERE/install.sh"
adm=$TMP/admission.toml
printf 'http_bind_addr = "127.0.0.1:8742"\n' >"$adm"
[[ $(admission_mode "$adm") == everyone-default ]] || fail_hard "缺 [access] 应判 everyone-default"
printf 'access = "everyone"\n' >"$adm"
[[ $(admission_mode "$adm") == everyone ]] || fail_hard "显式 everyone 应判 everyone"
printf '[access]\nshared_token = []\n' >"$adm"
[[ $(admission_mode "$adm") == restricted ]] || fail_hard "[access] 节应判 restricted"
printf '[access.http]\nurl = "https://auth.example/relay"\n' >"$adm"
[[ $(admission_mode "$adm") == restricted ]] || fail_hard "[access.http] 应判 restricted"
printf 'access = { shared_token = [] }\n' >"$adm"
[[ $(admission_mode "$adm") == restricted ]] || fail_hard "行内表 access 应判 restricted"
pass "准入判定用例（缺节=everyone-default / 显式 everyone / [access]、[access.http]、行内表=restricted）"

mkdir -p "$CACHE"
[[ -n $(docker images -q "$NGINX_IMAGE") ]] || docker pull -q "$NGINX_IMAGE" >/dev/null

# 2. 官方二进制：只有版本对得上才跳过下载（sha256 写在 release.env）。
arch=$(uname -m)
case $arch in
    x86_64) sha=$IROH_RELAY_SHA256_X86_64 ;;
    aarch64) sha=$IROH_RELAY_SHA256_AARCH64 ;;
    *) fail_hard "不支持的架构 $arch" ;;
esac
if [[ ! -x $RELAY_BIN ]] || ! "$RELAY_BIN" --version 2>/dev/null | grep -qx "iroh-relay $IROH_RELAY_VERSION"; then
    url="$IROH_RELAY_RELEASE_BASE/iroh-relay-v$IROH_RELAY_VERSION-$arch-unknown-linux-musl.tar.gz"
    echo "下载 $url"
    tmp_tar=$TMP/relay.tar.gz
    curl -fsSL --retry 3 --max-time 300 -o "$tmp_tar" "$url"
    echo "$sha  $tmp_tar" | sha256sum -c - >/dev/null || fail_hard "sha256 不匹配"
    tar xzf "$tmp_tar" -C "$TMP" ./iroh-relay
    install -m 0755 "$TMP/iroh-relay" "$RELAY_BIN"
fi
"$RELAY_BIN" --version | grep -qx "iroh-relay $IROH_RELAY_VERSION" || fail_hard "二进制版本不对"

# 3. 上游同版本的 transfer 示例：只用来当一对 iroh 端点（--relay-only 禁直连）。
if [[ ! -x $TRANSFER ]]; then
    src=$CACHE/iroh-$IROH_RELAY_VERSION
    if [[ ! -d $src ]]; then
        echo "拉 iroh v$IROH_RELAY_VERSION 源码（只为编 transfer 示例）"
        mkdir -p "$src"
        curl -fsSL --retry 3 --max-time 600 \
            "https://codeload.github.com/n0-computer/iroh/tar.gz/refs/tags/v$IROH_RELAY_VERSION" |
            tar xz -C "$src" --strip-components=1
    fi
    echo "编译 transfer 示例"
    (cd "$src" && CARGO_TARGET_DIR=$CACHE/target cargo build --release -p iroh --example transfer --features test-utils)
    install -m 0755 "$CACHE/target/release/examples/transfer" "$TRANSFER"
fi

# 4. 自签证书（客户端用 --env dev，会 insecure_skip_verify）。
mkdir -p "$TMP/tls"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
    -keyout "$TMP/tls/privkey.pem" -out "$TMP/tls/fullchain.pem" \
    -days 2 -nodes -subj '/CN=localhost' -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1' 2>/dev/null

# 5. 中继按模板配置起在回环（--dev：HTTP 明文，TLS 只留给 QAD）。
sed -e "s#127.0.0.1:8742#127.0.0.1:$HTTP_PORT#" \
    -e "s#127.0.0.1:19090#127.0.0.1:$METRICS_PORT#" \
    -e "s#0.0.0.0:7842#0.0.0.0:$QAD_PORT#" \
    -e "s#/etc/wonton-iroh-relay/tls#$TMP/tls#g" \
    "$HERE/config.toml.example" > "$TMP/relay.toml"
# 5a. 准入默认是关的：缺令牌文件、或令牌为空，都必须拒绝启动，而不是落到 everyone。
for mode in missing empty; do
    rc=0
    if [[ $mode == missing ]]; then
        env -u IROH_RELAY_ACCESS_TOKEN timeout 10 "$RELAY_BIN" --dev --config-path "$TMP/relay.toml" \
            >"$TMP/relay-$mode.log" 2>&1 || rc=$?
    else
        env IROH_RELAY_ACCESS_TOKEN= timeout 10 "$RELAY_BIN" --dev --config-path "$TMP/relay.toml" \
            >"$TMP/relay-$mode.log" 2>&1 || rc=$?
    fi
    [[ $rc -ne 0 ]] || fail_hard "令牌 $mode 时中继也起来了（默认放开了？）"
    grep -q 'shared_token must not be empty' "$TMP/relay-$mode.log" ||
        fail_hard "令牌 $mode 时报错不是 shared_token 为空：$(tail -2 "$TMP/relay-$mode.log")"
done
pass "缺令牌/空令牌都拒绝启动"

LIMIT_BPS=$(awk -F' = ' '/^bytes_per_second/ {print $2}' "$TMP/relay.toml")
LIMIT_BURST=$(awk -F' = ' '/^max_burst_bytes/ {print $2}' "$TMP/relay.toml")
[[ -n $LIMIT_BPS && -n $LIMIT_BURST ]] || fail_hard "模板里没有 [limits.client.rx] 限速配置"
# 合法上界 = 稳态速率 + 突发在 TRANSFER_DURATION 内摊平，再留 20% 给 QUIC 开销/取整。
MAX_MBPS=$(awk -v b="$LIMIT_BPS" -v x="$LIMIT_BURST" -v d="$TRANSFER_DURATION" \
    'BEGIN { printf "%.2f", (b * 8 + x * 8 / d) * 1.2 / 1000000 }')

IROH_RELAY_ACCESS_TOKEN=$TOKEN RUST_LOG=info \
    "$RELAY_BIN" --dev --config-path "$TMP/relay.toml" >"$TMP/relay.log" 2>&1 &
RELAY_PID=$!
for _ in $(seq 1 40); do
    curl --noproxy '*' -fsS -o /dev/null "http://127.0.0.1:$HTTP_PORT/healthz" && break
    sleep 0.25
done
curl --noproxy '*' -fsS -o /dev/null "http://127.0.0.1:$HTTP_PORT/healthz" || {
    tail -20 "$TMP/relay.log" >&2
    fail_hard "中继没起来"
}
pass "中继 healthz"
# 令牌不能出现在日志里（默认 info；debug/trace 会把整份配置 dump 出来，已知且写进 README）。
if grep -qF -- "$TOKEN" "$TMP/relay.log"; then
    fail_hard "令牌进了 info 日志"
fi
pass "令牌不在 info 日志"
# 6a. QAD 必须对外监听（quic_bind_addr 若放错到顶层，上游会静默忽略、只绑回环）。
qad=$(ss -lunH "sport = :$QAD_PORT" || true)
echo "$qad" | grep -qE "(\*|0\.0\.0\.0):$QAD_PORT" ||
    fail_hard "QAD 没有对外监听 0.0.0.0:$QAD_PORT（多半是 quic_bind_addr 放错节了）：$qad"
pass "QAD 对外监听 0.0.0.0:$QAD_PORT"

# 6. nginx 终结 TLS，按 WebSocket Upgrade 反代到中继（部署配置本身过一遍 sed）。
# 宿主机可能已经占着 80/443，所以测试里把两个 listen 都挪到空闲端口（只改测试的绑定，不改文件语义）。
sed -e "s|@DOMAIN@|localhost|g" \
    -e "s#listen 443 ssl;#listen 127.0.0.1:$TLS_PORT ssl;#" \
    -e "s#^    listen 80;#    listen 127.0.0.1:$HTTP_REDIRECT_PORT;#" \
    -e '/listen \[::\]:443 ssl;/d' \
    -e '/listen \[::\]:80;/d' \
    -e "s#/etc/letsencrypt/live/localhost/fullchain.pem#/tls/fullchain.pem#" \
    -e "s#/etc/letsencrypt/live/localhost/privkey.pem#/tls/privkey.pem#" \
    -e "s#127.0.0.1:8742#127.0.0.1:$HTTP_PORT#" \
    -e "s#/var/log/nginx/localhost.access.log#/dev/stdout#" \
    "$HERE/nginx/relay.conf.template" > "$TMP/nginx.conf"
docker run --rm -d --name "$NGINX_NAME" --network host \
    -v "$TMP/nginx.conf:/etc/nginx/conf.d/default.conf:ro" \
    -v "$TMP/tls:/tls:ro" "$NGINX_IMAGE" >/dev/null
NGINX_STARTED=1
for _ in $(seq 1 40); do
    curl --noproxy '*' -kfsS -o /dev/null "https://127.0.0.1:$TLS_PORT/healthz" && break
    sleep 0.25
done
curl --noproxy '*' -kfsS -o /dev/null "https://127.0.0.1:$TLS_PORT/healthz" || {
    docker logs "$NGINX_NAME" >&2 || true
    fail_hard "nginx 到中继的 WebSocket 反代不通"
}
pass "nginx TLS 终结 + 反代 healthz"

# 7. provider：只用这个中继、禁直连。
"$TRANSFER" provide --env dev --relay-url "$RELAY_URL" --relay-auth-token "$TOKEN" \
    --relay-only --no-address-lookup --output json >"$TMP/provider.jsonl" 2>"$TMP/provider.err" &
PROVIDER_PID=$!
peer=""
for _ in $(seq 1 120); do
    peer=$(jq -r 'select(.kind == "EndpointBound") | .endpoint_id' "$TMP/provider.jsonl" 2>/dev/null | head -1)
    [[ -n $peer ]] && break
    kill -0 "$PROVIDER_PID" 2>/dev/null || { cat "$TMP/provider.err" >&2; fail_hard "provider 退出了"; }
    sleep 0.5
done
[[ -n $peer ]] || fail_hard "provider 没连上中继"
pass "provider 经中继上线"

# 8. 正例：30 秒下载，算 Mbps；路径必须是中继，中继的字节计数必须涨。
timeout 120 "$TRANSFER" fetch "$peer" --env dev --relay-url "$RELAY_URL" --relay-auth-token "$TOKEN" \
    --relay-only --no-address-lookup --output json \
    --duration "$TRANSFER_DURATION" --remote-relay-url "$RELAY_URL" \
    >"$TMP/fetch.jsonl" 2>"$TMP/fetch.err" || fail_hard "fetch 失败：$(tail -3 "$TMP/fetch.err")"
done_line=$(jq -c 'select(.kind == "DownloadComplete")' "$TMP/fetch.jsonl" | tail -1)
[[ -n $done_line ]] || fail_hard "没收到 DownloadComplete"
size=$(jq -r .size <<<"$done_line")
dur=$(jq -r .duration <<<"$done_line")
mbps=$(awk -v s="$size" -v d="$dur" 'BEGIN { if (d <= 0) print 0; else printf "%.2f", s * 8 / d }')
awk -v m="$mbps" -v min="$MIN_MBPS" 'BEGIN { exit !(m >= min) }' ||
    fail_hard "吞吐 $mbps Mbps 低于门槛 $MIN_MBPS Mbps"
awk -v m="$mbps" -v max="$MAX_MBPS" 'BEGIN { exit !(m <= max) }' ||
    fail_hard "吞吐 $mbps Mbps 超过限速+突发允许的上界 $MAX_MBPS Mbps，[limits] 没生效？"
pass "经中继下载 ${size} 字节，${mbps} Mbps（限速窗口内，门槛 $MIN_MBPS，上界 $MAX_MBPS）"

jq -e 'select(.kind == "PathStats") | .paths[] | select(.remote_addr | tostring | test("relay"; "i"))' \
    "$TMP/fetch.jsonl" >/dev/null || fail_hard "路径不是中继"
pass "路径是中继（禁直连生效）"

sent=$(curl --noproxy '*' -fsS "http://127.0.0.1:$METRICS_PORT/metrics" |
    awk '/^relayserver_bytes_sent_total/ { print $2 }')
[[ ${sent:-0} -gt 0 ]] || fail_hard "中继 bytes_sent 计数没涨"
pass "中继 bytes_sent_total=$sent"

# 9. 反例：错令牌会被拒（连不上中继，拿不到 DownloadComplete）。
rc=0
timeout 25 "$TRANSFER" fetch "$peer" --env dev --relay-url "$RELAY_URL" --relay-auth-token "$BAD_TOKEN" \
    --relay-only --no-address-lookup --output json \
    --duration 5 --remote-relay-url "$RELAY_URL" \
    >"$TMP/bad.jsonl" 2>"$TMP/bad.err" || rc=$?
if jq -e 'select(.kind == "DownloadComplete")' "$TMP/bad.jsonl" >/dev/null 2>&1; then
    fail_hard "错令牌也能下载（rc=$rc）"
fi
grep -qi 'denying client auth\|serverdeniedauth' "$TMP/relay.log" ||
    echo "注意：info 日志里没有 denial 行（上游是 trace），但行为已拒绝"
pass "错令牌被拒（rc=$rc）"

echo "全部通过"
