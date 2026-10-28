#!/usr/bin/env bash
# M0 端到端冒烟：一条命令装好示例插件 → 健康检查通过 → 未登录的请求被拒 →
# 像浏览器一样经核心网关打开插件并自动登录 → 已配对设备经隧道打开插件，直接是主人 →
# 核心重启后恢复 → 卸载干净。
#
#   deploy/dev/core/smoke.sh                          本机 Docker
#   DOCKER_HOST=ssh://npchy deploy/dev/core/smoke.sh  远端 Linux 主机（远端只需要 Docker）
#
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 本机需要 bash、docker、python3；HTTP 请求都在与核心共享网络的 curl 容器里发出。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"

export WONTON_DEV_PROJECT=wonton-smoke
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18481}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18483}"
export WONTON_DEV_DOMAIN=wonton.localhost
export WONTON_DEV_DATA="/tmp/wonton-smoke-$(date +%s)-$$"
core="$WONTON_DEV_PROJECT"
browser="$WONTON_DEV_PROJECT-browser"
phone="$WONTON_DEV_PROJECT-phone"
phone_image="wonton-tunnel-client:dev"
curl_image="curlimages/curl:8.23.0"
core_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
grafana_url="http://app-grafana.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
whoami_url="http://app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
plugins=(app.grafana app.whoami)

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
# 浏览器：同一个 Cookie 罐，跨请求保持会话。它共享核心容器的网络命名空间，核心重启后要重建。
start_browser() {
  docker rm -f "$browser" >/dev/null 2>&1 || true
  docker run -d --name "$browser" --network "container:$core" --entrypoint sleep "$curl_image" 1800 >/dev/null
}
b() { docker exec "$browser" curl -sS -c /tmp/jar -b /tmp/jar "$@"; }
# 经手机的本地代理访问：所有连接都交给代理，Host 和地址原样保留；Cookie 罐与上面的浏览器分开。
t() { docker exec "$browser" curl -sS -c /tmp/phone-jar -b /tmp/phone-jar --connect-to ::127.0.0.1:18008 "$@"; }

labeled() {
  local id="$1"
  {
    docker ps -aq --filter "label=wonton.plugin=$id"
    docker network ls -q --filter "label=wonton.plugin=$id"
    docker volume ls -q --filter "label=wonton.plugin=$id"
  } | tr -d '[:space:]'
}

cleanup() {
  local status=$?
  if [[ "${KEEP:-}" == 1 ]]; then
    echo "保留现场：项目 $core，数据目录 $WONTON_DEV_DATA（清理：WONTON_DEV_PROJECT=$core WONTON_DEV_DATA=$WONTON_DEV_DATA $dev reset）"
    return
  fi
  docker rm -f "$browser" "$phone" >/dev/null 2>&1 || true
  # 中途失败时插件可能还在：先删插件容器，再停核心，最后删插件网络和卷。
  local id resource
  for id in "${plugins[@]}"; do
    for resource in $(docker ps -aq --filter "label=wonton.plugin=$id"); do docker rm -f "$resource" >/dev/null; done
  done
  "$dev" reset >/dev/null 2>&1 || true
  for id in "${plugins[@]}"; do
    for resource in $(docker network ls -q --filter "label=wonton.plugin=$id"); do docker network rm "$resource" >/dev/null; done
    for resource in $(docker volume ls -q --filter "label=wonton.plugin=$id"); do docker volume rm "$resource" >/dev/null; done
  done
  docker run --rm --user 0 --entrypoint rm -v /tmp:/host-tmp "$curl_image" \
    -rf "/host-tmp/$(basename "$WONTON_DEV_DATA")" >/dev/null 2>&1 || true
  if [[ $status == 0 ]]; then echo "已清理"; else echo "冒烟失败，已清理（KEEP=1 可保留现场）" >&2; fi
}

step "预检"
docker version --format '   Docker {{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}'
for id in "${plugins[@]}"; do
  [[ -z "$(labeled "$id")" ]] || fail "Docker 上已经有插件 $id 的资源，先卸载它再跑冒烟"
done
ok "没有残留的示例插件资源"
trap cleanup EXIT

step "启动核心和身份提供方（项目 $core，端口 $WONTON_DEV_PORT）"
"$dev" up >/dev/null
start_browser
ok "核心健康：$core_url"

