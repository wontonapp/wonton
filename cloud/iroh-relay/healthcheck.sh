#!/bin/sh
# iroh 中继的本机运行期自检：配置和令牌的权限、/healthz、UDP 7842 的 QAD 监听、证书剩余天数、二进制版本。
# 任一失败打印 FAIL 并 exit 1，交给系统的失败日志（见 README 的“运行期健康自检”）；全过 exit 0。
# 只读检查，不重启服务。缺 curl/ss/openssl 时该项打 SKIP 并注明，不当成通过。
#
# 环境变量（测试和排错用，默认值就是 install.sh 的部署路径）：
#   ETC=/etc/wonton-iroh-relay BIN=/opt/wonton-iroh-relay/iroh-relay
#   RELEASE_ENV=<脚本目录>/release.env CERT=$ETC/tls/fullchain.pem
#   HEALTHZ_URL=http://127.0.0.1:8742/healthz QAD_PORT=7842 MIN_CERT_DAYS=30
set -eu

ETC=${ETC:-/etc/wonton-iroh-relay}
BIN=${BIN:-/opt/wonton-iroh-relay/iroh-relay}
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
RELEASE_ENV=${RELEASE_ENV:-$HERE/release.env}
CERT=${CERT:-$ETC/tls/fullchain.pem}
HEALTHZ_URL=${HEALTHZ_URL:-http://127.0.0.1:8742/healthz}
QAD_PORT=${QAD_PORT:-7842}
MIN_CERT_DAYS=${MIN_CERT_DAYS:-30}

failed=0
fail() {
    failed=$((failed + 1))
    echo "FAIL $*"
}
skip() {
    echo "SKIP $*"
}

# install.sh 建的是 config.toml 0640、relay.env 0600；两者都只收 600/640，超过 640 就报。
check_private_file() {
    file=$1
    label=$2
    if [ ! -e "$file" ]; then
        fail "$label: $file 不存在"
        return 0
    fi
    if [ ! -f "$file" ]; then
        fail "$label: $file 不是普通文件"
        return 0
    fi
    mode=$(stat -c %a -- "$file" 2>/dev/null || echo '?')
    case $mode in
        600 | 640) ;;
        *) fail "$label: $file 权限 $mode，应为 600 或 640（组或其他可读写）" ;;
    esac
    return 0
}

check_healthz() {
    if ! command -v curl >/dev/null 2>&1; then
        skip "healthz: 未安装 curl，未检查 $HEALTHZ_URL"
        return 0
    fi
    code=$(curl --noproxy '*' -fsS -m 5 -o /dev/null -w '%{http_code}' "$HEALTHZ_URL" 2>/dev/null) ||
        code=${code:-000}
    if [ "$code" != 200 ]; then
        fail "healthz: $HEALTHZ_URL 返回 $code，应为 200"
    fi
    return 0
}

# QAD 必须对公网监听；沿用 install.sh 的判定，只认 *、0.0.0.0、[::]。
check_qad() {
    if ! command -v ss >/dev/null 2>&1; then
        skip "qad: 未安装 ss，未检查 UDP $QAD_PORT 监听"
        return 0
    fi
    listeners=$(ss -lunH "sport = :$QAD_PORT" 2>/dev/null) || listeners=
    if [ -z "$listeners" ]; then
        fail "qad: UDP $QAD_PORT 没有监听"
        return 0
    fi
    if ! printf '%s\n' "$listeners" | grep -qE "(\*|0\.0\.0\.0|\[::\]):$QAD_PORT([[:space:]]|$)"; then
        fail "qad: UDP $QAD_PORT 没有监听在 0.0.0.0/[::]：$(printf '%s\n' "$listeners" | head -n 1)"
    fi
    return 0
}

check_cert() {
    if [ ! -e "$CERT" ]; then
        fail "cert: $CERT 不存在"
        return 0
    fi
    if ! command -v openssl >/dev/null 2>&1; then
        skip "cert: 未安装 openssl，未检查 $CERT"
        return 0
    fi
    if ! openssl x509 -checkend "$((MIN_CERT_DAYS * 86400))" -noout -in "$CERT" >/dev/null 2>&1; then
        fail "cert: $CERT 在 $MIN_CERT_DAYS 天内到期或无法读取"
    fi
    return 0
}

# 版本必须和 release.env 的固定值逐字一致；install.sh 也用同一个比较。
check_version() {
    if [ ! -f "$RELEASE_ENV" ]; then
        fail "version: $RELEASE_ENV 不存在，无法比对固定版本"
        return 0
    fi
    want=$(sed -n 's/^IROH_RELAY_VERSION=//p' -- "$RELEASE_ENV" | head -n 1)
    if [ -z "$want" ]; then
        fail "version: $RELEASE_ENV 没有 IROH_RELAY_VERSION"
        return 0
    fi
    if [ ! -x "$BIN" ]; then
        fail "version: 二进制 $BIN 不存在或不可执行"
        return 0
    fi
    got=$("$BIN" --version 2>/dev/null) || got=
    if [ "$got" != "iroh-relay $want" ]; then
        fail "version: 二进制版本 '$got'，release.env 固定 $want（版本漂移）"
    fi
    return 0
}

check_private_file "$ETC/config.toml" config
check_private_file "$ETC/relay.env" token
check_healthz
check_qad
check_cert
check_version

if [ "$failed" -ne 0 ]; then
    echo "healthcheck: $failed 项失败"
    exit 1
fi
echo "healthcheck: OK"
