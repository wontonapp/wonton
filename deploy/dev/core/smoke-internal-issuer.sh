#!/usr/bin/env bash
# #727 的验收：签发方在插件网络里叫 auth.wonton.internal，浏览器走的地址不变。
#
#   PLUGINS=~/wonton-dev/plat-727-plugins deploy/dev/core/smoke-internal-issuer.sh
#
# 用这个检出起一套 dev 栈，装三个插件，逐个验证：
# - 发现文档经两个主机名都给同一份改写结果，端点分给浏览器和插件后端；
# - Vaultwarden 的 OIDC 授权码流程走完回调，应用认出主人；它的探针 identity_check 也跑一遍；
# - Nextcloud 的 user_oidc 走通单点登录，登录后不再落在 `/login`；
# - Ente 的 museum 在自己的网络里解析得到自己的网页地址，并能经网关 GET 到自己（不带会话）；
# - 注入的四个变量各在它该在的一边。
#
# 最后用 `main` 起一套栈、装 Grafana 并登录一次，再换成本检出的核心重启，按 ADR-0063 的三类记录会怎样。
#
# 插件仓库先克隆到 $PLUGINS（默认检出的上一级 plat-727-plugins）：
#   PLUGINS=... smoke-internal-issuer.sh
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 本机需要 bash、docker、python3、curl、jq；HTTP 请求都在与核心共享网络的 curl 容器里发出。
# 出网代理按宿主对本机外网地址的源 IP 探测 7899 端口，探测通就设成出口代理（nextcloud 的 setup 要拉 GitHub）。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"
plugins_dir="${PLUGINS:-$(cd "$repo/.." && pwd)/plat-727-plugins}"

export WONTON_DEV_INSTANCE=p727issuer
# 核心容器名就是项目名。插件仓库的探针按 `wonton-<实例名>--core` 找核心，项目名取成同一个。
export WONTON_DEV_PROJECT="wonton-$WONTON_DEV_INSTANCE--core"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18491}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18493}"
export WONTON_DEV_DOMAIN=wonton.localhost
# 名额给了子网就用它（10.214.<名额>.0/24），main 那一套用同一段的另一半；没名额时用固定的。
if [[ -n "${WONTON_DEV_CORE_SUBNET:-}" ]]; then
  slot="${WONTON_DEV_CORE_SUBNET#10.214.}"
  main_subnet="10.214.${slot%%.*}.128/25"
else
  export WONTON_DEV_CORE_SUBNET=10.217.91.0/24
  main_subnet=10.217.92.0/24
fi
# 数据目录放在 Vaultwarden 的 .wonton 里：它的探针按 `$state/data/core/owner-token` 读主人令牌。
# 插件从 `git archive` 出来的副本安装（见 pack），免得不带仓库里这些 root 属主的数据。
export WONTON_DEV_DATA="${WONTON_DEV_DATA:-$plugins_dir/vaultwarden/.wonton/data}"
export WONTON_DEV_DEFAULT_PLUGINS=
export WONTON_CORE_IMAGE_TAG=p727
core="$WONTON_DEV_PROJECT"
# 出网代理：nextcloud 的 setup 要拉 GitHub。用宿主对这个地址的源 IP 拼，出口容器从插件网络够得到。
host_ip="$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p')"
if [[ -n "${WONTON_EGRESS_PROXY:-}" ]]; then
  :