step "一条命令安装 Grafana 示例插件"
started=$(date +%s)
installed="$("$dev" install "$repo/plugins/examples/grafana/docker-compose.yml" --yes 2>/dev/null)"
install_seconds=$(($(date +%s) - started))
expect "安装完成且健康检查通过（${install_seconds}s）" "$(json status.state <<<"$installed")" running
expect "网页地址" "$(json url <<<"$installed")" "$grafana_url"

step "隔离"
expect "插件容器不向宿主暴露端口" \
  "$(docker inspect -f '{{len .HostConfig.PortBindings}}' wonton-app-grafana--grafana)" 0
expect "插件容器只在自己的网络里" \
  "$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' wonton-app-grafana--grafana)" "wonton-app-grafana "
aliases="$(docker inspect -f '{{json (index .NetworkSettings.Networks "wonton-app-grafana").Aliases}}' "$core")"
[[ "$aliases" == *"\"auth.$WONTON_DEV_DOMAIN\""* ]] || fail "核心没有以 auth.$WONTON_DEV_DOMAIN 加入插件网络：$aliases"
ok "核心加入插件网络，别名 auth.$WONTON_DEV_DOMAIN（插件经它访问身份提供方）"
if docker exec wonton-app-grafana--grafana wget -q -T 3 -O /dev/null http://hydra:4445/health/ready 2>/dev/null; then
  fail "插件能访问身份提供方的管理端口"
fi
ok "插件访问不到身份提供方的管理端口"

step "未登录的请求被拒"
reply="$(b -o /dev/null -w '%{http_code} %{redirect_url}' -H 'Accept: text/html' "$grafana_url/")"
expect "页面请求跳到核心换取会话" "${reply%% *}" 303
[[ "${reply#* }" == "$core_url/_wonton/sso?to="* ]] || fail "跳转地址不对：${reply#* }"
expect "API 请求返回 401" "$(b -o /dev/null -w '%{http_code}' -H 'Accept: application/json' "$grafana_url/api/user")" 401
expect "伪造的插件会话无效" \
  "$(b -o /dev/null -w '%{http_code}' -H 'Cookie: wonton_app=forged' "$grafana_url/api/user")" 401

step "像浏览器一样打开插件：登录核心一次，插件自动登录"
expect "主人令牌登录核心" \
  "$(b -o /dev/null -w '%{http_code}' -X POST --data-urlencode "token=$("$dev" token)" "$core_url/_wonton/login")" 303
reply="$(b -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective} %{num_redirects}' "$grafana_url/")"
read -r code final redirects <<<"$reply"
expect "落在 Grafana 首页（经过 $redirects 次跳转：会话交接 + OIDC）" "$code $final" "200 $grafana_url/"
user="$(b "$grafana_url/api/user")"
expect "Grafana 里的用户是主人" "$(json login <<<"$user")" owner
expect "邮箱来自核心签发的 ID 令牌" "$(json email <<<"$user")" "owner@$WONTON_DEV_DOMAIN"
expect "按 groups 声明映射为管理员" "$(b "$grafana_url/api/user/orgs" | json 0.role)" Admin

step "已配对设备经隧道打开插件：不用登录，直接是主人"
docker build -q --target tunnel-client -t "$phone_image" -f "$here/Dockerfile" "$repo" >/dev/null
server_id="$("$dev" tunnel | json endpoint_id)"
# 手机：隧道客户端示例跑在核心的网络命名空间里，经回环地址直连隧道入口，本地代理在 127.0.0.1:18008。
docker run -d --name "$phone" --network "container:$core" --entrypoint sh "$phone_image" -c \
  "tunnel-client --key /tmp/phone.key id && exec tunnel-client --key /tmp/phone.key proxy \
   --server $server_id --addr 127.0.0.1:$WONTON_DEV_TUNNEL_PORT --listen 127.0.0.1:18008" >/dev/null
for _ in $(seq 30); do
  docker logs "$phone" 2>/dev/null | grep -q '^本地代理' && break
  sleep 1
