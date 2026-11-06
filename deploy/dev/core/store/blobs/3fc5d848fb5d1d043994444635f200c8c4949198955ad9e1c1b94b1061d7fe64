#!/bin/sh
# 一次性服务：把 Navidrome 的内置 admin 换成用户自己的账号，装上就可用，不用等第一次打开网页。
# Navidrome 0.64.2 的实测行为：ND_DEVAUTOCREATEADMINPASSWORD 只在空库时建 admin；/auth/login 返回 token
# 和 admin 的 id；/api/user 带 x-nd-authorization 能建号；/rest/ping.view 用 u/p 校验。
# 已经prov成功过（/data/.owner-ready）就什么都不做。
set -eu

base=http://navidrome:4533
ready=/data/.owner-ready
[ -f "$ready" ] || true

json() { wget -qO- --header='Content-Type: application/json' "$@"; }

until wget -q -O /dev/null "$base/ping"; do sleep 1; done
[ -f "$ready" ] && exit 0

token_of() { sed -n 's/.*"token":"\([^"]*\)".*/\1/p'; }
id_of() { sed -n 's/.*"id":"\([^"]*\)".*/\1/p'; }

admin_json="$(json --post-data="{\"username\":\"admin\",\"password\":\"$WONTON_SECRET_CLIENT_PASSWORD\"}" "$base/auth/login")"
admin_token="$(printf '%s' "$admin_json" | token_of)"
admin_id="$(printf '%s' "$admin_json" | id_of)"
[ -n "$admin_token" ] && [ -n "$admin_id" ] || { echo "admin 登录失败：$admin_json" >&2; exit 1; }

# 账号已存在时（重装没删数据、但 ready 文件丢了）接口回 409，接着用新账号登录就行。
json --header="x-nd-authorization: Bearer $admin_token" \
  --post-data="{\"userName\":\"$WONTON_OWNER_NAME\",\"name\":\"$WONTON_OWNER_NAME\",\"isAdmin\":true,\"password\":\"$WONTON_SECRET_CLIENT_PASSWORD\"}" \
  "$base/api/user" >/dev/null 2>&1 || true

owner_json="$(json --post-data="{\"username\":\"$WONTON_OWNER_NAME\",\"password\":\"$WONTON_SECRET_CLIENT_PASSWORD\"}" "$base/auth/login")"
owner_token="$(printf '%s' "$owner_json" | token_of)"
[ -n "$owner_token" ] || { echo "新账号登录失败：$owner_json" >&2; exit 1; }

if [ "$admin_id" != "$(printf '%s' "$owner_json" | id_of)" ]; then
  wget -qO- --header="x-nd-authorization: Bearer $owner_token" -X DELETE "$base/api/user/$admin_id" >/dev/null 2>&1 || true
fi

wget -qO- "$base/rest/ping.view?u=$WONTON_OWNER_NAME&p=$WONTON_SECRET_CLIENT_PASSWORD&v=1.16.1&c=wonton&f=json" |
  grep -q '"status":"ok"' || { echo "Subsonic 校验失败" >&2; exit 1; }

: >"$ready"
echo "账号 $WONTON_OWNER_NAME 已就绪"