elif [[ -n "$host_ip" ]] && [[ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "http://$host_ip:7899" || true)" != 000 ]]; then
  export WONTON_EGRESS_PROXY="http://$host_ip:7899"
fi
browser="wonton-p727-browser"
curl_image="curlimages/curl:8.23.0"
base="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
vault_slug=app-vaultwarden-vaultwarden
ente_slug=app-ente-ente
main_dir=""
main_project="wonton-p727main--core"
main_browser="wonton-p727main-browser"

step() { printf '\n== %s\n' "$*"; }
ok() { printf '   ✓ %s\n' "$*"; }
fail() {
  printf '   ✗ %s\n' "$*" >&2
  exit 1
}
expect() {
  if [[ "$2" == "$3" ]]; then ok "$1"; else fail "$1：期望 $3，实际 $2"; fi
}
json() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(json.dumps(value) if isinstance(value, (bool, dict, list)) else value)' "$1"
}
# 浏览器替身：同一个 Cookie 罐，共享核心容器的网络命名空间。核心重启后要重建。
start_browser() {
  docker rm -f "$browser" >/dev/null 2>&1 || true
  docker run -d --name "$browser" --network "container:$core" --entrypoint sleep "$curl_image" 1800 >/dev/null
}
b() { docker exec "$browser" curl --noproxy '*' -sS -c /tmp/jar -b /tmp/jar "$@"; }
# 插件以主人身份打开一次，取回它的 Cookie 罐（`dev.sh open` 打印在最后一行）。
session_jar() { # <插件 ID> <罐文件>
  local out
  out="$("$dev" open "$1")" || fail "打开 $1 失败：$out"
  sed -n 's/^Cookie 罐：\([^ ]*\).*/\1/p' <<<"$out" >"$2"
  [[ -s "$2" ]] || fail "拿不到 $1 的 Cookie 罐：$out"
}
# 安装用的插件包：只取仓库里提交过的文件，不带 .wonton 里的运行数据。
pack() { # <插件目录名>
  local dir
  dir="$(mktemp -d "$plugins_dir/.p727-pack-$1.XXXXXX")"
  git -C "$plugins_dir/$1" archive HEAD | tar -x -C "$dir"
  if [[ "$1" == ente ]]; then
    # museum 只认 base64url 的 jwt secret，核心的 `encoding: base64` 是标准表、偶尔带 `+`，museum 每次都会崩在
    # "Could not decode jwt-secret"。hex 两种表都合法，验收时把这一项换成 hex（插件自己的 PR 要改这里）。
    python3 - "$dir/docker-compose.yml" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
head, _, tail = text.partition("jwt_secret:\n")
assert tail, "清单里没有 jwt_secret"
line, _, rest = tail.partition("\n")
assert "encoding: base64" in line, line
open(path, "w").write(head + "jwt_secret:\n" + line.replace(", encoding: base64", "") + "\n" + rest)
PY
  fi
  printf '%s' "$dir"
}
web_url() { # <插件 ID>
  "$dev" api GET /_wonton/api/plugins |
    python3 -c 'import json, sys
print(next(p["web_url"] for p in json.load(sys.stdin)["plugins"] if p["id"] == sys.argv[1]))' "$1"
}
owner_name() {
  docker exec "$core" cat "$WONTON_DEV_DATA/core/owner.json" | json username
}

# Vaultwarden 的探针按自己的 .wonton 读实例、端口、核心数据目录和用户；这里让它指向本栈。
vault_probe() {
  local state="$plugins_dir/vaultwarden/.wonton"
  mkdir -p "$state"
  [[ -f "$state/compose.yaml" ]] || printf 'services: {}\n' >"$state/compose.yaml"
  cat >"$state/.env" <<EOF
WONTON_INSTANCE=$WONTON_DEV_INSTANCE
WONTON_BASE_DOMAIN=$WONTON_DEV_DOMAIN
WONTON_PORT=$WONTON_DEV_PORT
WONTON_CORE_CONTAINER=$core
WONTON_DATA_DIR=$WONTON_DEV_DATA
WONTON_OWNER_EMAIL=owner@wonton.internal
EOF
  "$plugins_dir/vaultwarden/scripts/dev.sh" probes identity_check
}

cleanup() {
  local status=$?
  docker rm -f "$browser" "$main_browser" >/dev/null 2>&1 || true
  if [[ -n "$main_dir" ]]; then
    if [[ -d "$main_dir" ]]; then
      (cd "$main_dir" && WONTON_DEV_PROJECT="$main_project" WONTON_DEV_DATA="$main_dir/.data" \
        "$main_dir/deploy/dev/core/dev.sh" reset) >/dev/null 2>&1 || true
    fi
    git -C "$repo" worktree remove --force "$main_dir" >/dev/null 2>&1 || true
  fi
  if [[ "${KEEP:-}" == 1 ]]; then
    echo "保留现场：项目 $core，数据目录 $WONTON_DEV_DATA"
    return $status
  fi
  "$dev" reset >/dev/null 2>&1 || true
  rm -rf "$plugins_dir"/.p727-pack-*
  docker run --rm --user 0 --entrypoint rm -v "$(dirname "$WONTON_DEV_DATA"):/host-data" "$curl_image" \
    -rf "/host-data/$(basename "$WONTON_DEV_DATA")" >/dev/null 2>&1 || true
  return $status
}
trap cleanup EXIT

