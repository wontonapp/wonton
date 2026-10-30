#!/usr/bin/env bash
# M0 端到端冒烟：一条命令装好示例插件 → 健康检查通过 → 未登录的请求被拒 →
# 像浏览器一样经核心网关打开插件并自动登录 → 核心预装模型网关，使用方拿到自己的凭据 →
# 已配对设备经隧道打开插件，直接是主人 → 经隧道登录 Matrix，和回声 Bot 加密收发 →
# 装 Hermes，经模型网关和主人加密对话两轮 → 网关代登录打开“模型设置”网页 → 核心重启后恢复 → 卸载干净。
# Hermes 一步要真实模型：开发机上要有在运行的 magpie（容器里经 host.docker.internal:3425 访问），
# 提供 rm/glm-5.3-flash。
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
# 核心镜像和隧道客户端镜像用同一个提交号构建，共用 Dockerfile 里的编译阶段。
WONTON_BUILD_COMMIT="$("$dev" commit)"
export WONTON_BUILD_COMMIT
core="$WONTON_DEV_PROJECT"
browser="$WONTON_DEV_PROJECT-browser"
phone="$WONTON_DEV_PROJECT-phone"
phone_image="wonton-tunnel-client:dev"
echo_image="wonton-echo-bot:dev"
curl_image="curlimages/curl:8.23.0"
core_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
grafana_url="http://app-grafana.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
whoami_url="http://app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
echo_bot="wonton-agent-echo--bot"
magpie="wonton-ai-usemagpie-magpie--magpie"
model_client="wonton-app-model-client--client"
hermes="wonton-com-nousresearch-hermes--gateway"
plugins=(app.grafana app.whoami agent.echo ai.usemagpie.magpie app.model-client com.nousresearch.hermes)

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
# 经手机的本地代理访问，和 App 一样带会话凭据（$credential）。t 像 Android WebView：http 地址直接发绝对形式的
# 请求行；tc 像 iOS WKWebView：每条连接先 CONNECT。两者各用一个 Cookie 罐，与上面的浏览器分开。
t() { docker exec "$browser" curl -sS -c /tmp/phone-jar -b /tmp/phone-jar -x http://127.0.0.1:18008 -U "$credential" "$@"; }
tc() { docker exec "$browser" curl -sS -c /tmp/ios-jar -b /tmp/ios-jar -p -x http://127.0.0.1:18008 -U "$credential" "$@"; }

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

step "模型网关（models）：核心预装 magpie，给使用方签发只属于它的凭据，经 models.wonton.internal 转发"
plugin_state() {
  "$dev" list | python3 -c 'import json, sys
print(next((p["status"]["state"] for p in json.load(sys.stdin)["plugins"] if p["id"] == sys.argv[1]), "absent"))' "$1"
}
for _ in $(seq 180); do [[ "$(plugin_state ai.usemagpie.magpie)" == running ]] && break; sleep 1; done
expect "核心在后台预装了模型网关" "$(plugin_state ai.usemagpie.magpie)" running
client_file="$(mktemp)"
cat >"$client_file" <<'YAML'
services:
  client:
    image: curlimages/curl:8.23.0
    entrypoint: ["sleep", "infinity"]
    environment:
      BASE: ${WONTON_MODEL_BASE_URL}
      KEY: ${WONTON_MODEL_API_KEY}
      MODEL: ${WONTON_MODEL_DEFAULT}
x-wonton:
  id: app.model-client
  version: 0.1.0
  kind: app
  title: { zh: 模型客户端, en: Model client }
  description: { zh: 冒烟用的模型网关使用方, en: Model gateway consumer for the smoke test }
  permissions: [models]
YAML
installed="$("$dev" install "$client_file" --yes 2>/dev/null)" || fail "使用方安装失败：$installed"
rm -f "$client_file"
expect "使用方安装完成" "$(json status.state <<<"$installed")" running
client_env() { docker inspect "$model_client" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
model_key="$(client_env KEY)"
expect "使用方经核心在插件网络里的别名访问模型网关" "$(client_env BASE)" "http://models.wonton.internal:$WONTON_DEV_PORT/v1"
expect "默认模型由网关决定" "$(client_env MODEL)" group/wonton-default
in_client() { docker exec "$model_client" curl -sS -o /dev/null -w '%{http_code}' "$@" "$(client_env BASE)/models"; }
expect "凭据能用" "$(in_client -H "Authorization: Bearer $model_key")" 200
expect "不带凭据被网关拒绝" "$(in_client)" 401
gateway_keys() { docker exec "$magpie" magpie gateway-key list | awk 'NR > 1 { print $2 }'; }
expect "网关里的凭据以使用方命名" "$(gateway_keys)" app.model-client
expect "卸载使用方" "$("$dev" uninstall app.model-client --purge | json data_removed)" true
expect "卸载时在网关里吊销凭据" "$(gateway_keys)" ""
expect "吊销后旧凭据无效" "$(docker run --rm --network "container:$core" "$curl_image" -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $model_key" "http://$magpie:3425/v1/models")" 401

step "已配对设备经隧道打开插件：不用登录，直接是主人"
docker build -q --target tunnel-client --build-arg WONTON_BUILD_COMMIT -t "$phone_image" \
  -f "$here/Dockerfile" "$repo" >/dev/null
server_id="$("$dev" tunnel | json endpoint_id)"
# 手机：隧道客户端示例跑在核心的网络命名空间里，经回环地址直连隧道入口，本地代理在 127.0.0.1:18008，
# 和 App 一样只接受带本次会话凭据的请求。
docker run -d --name "$phone" --network "container:$core" --entrypoint sh "$phone_image" -c \
  "tunnel-client --key /tmp/phone.key id && exec tunnel-client --key /tmp/phone.key proxy --auth \
   --domain $WONTON_DEV_DOMAIN --server $server_id --addr 127.0.0.1:$WONTON_DEV_TUNNEL_PORT \
   --listen 127.0.0.1:18008" >/dev/null
for _ in $(seq 30); do
  docker logs "$phone" 2>/dev/null | grep -q '^本地代理' && break
  sleep 1
done
device_id="$(docker logs "$phone" 2>/dev/null | head -1)"
credential="$(docker logs "$phone" 2>/dev/null | sed -n 's/^会话凭据：//p')"
[[ "$device_id" =~ ^[0-9a-f]{64}$ && -n "$credential" ]] || fail "手机没有启动：$(docker logs "$phone" 2>&1)"
expect "家里服务器以外的地址不进隧道" \
  "$(t -o /dev/null -w '%{http_code}' http://cdn.example.com/app.js)" 403
expect "同机其他程序不带会话凭据，被本地代理拒绝" \
  "$(docker exec "$browser" curl -sS -o /dev/null -w '%{http_code}' -x http://127.0.0.1:18008 "$grafana_url/api/user")" 407
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "未登记的设备被隧道拒绝" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
grep -qi '^x-wonton-tunnel-error: unauthorized' <<<"$reply" || fail "缺少 unauthorized 原因：$reply"
# 局域网里的浏览器用 IP 打开状态页，上面有首次配对码；手机拿它经 wonton/pair/0 成为主人的设备。
lan_page() { docker exec "$browser" curl -sS "http://127.0.0.1:$WONTON_DEV_PORT/"; }
pairing_code="$(lan_page | sed -n 's/.*<code class="pairing-code">\([^<]*\)<\/code>.*/\1/p' | sed 's/\&amp;/\&/g')"
[[ "$pairing_code" == wonton://pair\?* ]] || fail "状态页上没有首次配对码：$(lan_page)"
ok "局域网状态页上有首次配对码"
expect "手机用首次配对码配对" \
  "$(docker exec "$phone" tunnel-client --key /tmp/phone.key pair "$pairing_code" --name 冒烟的手机 | json result)" paired
expect "配对后状态页不再出码" "$(lan_page | grep -c 'pairing-code')" 0
expect "同一个码不能再用" \
  "$(docker exec "$phone" tunnel-client --key /tmp/other.key pair "$pairing_code" | json reason)" used
reply="$(t -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective} %{num_redirects}' "$grafana_url/")"
read -r code final redirects <<<"$reply"
expect "直接落在 Grafana 首页（经过 $redirects 次跳转：只有 OIDC，没有登录和会话交接）" "$code $final" "200 $grafana_url/"
expect "Grafana 里的用户是主人" "$(t "$grafana_url/api/user" | json login)" owner
reply="$(tc -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective}' "$grafana_url/")"
expect "经 CONNECT（iOS WKWebView 的方式）同样直接进入 Grafana" "$reply" "200 $grafana_url/"
expect "经 CONNECT 时用户也是主人" "$(tc "$grafana_url/api/user" | json login)" owner
! docker exec "$browser" grep -q 'wonton_' /tmp/phone-jar /tmp/ios-jar || fail "经隧道时网关不应发会话 Cookie"
ok "手机没有拿到任何网关 Cookie"
expect "管理 API 不接受隧道身份" \
  "$(t -o /dev/null -w '%{http_code}' "$core_url/_wonton/api/plugins")" 401

step "已配对设备经隧道登录 Matrix（ADR-0009 的 App 接口）"
matrix_url="http://matrix.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
login="$(t -X POST "$core_url/_wonton/matrix/login-token")"
expect "登录令牌给出 homeserver 地址" "$(json homeserver_url <<<"$login")" "$matrix_url"
expect "令牌只能登录主人" "$(json user_id <<<"$login")" "@owner:$WONTON_DEV_DOMAIN"
body="$(python3 -c 'import json, sys
print(json.dumps({"type": sys.argv[1], "token": sys.argv[2], "initial_device_display_name": "wonton smoke"}))' \
  "$(json login_type <<<"$login")" "$(json token <<<"$login")")"
access="$(t -X POST -H 'content-type: application/json' --data-binary "$body" \
  "$matrix_url/_matrix/client/v3/login" | json access_token)"
expect "经 matrix.<基础域名> 登录 Tuwunel，设备属于主人" \
  "$(t -H "Authorization: Bearer $access" "$matrix_url/_matrix/client/v3/account/whoami" | json user_id)" \
  "@owner:$WONTON_DEV_DOMAIN"
expect "局域网浏览器（有核心会话）拿不到登录令牌" \
  "$(b -o /dev/null -w '%{http_code}' -X POST "$core_url/_wonton/matrix/login-token")" 401
expect "局域网进不了 Matrix" "$(b -o /dev/null -w '%{http_code}' "$matrix_url/_matrix/client/versions")" 403
expect "管理接口不转发" "$(t -o /dev/null -w '%{http_code}' "$matrix_url/_synapse/admin/v1/register")" 404
expect "绕路到管理接口的路径被挡住" \
  "$(t --path-as-is -o /dev/null -w '%{http_code}' "$matrix_url/_matrix/client/../../_synapse/admin/v1/register")" 400
# Megolm 会话 ID 里的 `/` 编成 %2F：按会话取密钥备份的请求要原样转给 Tuwunel，由它作答。
reply="$(t -o /dev/null -D - -H "Authorization: Bearer $access" \
  "$matrix_url/_matrix/client/v3/room_keys/keys/!nope:$WONTON_DEV_DOMAIN/ab%2Fcd?version=1")"
! grep -qi '^x-wonton-error' <<<"$reply" || fail "网关挡下了会话 ID 带 %2F 的请求：$reply"
ok "会话 ID 带 %2F 的密钥备份请求转给了 Tuwunel（HTTP $(head -1 <<<"$reply" | cut -d' ' -f2)）"

step "回声 Bot（matrix.bot）：核心代它登录、建好加密私聊，手机经隧道和它加密收发"
docker build -q --target echo-bot --build-arg WONTON_BUILD_COMMIT -t "$echo_image" \
  -f "$here/Dockerfile" "$repo" >/dev/null
install_echo() {
  local installed
  installed="$("$dev" install "$repo/plugins/examples/echo-bot/docker-compose.yml" --yes 2>/dev/null)" ||
    fail "回声 Bot 安装失败：$installed"
  expect "回声 Bot 安装完成，同步上了 Matrix" \
    "$(json status.state <<<"$installed") $(json health <<<"$installed")" "running healthy"
}
# 核心注入 Bot 容器的变量。
bot_env() { docker inspect "$echo_bot" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
owner_api() { t -H "Authorization: Bearer $access" "$matrix_url/_matrix/client/v3$1"; }
# 主人 m.direct 里和这个用户的私聊，一行一个。
direct_rooms() {
  owner_api "/user/@owner:$WONTON_DEV_DOMAIN/account_data/m.direct" |
    python3 -c 'import json, sys; print("\n".join(json.load(sys.stdin).get(sys.argv[1], [])))' "$1"
}
install_echo
bot_user="$(bot_env WONTON_MATRIX_USER_ID)"
bot_token="$(bot_env WONTON_MATRIX_ACCESS_TOKEN)"
expect "Bot 有自己的账号" "$bot_user" "@echo:$WONTON_DEV_DOMAIN"
expect "Bot 经核心在插件网络里的别名访问 Matrix" \
  "$(bot_env WONTON_MATRIX_HOMESERVER)" "http://matrix.wonton.internal:$WONTON_DEV_PORT"
expect "Bot 的显示名" "$(owner_api "/profile/$bot_user/displayname" | json displayname)" 回声
home_room="$(direct_rooms "$bot_user")"
[[ "$home_room" == '!'* && "$home_room" != *$'\n'* ]] || fail "主人的 m.direct 里应有一个和 Bot 的私聊：$home_room"
ok "主房间记在主人的 m.direct 里"
expect "主人和 Bot 都已经在主房间里" \
  "$(owner_api "/rooms/$home_room/joined_members" | python3 -c 'import json, sys; print(" ".join(sorted(json.load(sys.stdin)["joined"])))')" \
  "$bot_user @owner:$WONTON_DEV_DOMAIN"
expect "主房间是加密的" "$(owner_api "/rooms/$home_room/state/m.room.encryption/" | json algorithm)" m.megolm.v1.aes-sha2
# 主人的一台新设备（echo-bot probe）经手机的本地代理登录，发一条加密消息，等 Bot 的加密回声。
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
probe="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
  --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$home_room" \
  --bot "$bot_user" 2>&1)" || fail "加密收发失败：$probe"
ok "经隧道发出加密消息，收到 Bot 的加密回声"
report="$("$dev" uninstall agent.echo)"
expect "卸载时吊销 Bot 的设备" "$(json bot.devices_removed <<<"$report") $(json bot.deactivated <<<"$report")" "1 false"
expect "Bot 的令牌立即失效" \
  "$(t -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $bot_token" "$matrix_url/_matrix/client/v3/account/whoami")" 401
install_echo
expect "不删数据重装：同一个账号、同一个主房间" \
  "$(bot_env WONTON_MATRIX_USER_ID) $(direct_rooms "$bot_user")" "$bot_user $home_room"
report="$("$dev" uninstall agent.echo --purge)"
expect "删除数据时停用 Bot 账号" "$(json bot.deactivated <<<"$report")" true
members="$(owner_api "/rooms/$home_room/joined_members")"
expect "停用的 Bot 退出了主房间" \
  "$(python3 -c 'import json, sys; print(sys.argv[1] in json.load(sys.stdin)["joined"])' "$bot_user" <<<"$members")" False

step "Hermes（默认 Agent）：经 magpie 调模型，在加密私聊里和主人对话两轮，用量记在它的凭据名下"
# 模型只经开发机上的 magpie（容器里是 host.docker.internal:3425），不读取、不复制主人 magpie 的配置和密钥。
docker exec "$magpie" magpie provider add upstream url=http://host.docker.internal:3425/v1 key=placeholder \
  models=rm/glm-5.3-flash >/dev/null || fail "magpie 连不上开发机上的 magpie（host.docker.internal:3425）"
docker exec "$magpie" magpie group add wonton-default models=upstream/rm/glm-5.3-flash routing=order >/dev/null
installed="$("$dev" install "$repo/plugins/hermes" --yes 2>/dev/null)" || fail "Hermes 安装失败：$installed"
expect "插件包目录（带 config.yaml）一条命令装好 Hermes，连上了 Matrix" \
  "$(json status.state <<<"$installed") $(json health <<<"$installed")" "running healthy"
hermes_user="@hermes:$WONTON_DEV_DOMAIN"
hermes_env() { docker inspect "$hermes" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
expect "Hermes 是一个 Bot" "$(hermes_env MATRIX_USER_ID)" "$hermes_user"
expect "Hermes 的凭据以它的 Bot 命名" "$(gateway_keys)" com.nousresearch.hermes/default
hermes_room="$(direct_rooms "$hermes_user")"
[[ "$hermes_room" == '!'* ]] || fail "主人的 m.direct 里没有和 Hermes 的私聊：$hermes_room"
jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
probe="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
  --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$hermes_room" --bot "$hermes_user" \
  --reply any --wait 180 --text "你好，用一句话介绍你自己。" --text "我刚才让你做什么？用一句话回答。" 2>&1)" ||
  fail "和 Hermes 的对话失败：$probe"
grep '^第' <<<"$probe" | sed 's/^/   · /'
ok "经隧道和 Hermes 加密对话两轮"
docker exec "$magpie" magpie usage --last 15m --csv | grep -q ',com.nousresearch.hermes/default,' ||
  fail "用量没记在 com.nousresearch.hermes/default 名下"
ok "用量记在 Hermes 的凭据名下"
hermes_token="$(hermes_env MATRIX_ACCESS_TOKEN)"
report="$("$dev" uninstall com.nousresearch.hermes --purge)"
expect "卸载 Hermes：吊销 Bot 的设备、停用账号" \
  "$(json bot.devices_removed <<<"$report") $(json bot.deactivated <<<"$report")" "1 true"
expect "Hermes 的 Matrix 令牌失效" \
  "$(t -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $hermes_token" "$matrix_url/_matrix/client/v3/account/whoami")" 401
expect "网关里 Hermes 的凭据吊销了" "$(gateway_keys)" ""

step "“模型设置”网页（sso: proxy）：主人不用输网页密钥就能打开，密钥不出网关"
magpie_url="http://ai-usemagpie-magpie.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
# 核心生成、注入 magpie 容器的网页密钥，只用来检查它没有出现在任何响应和 Cookie 罐里。
web_key="$(docker inspect "$magpie" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MAGPIE_WEB_KEY=//p')"
[[ "$web_key" =~ ^[0-9a-f]{64}$ ]] || fail "magpie 没有拿到核心生成的网页密钥"
upstream_off() { [[ "$(docker exec "$magpie" magpie providers)" == *'switched off'* ]]; }
# 没有任何 Cookie 的浏览器。
anon() { docker exec "$browser" curl -sS "$@"; }
reply="$(anon -o /dev/null -w '%{http_code} %{redirect_url}' -H 'Accept: text/html' "$magpie_url/")"
expect "没登录：打开页面跳到核心登录" "${reply%% *}" 303
[[ "${reply#* }" == "$core_url/_wonton/sso?to="* ]] || fail "跳转地址不对：${reply#* }"
expect "没登录时知道网页密钥也进不去" \
  "$(anon -o /dev/null -w '%{http_code}' -H "Cookie: magpie_web_3430=$web_key" "$magpie_url/api/settings")" 401
# 局域网浏览器登录过核心（Grafana 一步），换一次插件会话就进去了。
seen="$(b -L -D - -H 'Accept: text/html' -w '\n%{http_code} %{url_effective}' "$magpie_url/")"
expect "局域网浏览器直接打开“模型设置”" "$(tail -1 <<<"$seen")" "200 $magpie_url/"
grep -q '<title>magpie</title>' <<<"$seen" || fail "打开的不是 magpie 的网页"
reply="$(b -D - "$magpie_url/api/providers")"
seen+="$reply"
grep -q '"id":"upstream"' <<<"$reply" || fail "网页上没有命令行配置的上游：$reply"
ok "网页上看得到命令行配置的上游"
# 网页里的修改和它的 fetch 一样带 Origin：同源的放行，别的插件网页发来的按没登录处理，不转发。
page_post() { b -D - -H "Origin: $1" -H 'content-type: application/json' -d '{"id":"upstream"}' "$magpie_url/api/provider/$2"; }
reply="$(page_post "$grafana_url" off)"
expect "别的插件的网页发来的修改被拒" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 401
! upstream_off || fail "跨站请求改动了 magpie 的设置"
seen+="$(page_post "$magpie_url" off)"
upstream_off || fail "网页上关掉的上游在 magpie 里没生效"
seen+="$(page_post "$magpie_url" on)"
! upstream_off || fail "网页上打开的上游在 magpie 里没生效"
ok "在网页上关掉、再打开上游，magpie 里随即生效"
# magpie 自己的登录链接 /?k=<网页密钥> 回一个带密钥的 Set-Cookie：网关把它去掉。
seen+="$(b -D - -o /dev/null "$magpie_url/?k=$web_key")"
! grep -qi '^set-cookie: magpie_web' <<<"$seen" || fail "magpie 设置网页密钥的 Set-Cookie 到了浏览器"
ok "magpie 设置网页密钥的 Set-Cookie 被网关去掉"
reply="$(t -L -D - -H 'Accept: text/html' -w '\n%{http_code} %{num_redirects}' "$magpie_url/")"
expect "已配对设备经隧道直接打开“模型设置”（没有跳转）" "$(tail -1 <<<"$reply")" "200 0"
seen+="$reply"
reply="$(tc -D - -w '\n%{http_code}' "$magpie_url/api/settings")"
expect "经 CONNECT（iOS WKWebView 的方式）同样能读设置" "$(tail -1 <<<"$reply")" 200
seen+="$reply"
! grep -qF "$web_key" <<<"$seen" || fail "响应里出现了网页密钥"
! docker exec "$browser" grep -qF -e "$web_key" -e magpie_web /tmp/jar /tmp/phone-jar /tmp/ios-jar ||
  fail "浏览器或手机的 Cookie 罐里有 magpie 的网页密钥"
ok "网页密钥不在任何响应头、响应体和 Cookie 罐里"

step "吊销手机"
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
# 从 whoami 的插件网络里冒充 Bot 和模型网关的使用方：网关按来源地址认出插件，它两项权限都没申请。
from_whoami() {
  docker run --rm --network wonton-app-whoami "$curl_image" -sS -o /dev/null -D - \
    -H "Host: $1:$WONTON_DEV_PORT" "http://$core:$WONTON_DEV_PORT$2"
}
for internal in "matrix.wonton.internal /_matrix/client/versions" "models.wonton.internal /v1/models"; do
  read -r host path <<<"$internal"
  denied="$(from_whoami "$host" "$path")"
  expect "没申请权限的插件进不了 $host" "$(head -1 <<<"$denied" | cut -d' ' -f2)" 403
  grep -qi '^x-wonton-error: permission_denied' <<<"$denied" || fail "缺少 permission_denied：$denied"
done
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
expect "核心重启后“模型设置”仍免密码打开（代登录的密钥从密钥库读回）" \
  "$(b -L -o /dev/null -w '%{http_code} %{url_effective}' -H 'Accept: text/html' "$magpie_url/")" "200 $magpie_url/"

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
