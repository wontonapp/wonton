#!/usr/bin/env bash
# 本地开发用的 Tuwunel（Matrix 服务器）：一条命令起停，并模拟核心为设备和 Bot 签发 JWT 登录。
#
#   dev.sh up              首次生成核心的 Ed25519 签发密钥，启动并等到健康
#   dev.sh down            停止，保留数据
#   dev.sh reset           停止并删除数据（密钥保留）
#   dev.sh status | logs   查看状态 / 跟随日志
#   dev.sh jwt <用户名>     签发一个 2 分钟内有效的登录 JWT
#   dev.sh login <用户名>   用 JWT 登录一个新设备，输出 user_id、device_id、access_token
#
# 密钥在 .secrets/（已忽略，不提交）。签发需要 OpenSSL 3 及以上；端口用 WONTON_DEV_MATRIX_PORT 改。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
secrets="$here/.secrets"
private_key="$secrets/jwt-ed25519.pem"
public_key="$secrets/jwt-ed25519.pub.pem"
base_url="http://127.0.0.1:${WONTON_DEV_MATRIX_PORT:-8008}"

die() {
  echo "$*" >&2
  exit 1
}

require_openssl3() {
  local version
  version="$(openssl version 2>/dev/null || true)"
  [[ "$version" =~ ^OpenSSL\ [3-9] ]] ||
    die "需要 OpenSSL 3 及以上（当前：${version:-未安装}）。macOS 可以 brew install openssl@3。"
}

ensure_keys() {
  [[ -f "$private_key" && -f "$public_key" ]] && return
  require_openssl3
  mkdir -p "$secrets"
  chmod 700 "$secrets"
  openssl genpkey -algorithm ed25519 -out "$private_key"
  openssl pkey -in "$private_key" -pubout -out "$public_key"
  echo "已生成核心的 JWT 签发密钥：$private_key"
}

# 联邦用的自签证书（ADR-0050）：服务器名拿不到公共证书，TLS 只剩传输形式，对端认证靠 X-Matrix 签名。
ensure_tls_cert() {
  [[ -f "$secrets/tls.crt" && -f "$secrets/tls.key" ]] && return
  require_openssl3
  mkdir -p "$secrets"
  chmod 700 "$secrets"
  openssl req -x509 -newkey ed25519 -nodes -days 3650 \
    -keyout "$secrets/tls.key" -out "$secrets/tls.crt" \
    -subj "/CN=wonton.localhost" -addext "subjectAltName=DNS:wonton.localhost" >/dev/null 2>&1
  echo "已生成联邦自签证书：$secrets/tls.crt"
}

compose() {
  [[ -f "$public_key" ]] || die "还没有签发密钥，先运行：$0 up"
  WONTON_DEV_JWT_PUBLIC_KEY="$(cat "$public_key")" docker compose -f "$here/compose.yaml" "$@"
}

b64url() {
  openssl base64 -A | tr '+/' '-_' | tr -d '='
}

# 声明与 tuwunel.toml 的 [global.jwt] 对应：sub 是用户名（localpart），iss、aud 必填，exp 必填。
mint_jwt() {
  local user="$1" now signing_input input_file signature
  [[ "$user" =~ ^[a-z0-9._=/+-]+$ ]] || die "用户名只能包含小写字母、数字和 ._=/+-：$user"
  require_openssl3
  [[ -f "$private_key" ]] || die "还没有签发密钥，先运行：$0 up"
  now="$(date +%s)"
  signing_input="$(printf '{"alg":"EdDSA","typ":"JWT"}' | b64url).$(
    printf '{"sub":"%s","iss":"wonton-core","aud":"tuwunel","iat":%d,"exp":%d}' \
      "$user" "$now" "$((now + 120))" | b64url
  )"
  # Ed25519 是一次性签名，openssl 不能从标准输入读，只能读文件。
  input_file="$(mktemp)"
  printf '%s' "$signing_input" >"$input_file"
  signature="$(openssl pkeyutl -sign -rawin -inkey "$private_key" -in "$input_file" | b64url)"
  rm -f "$input_file"
  printf '%s.%s\n' "$signing_input" "$signature"
}

login() {
  local token
  token="$(mint_jwt "$1")"
  # 本机配了代理环境变量时，curl 会把 127.0.0.1 也送进代理。
  curl -sS --noproxy '*' --fail-with-body -X POST "$base_url/_matrix/client/v3/login" \
    -H 'content-type: application/json' \
    -d "{\"type\":\"org.matrix.login.jwt\",\"token\":\"$token\",\"initial_device_display_name\":\"wonton-dev\"}"
  echo
}

case "${1:-}" in
  up)
    ensure_keys
    ensure_tls_cert
    compose up -d --wait
    echo "Tuwunel 已就绪：$base_url"
    ;;
  down) compose down ;;
  reset) compose down --volumes ;;
  status) compose ps ;;
  logs) compose logs -f tuwunel ;;
  jwt) mint_jwt "${2:?用法：$0 jwt <用户名>}" ;;
  login) login "${2:?用法：$0 login <用户名>}" ;;
  *) die "用法：$0 up|down|reset|status|logs|jwt <用户名>|login <用户名>" ;;
esac