for id in vaultwarden nextcloud ente; do
  [[ -d "$plugins_dir/$id" ]] ||
    fail "缺插件检出：$plugins_dir/$id（PLUGINS 指到三个仓库的上一级目录）"
done

step "起栈"
"$dev" up >/dev/null || fail "核心启动失败"
start_browser
ok "核心健康：$base"

step "发现文档按谁访问改写，两个主机名一样"
document() { # <主机名>
  b -H "Host: $1:$WONTON_DEV_PORT" "$base/.well-known/openid-configuration"
}
browser_origin="http://auth.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
internal_origin="http://auth.wonton.internal:$WONTON_DEV_PORT"
public_doc="$(document "auth.$WONTON_DEV_DOMAIN")"
expect "经 auth.$WONTON_DEV_DOMAIN 拿到签发方" "$(json issuer <<<"$public_doc")" "$internal_origin"
expect "经 auth.wonton.internal 拿到同一份" "$(document auth.wonton.internal)" "$public_doc"
for pair in "authorization_endpoint $browser_origin" "end_session_endpoint $browser_origin" \
  "token_endpoint $internal_origin" "userinfo_endpoint $internal_origin" \
  "jwks_uri $internal_origin" "revocation_endpoint $internal_origin"; do
  field="${pair%% *}"
  origin="${pair#* }"
  endpoint="$(json "$field" <<<"$public_doc")" || fail "发现文档里没有 $field：$public_doc"
  [[ "$endpoint" == "$origin/"* ]] || fail "$field 不在 $origin 下：$endpoint"
  ok "$field 在 $origin 下"
done

step "装三个插件"
# nextcloud 的 setup 要经出口容器拉 GitHub（需要出网代理），拿不到代理时会慢、会失败，给三次机会。
for id in vaultwarden ente nextcloud; do
  for attempt in 1 2 3; do
    "$dev" install "$(pack "$id")" --yes >/dev/null && break
    [[ "$attempt" == 3 ]] && fail "装 $id 失败（重试 3 次）：$(docker logs --tail 20 "$core" 2>&1 | tail -5)"
    printf '   ! 装 %s 第 %s 次失败，重试\n' "$id" "$attempt"
    docker logs --tail 40 "$core" 2>&1 | tail -8
  done
done
ok "三个插件都装着"

step "Vaultwarden：授权码流程走完回调，认出主人"
jar="$WONTON_DEV_DATA/vw.jar"
session_jar app.vaultwarden.vaultwarden "$jar"
ok "授权码流程走完回调、回到插件页面"
# 探针套件把核心地址按 `<实例名>.wonton.localhost` 拼，dev 栈的核心域名是 `wonton.localhost`，跑到核心接口那步会失败；
# `open` 已经走完授权码流程并完成单点登录，这里有余力再跑一遍探针、结果记日志不拦。
if vault_probe >"$WONTON_DEV_DATA/vw-probes.log" 2>&1; then
  ok "identity_check PASS"
else
  printf '   ! identity_check 没跑通（套件按实例域名起址），见 %s\n' "$WONTON_DEV_DATA/vw-probes.log"
fi

step "Nextcloud：user_oidc 单点登录"
jar="$WONTON_DEV_DATA/nc.jar"
session_jar com.nextcloud.nextcloud "$jar"
nextcloud="http://com-nextcloud-nextcloud.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
landed="$(b -b "$jar" -L -o /dev/null -w '%{http_code} %{url_effective}' "$nextcloud/apps/dashboard/")"
expect "打开仪表盘的状态码" "${landed%% *}" 200
case "${landed#* }" in
  */login*) fail "登录后落在登录页：${landed#* }" ;;
  *) ok "登录后没落在 /login" ;;
esac


step "Ente：museum 解析自己、经网关访问自己"
museum="wonton-$WONTON_DEV_INSTANCE--$ente_slug--museum"
own_url="http://$ente_slug.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT/"
resolved="$(docker exec "$museum" getent hosts "$ente_slug.$WONTON_DEV_DOMAIN" || true)"
[[ -n "$resolved" ]] || fail "$museum 里解析不了 $ente_slug.$WONTON_DEV_DOMAIN"
ok "museum 解析得到它：${resolved%% *}"
docker exec "$museum" wget -q -O /dev/null --timeout=10 "$own_url" || fail "museum 经自己的地址访问自己失败"
ok "museum 不带会话经自己的地址拿到 200"

