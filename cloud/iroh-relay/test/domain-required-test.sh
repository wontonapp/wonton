#!/usr/bin/env bash
# 域名参数化用例：install/uninstall 没给 DOMAIN 都要报错退出；非法 DOMAIN 也要退出；合法值通过。
# 不用 root、不连网：只 source install.sh 调 require_domain，uninstall.sh 在 root 检查之前就会退出。
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
err=$(mktemp)
trap 'rm -f "$err"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

if env -u DOMAIN bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>"$err"; then
    fail "install.sh 没给 DOMAIN 也过了"
fi
grep -q '缺 DOMAIN' "$err" || fail "install.sh 没给 DOMAIN 的报错不对：$(cat "$err")"

if DOMAIN=bad_domain bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>"$err"; then
    fail "install.sh 非法 DOMAIN 也过了"
fi
grep -q '不是合法域名' "$err" || fail "install.sh 非法 DOMAIN 的报错不对：$(cat "$err")"

DOMAIN=relay.example.com bash -c '. "$1"; require_domain' _ "$HERE/install.sh" ||
    fail "install.sh 合法 DOMAIN 被拒"

if out=$(env -u DOMAIN bash "$HERE/uninstall.sh" 2>&1); then
    fail "uninstall.sh 没给 DOMAIN 也过了"
fi
grep -q '缺 DOMAIN' <<<"$out" || fail "uninstall.sh 没给 DOMAIN 的报错不对：$out"

echo "PASS: DOMAIN 必填（install/uninstall 缺或非法都退出，合法通过）"
