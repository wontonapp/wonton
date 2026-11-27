#!/usr/bin/env bash
# healthcheck.sh 的行为测试：PATH 里放假 curl/ss/openssl/iroh-relay，构造五类故障和全绿。
# 纯 shell，不用 Docker、不连网。跑法：cloud/iroh-relay/test/healthcheck-test.sh
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SCRIPT=$HERE/healthcheck.sh
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAKEBIN=$TMP/bin
RELEASE_ENV=$TMP/release.env
printf 'IROH_RELAY_VERSION=1.4.1\n' >"$RELEASE_ENV"

passed=0
failed=0
ok() { passed=$((passed + 1)); echo "PASS: $*"; }
bad() {
    failed=$((failed + 1))
    echo "FAIL: $*"
}

fake() {
    printf '#!/bin/sh\n%s\n' "$2" >"$FAKEBIN/$1"
    chmod +x "$FAKEBIN/$1"
}

# 每个用例一套独立的环境，初始全部健康；用例只改要坏的那一项。
reset() {
    ETC=$TMP/etc
    rm -rf "$ETC" "$FAKEBIN"
    mkdir -p "$ETC/tls" "$FAKEBIN"
    : >"$ETC/config.toml"
    printf 'IROH_RELAY_ACCESS_TOKEN=test\n' >"$ETC/relay.env"
    : >"$ETC/tls/fullchain.pem"
    chmod 640 "$ETC/config.toml"
    chmod 600 "$ETC/relay.env"
    fake curl 'printf 200'
    fake ss 'printf "UNCONN 0 0 0.0.0.0:7842 0.0.0.0:*\n"'
    fake openssl 'exit 0'
    fake iroh-relay 'echo "iroh-relay 1.4.1"'
    CHECK_PATH=
}

run() {
    OUT=$(PATH="${CHECK_PATH:-$FAKEBIN:$PATH}" ETC="$ETC" BIN="$FAKEBIN/iroh-relay" \
        RELEASE_ENV="$RELEASE_ENV" sh "$SCRIPT" 2>&1) && RC=0 || RC=$?
}

dump() {
    printf '%s\n' "$OUT" | sed 's/^/    /'
}

# 断言：退出码 1、恰好一条匹配的 FAIL、没有别的检查被带坏。
expect_fail() {
    local name=$1 pattern=$2 count
    if [ "$RC" -ne 1 ]; then
        bad "$name: 退出码 $RC，应为 1"
        dump
        return
    fi
    if ! printf '%s\n' "$OUT" | grep -qE "$pattern"; then
        bad "$name: 输出里没有 /$pattern/"
        dump
        return
    fi
    count=$(printf '%s\n' "$OUT" | grep -c '^FAIL ' || true)
    if [ "$count" -ne 1 ]; then
        bad "$name: 有 $count 条 FAIL，其他检查应当仍然通过"
        dump
        return
    fi
    ok "$name"
}

if [ ! -f "$SCRIPT" ]; then
    bad "healthcheck.sh 不存在：$SCRIPT"
fi

reset
run
if [ "$RC" -ne 0 ] || ! printf '%s\n' "$OUT" | grep -qx 'healthcheck: OK'; then
    bad "全绿应 exit 0 并打印 healthcheck: OK"
    dump
else
    ok "全绿 exit 0"
fi
 
reset
fake curl 'printf 503; exit 22'
run
expect_fail "healthz 5xx" '^FAIL healthz: .*503'

reset
fake ss 'printf "UNCONN 0 0 127.0.0.1:7842 0.0.0.0:*\n"'
run
expect_fail "UDP 7842 只监听回环" '^FAIL qad: '

reset
fake openssl 'exit 1'
run
expect_fail "证书 10 天后到期" '^FAIL cert: '

reset
fake iroh-relay 'echo "iroh-relay 1.3.0"'
run
expect_fail "二进制版本漂移" '^FAIL version: .*1\.3\.0'

reset
chmod 644 "$ETC/relay.env"
run
expect_fail "令牌权限过宽" '^FAIL token: .*644'

# 缺工具时单项 SKIP 并注明，不静默当通过。PATH 只放脚本要用的真实工具，真的找不到 curl。
SKIPBIN=$TMP/skipbin
mkdir -p "$SKIPBIN"
for tool in dirname grep head sed stat sh; do ln -s "$(command -v "$tool")" "$SKIPBIN/$tool"; done
cp "$FAKEBIN/ss" "$FAKEBIN/openssl" "$FAKEBIN/iroh-relay" "$SKIPBIN/"

reset
rm -f "$FAKEBIN/curl"
CHECK_PATH=$SKIPBIN
run
if [ "$RC" -ne 0 ]; then
    bad "缺 curl 时其余项应通过，退出码 $RC"
    dump
elif ! printf '%s\n' "$OUT" | grep -q '^SKIP healthz: '; then
    bad "缺 curl 应打印 SKIP healthz"
    dump
else
    ok "缺 curl 时 SKIP 且不假绿"
fi

echo
if [ "$failed" -ne 0 ]; then
    echo "healthcheck 测试：$passed 通过，$failed 失败"
    exit 1
fi
echo "healthcheck 测试：$passed 通过"