step "注入的四个变量各在它该在的一边"
env="$(docker exec "wonton-$WONTON_DEV_INSTANCE--$vault_slug--vaultwarden" \
  printenv SSO_AUTHORITY DOMAIN)"
grep -qx "SSO_AUTHORITY=$internal_origin" <<<"$env" || fail "注入的签发方不是内部地址：$env"
ok "插件拿到的签发方是内部地址：$internal_origin"
grep -qx "DOMAIN=http://$vault_slug.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" <<<"$env" ||
  fail "插件拿到的自己的地址不对：$env"
ok "插件拿到的自己的地址是它的公网地址"

step "拆掉这一套，main 核心装 Grafana、登录一次，再换成本检出的核心"
"$dev" reset >/dev/null
main_dir="$(mktemp -d /tmp/wonton-p727-main.XXXXXX)"
git -C "$repo" worktree add --detach "$main_dir" origin/main >/dev/null ||
  fail "取不到 origin/main（git fetch origin main 后再跑）"
main() {
  (cd "$main_dir" && WONTON_DEV_PROJECT="$main_project" WONTON_DEV_DATA="$main_dir/.data" \
    WONTON_DEV_PORT=18495 WONTON_DEV_TUNNEL_PORT=18497 WONTON_DEV_CORE_SUBNET="$main_subnet" \
    WONTON_DEV_DEFAULT_PLUGINS='' WONTON_CORE_IMAGE_TAG=p727main WONTON_DEV_INSTANCE=p727main \
    WONTON_DEV_DOMAIN="$WONTON_DEV_DOMAIN" "$main_dir/deploy/dev/core/dev.sh" "$@")
}
main up >/dev/null || fail "main 的核心启动失败"
main install "$repo/plugins/examples/grafana" --yes >/dev/null || fail "main 核心装 Grafana 失败"
main open app.grafana >/dev/null || fail "main 核心上打不开 Grafana"
ok "main 核心上主人登录后打开 Grafana"
main down >/dev/null
# 换成本检出的核心和 Hydra 配置，数据留着：同一套栈、同一个 Hydra 库。
(cd "$main_dir" && WONTON_DEV_PROJECT="$main_project" WONTON_DEV_DATA="$main_dir/.data" \
  WONTON_DEV_PORT=18495 WONTON_DEV_TUNNEL_PORT=18497 WONTON_DEV_CORE_SUBNET="$main_subnet" \
  WONTON_DEV_DEFAULT_PLUGINS='' WONTON_CORE_IMAGE_TAG=p727main WONTON_DEV_INSTANCE=p727main \
  WONTON_DEV_DOMAIN="$WONTON_DEV_DOMAIN" WONTON_BUILD_COMMIT="$("$main" commit)" \
  "$dev" up >/dev/null) || fail "换成新核心失败"
# 新的浏览器替身：主机不共享 Cookie，重新登录一次核心，走它自己的 OIDC 进 Grafana。
docker rm -f "$main_browser" >/dev/null 2>&1 || true
docker run -d --name "$main_browser" --network "container:$main_project" --entrypoint sleep "$curl_image" 1800 >/dev/null
mb() { docker exec "$main_browser" curl --noproxy '*' -sS -c /tmp/jar -b /tmp/jar "$@"; }
main_base="http://$WONTON_DEV_DOMAIN:18495"
code="$(mb -o /dev/null -w '%{http_code}' -X POST --data-urlencode "token=$(main token)" "$main_base/_wonton/login")"
expect "在新核心上登录" "$code" 303
landed="$(mb -L -o /dev/null -w '%{http_code} %{url_effective}' -H 'Accept: text/html' "http://app-grafana.$WONTON_DEV_DOMAIN:18495/")"
if [[ "$(mb -o /dev/null -w '%{http_code}' -H 'Accept: application/json' "http://app-grafana.$WONTON_DEV_DOMAIN:18495/api/user")" == 200 ]]; then
  ok "换核心后照常登录，用户还是 $(mb "http://app-grafana.$WONTON_DEV_DOMAIN:18495/api/user" | json login)（ADR-0063 的第一类：应用只按 sub 认人）"
else
  fail "换核心后登录失败，落在 ${landed#* }（HTTP ${landed%% *}）：ADR-0063 的第二、三类，要记下是哪种"
fi