done
device_id="$(docker logs "$phone" 2>/dev/null | head -1)"
[[ "$device_id" =~ ^[0-9a-f]{64}$ ]] || fail "手机没有启动：$(docker logs "$phone" 2>&1)"
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "未登记的设备被隧道拒绝" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
grep -qi '^x-wonton-tunnel-error: unauthorized' <<<"$reply" || fail "缺少 unauthorized 原因：$reply"
expect "主人登记这台设备" "$("$dev" allow "$device_id" | json added)" true
reply="$(t -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective} %{num_redirects}' "$grafana_url/")"
read -r code final redirects <<<"$reply"
expect "直接落在 Grafana 首页（经过 $redirects 次跳转：只有 OIDC，没有登录和会话交接）" "$code $final" "200 $grafana_url/"
expect "Grafana 里的用户是主人" "$(t "$grafana_url/api/user" | json login)" owner
! docker exec "$browser" grep -q 'wonton_' /tmp/phone-jar || fail "经隧道时网关不应发会话 Cookie"
ok "手机没有拿到任何网关 Cookie"
expect "管理 API 不接受隧道身份" \
  "$(t -o /dev/null -w '%{http_code}' "$core_url/_wonton/api/plugins")" 401
"$dev" revoke "$device_id" >/dev/null
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "吊销后立即被拒" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
docker rm -f "$phone" >/dev/null

step "没有 OIDC 的插件：网关转发并剥离网关 Cookie"
installed="$("$dev" install "$repo/plugins/examples/whoami/docker-compose.yml" --yes 2>/dev/null)"
expect "whoami 安装完成" "$(json status.state <<<"$installed")" running
echoed="$(b -L -H 'Accept: text/html' -H 'X-Forwarded-For: 6.6.6.6' "$whoami_url/")"
grep -qi "^X-Forwarded-Host: app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" <<<"$echoed" || fail "缺少 X-Forwarded-Host：$echoed"
! grep -q 'wonton_app=\|wonton_session=' <<<"$echoed" || fail "网关 Cookie 泄露给了插件：$echoed"
! grep -q '6.6.6.6' <<<"$echoed" || fail "伪造的 X-Forwarded-For 被转发：$echoed"
ok "插件收到转发头，收不到网关 Cookie 和伪造的转发头"
expect "whoami 卸载" "$("$dev" uninstall app.whoami --purge | json data_removed)" true

step "核心重启后恢复插件"
docker restart "$core" >/dev/null
for _ in $(seq 60); do
  [[ "$(docker inspect -f '{{.State.Health.Status}}' "$core")" == healthy ]] && break
  sleep 1
done
expect "核心重新健康" "$(docker inspect -f '{{.State.Health.Status}}' "$core")" healthy
expect "插件状态恢复为运行中" "$("$dev" list | json plugins.0.status.state)" running
start_browser
b -o /dev/null -X POST --data-urlencode "token=$("$dev" token)" "$core_url/_wonton/login"
# 新浏览器没有任何 Cookie：打开首页会重新走一遍会话交接和 OIDC。
expect "重新登录后打开插件仍自动登录" \
  "$(b -L -o /dev/null -w '%{http_code}' -H 'Accept: text/html' "$grafana_url/")" 200
expect "核心重启后插件仍认出主人" "$(b "$grafana_url/api/user" | json login)" owner

step "卸载干净"
started=$(date +%s)
report="$("$dev" uninstall app.grafana --purge)"
uninstall_seconds=$(($(date +%s) - started))
expect "删除了容器（${uninstall_seconds}s）" "$(json containers <<<"$report")" '["wonton-app-grafana--grafana"]'
expect "删除了插件网络" "$(json networks <<<"$report")" '["wonton-app-grafana"]'
expect "删除了 OIDC 客户端" "$(json oidc_client_removed <<<"$report")" true
expect "删除了应用数据" "$(json data_removed <<<"$report")" true
expect "Docker 上没有带插件标签的容器、网络、卷" "$(labeled app.grafana)" ""
docker exec "$core" test ! -e "$WONTON_DEV_DATA/apps/app-grafana" || fail "应用数据目录还在"
ok "应用数据目录已删除"
if docker exec "$core-hydra-1" wget -q -O /dev/null http://127.0.0.1:4445/admin/clients/app.grafana 2>/dev/null; then
  fail "身份提供方里还有 app.grafana 客户端"
fi
ok "身份提供方里没有 app.grafana 客户端"
expect "网关不再路由这个插件" \
  "$(b -o /dev/null -w '%{http_code}' -H 'Accept: application/json' "$grafana_url/")" 404

printf '\n端到端冒烟通过：安装 %ss，卸载 %ss\n' "$install_seconds" "$uninstall_seconds"
