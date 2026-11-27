#!/usr/bin/env bash
# 域名参数化用例：install/verify 没给 DOMAIN 都要报错退出；非法 DOMAIN 也要退出；合法值通过。
# 不用 root、不连网：source install.sh 调 require_domain，verify.sh 在连网之前就会退出。
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
err=$(mktemp)
trap 'rm -f "$err"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

expect_missing_domain() {
    local script=$1 name out
    name=$(basename "$script")
    out=$(env -u DOMAIN bash "$script" 2>&1) && fail "$name 没给 DOMAIN 也过了"
    grep -q '缺 DOMAIN' <<<"$out" || fail "$name 没给 DOMAIN 的报错不对：$out"
}

# install.sh 被 source 时只定义函数；require_domain 在 root 检查之前。
if env -u DOMAIN bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>"$err"; then
    fail "install.sh 没给 DOMAIN 也过了"
fi
grep -q '缺 DOMAIN' "$err" || fail "install.sh 没给 DOMAIN 的报错不对：$(cat "$err")"

if DOMAIN=bad_domain bash -c '. "$1"; require_domain' _ "$HERE/install.sh" 2>"$err"; then
    fail "install.sh 非法 DOMAIN 也过了"
fi
grep -q '不是合法域名' "$err" || fail "install.sh 非法 DOMAIN 的报错不对：$(cat "$err")"

DOMAIN=push.example.com bash -c '. "$1"; require_domain' _ "$HERE/install.sh" ||
    fail "install.sh 合法 DOMAIN 被拒"

expect_missing_domain "$HERE/verify.sh"

echo "PASS: DOMAIN 必填（install/verify 缺或非法都退出，合法通过）"
