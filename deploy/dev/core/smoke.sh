#!/usr/bin/env bash
# M0 端到端冒烟：一条命令装好示例插件 → 健康检查通过 → 未登录的请求被拒 →
# 像浏览器一样经核心网关打开插件并自动登录 → 核心预装模型网关，使用方拿到自己的凭据 →
# 已配对设备经隧道打开插件，直接是主人 → 经隧道登录 Matrix，和回声 Bot 加密收发 →
# 核心自己装好的默认 Agent Hermes 就绪后出现在主人的私聊里：没有模型时提示去“模型设置”，加了 provider 后经模型网关和
# 主人加密对话两轮 → 网关代登录打开“模型设置”网页 → 核心重启后恢复、不装回卸掉的 Hermes → 卸载干净。
# Hermes 一步要真实模型：开发机上要有在运行的 magpie。冒烟的 magpie 容器直连宿主 3425，
# 凭据是冒烟自己新建的一次性网关凭据：限定到这次的模型，退出时按建 key 时记下的 id 删掉
# （失败退出也删），凭据不进日志、不写进仓库。
# 模型名经 WONTON_SMOKE_MODEL 传入：开发机 magpie 里一个能对话的模型。
#
#   deploy/dev/core/smoke.sh    本机 Docker；要设 WONTON_SMOKE_MODEL，见上
#   DOCKER_HOST=ssh://$DEV_HOST deploy/dev/core/smoke.sh  远端 Linux 主机（远端只需要 Docker，DEV_HOST 是它的 SSH 目标）
#
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 本机需要 bash、docker、python3；HTTP 请求都在与核心共享网络的 curl 容器里发出。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"

export WONTON_DEV_PROJECT=wonton-smoke
# 带实例名：下面的清理只动这个实例的插件，同一台 Docker 上别的核心不受影响。冒烟一次只跑一个。
export WONTON_DEV_INSTANCE=smoke
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
busybox_image="public.ecr.aws/docker/library/busybox:1.37"
core_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
grafana_url="http://app-grafana.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
whoami_url="http://app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
# 插件的网络名；容器名是“网络名--服务名”。
grafana_network="wonton-$WONTON_DEV_INSTANCE--app-grafana"
whoami_network="wonton-$WONTON_DEV_INSTANCE--app-whoami"
magpie_network="wonton-$WONTON_DEV_INSTANCE--ai-usemagpie-magpie"
hermes_network="wonton-$WONTON_DEV_INSTANCE--com-nousresearch-hermes"
grafana="$grafana_network--grafana"
echo_bot="wonton-$WONTON_DEV_INSTANCE--agent-echo--bot"
magpie="$magpie_network--magpie"
model_client="wonton-$WONTON_DEV_INSTANCE--app-model-client--client"
# Hermes 声明了 bots（x-wonton.bots），gateway 是 Bot 服务：容器按 Bot 一份。
hermes="$hermes_network--gateway--default"
plugins=(app.grafana app.whoami agent.echo ai.usemagpie.magpie app.model-client com.nousresearch.hermes app.wonton.files org.navidrome.navidrome)

# shellcheck source=deploy/dev/core/smoke-cleanup.sh
source "$here/smoke-cleanup.sh"

# 构建镜像经 dev.sh：远端主机上经它自己的代理、限制并发（WONTON_DEV_BUILD_PROXY、WONTON_DEV_BUILD_JOBS）。
build_failed() {
  docker buildx version >/dev/null 2>&1 || fail "构建镜像需要 Docker buildx，请安装 Docker buildx 插件后重试"
  fail "$1（详情见上面的输出）"
}
build() { "$dev" build -q "$@" >/dev/null || build_failed "镜像构建失败"; }
# 真实模型：冒烟的 magpie 容器直连宿主上的 magpie（地址见模型段的按 Docker 种类区分），凭据见文件头。
model="${WONTON_SMOKE_MODEL:?未设 WONTON_SMOKE_MODEL：Hermes 一步要真实模型，填开发机 magpie 里能对话的模型名}"

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
# 直连探针忽略 Docker CLI 注入的代理；隧道探针则清空 NO_PROXY，避免 *.localhost 绕过显式的本地代理。
start_browser() {
  docker rm -f "$browser" >/dev/null 2>&1 || true
  docker run -d --name "$browser" --network "container:$core" --entrypoint sleep "$curl_image" 1800 >/dev/null
}
b() { docker exec "$browser" curl --noproxy '*' -sS -c /tmp/jar -b /tmp/jar "$@"; }
# 经手机的本地代理访问，和 App 一样带会话凭据（$credential）。t 像 Android WebView：http 地址直接发绝对形式的
# 请求行；tc 像 iOS WKWebView：每条连接先 CONNECT。两者各用一个 Cookie 罐，与上面的浏览器分开。
t() { docker exec "$browser" curl --noproxy '' -sS -c /tmp/phone-jar -b /tmp/phone-jar -x http://127.0.0.1:18008 -U "$credential" "$@"; }
tc() { docker exec "$browser" curl --noproxy '' -sS -c /tmp/ios-jar -b /tmp/ios-jar -p -x http://127.0.0.1:18008 -U "$credential" "$@"; }

labeled() {
  local id="$1"
  {
    docker ps -aq --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
    docker network ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
    docker volume ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
  } | tr -d '[:space:]'
}


step "预检"
docker version --format '   Docker {{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}'
for id in "${plugins[@]}"; do
  [[ -z "$(labeled "$id")" ]] || fail "Docker 上已经有插件 $id 的资源，先卸载它再跑冒烟"
done
ok "没有残留的示例插件资源"
trap cleanup EXIT

step "启动核心和身份提供方（项目 $core，端口 $WONTON_DEV_PORT）"
# 核心一启动就在后台装 Hermes：从现在起记下它的网关容器的健康状态变化，量它就绪后多久私聊出现。
hermes_events="$(mktemp)"
docker events --filter "container=$hermes" --filter event=health_status --format '{{.TimeNano}} {{.Action}}' \
  >"$hermes_events" 2>/dev/null &
events_pid=$!
"$dev" up >/dev/null || build_failed "核心启动失败"
start_browser
ok "核心健康：$core_url"

step "一条命令安装 Grafana 示例插件"
started=$(date +%s)
installed="$("$dev" install "$repo/plugins/examples/grafana" --yes 2>/dev/null)"
install_seconds=$(($(date +%s) - started))
expect "安装完成且健康检查通过（${install_seconds}s）" "$(json status.state <<<"$installed")" running
expect "网页地址" "$(json web_url <<<"$installed")" "$grafana_url/"

step "隔离"
expect "插件容器不向宿主暴露端口" \
  "$(docker inspect -f '{{len .HostConfig.PortBindings}}' "$grafana")" 0
expect "插件容器只在自己的网络里" \
  "$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' "$grafana")" "$grafana_network "
aliases="$(docker inspect -f "{{json (index .NetworkSettings.Networks \"$grafana_network\").Aliases}}" "$core")"
[[ "$aliases" == *"\"auth.wonton.internal\""* ]] || fail "核心没有以 auth.wonton.internal 加入插件网络：$aliases"
ok "核心加入插件网络，别名 auth.wonton.internal（插件经它访问身份提供方）"
[[ "$aliases" == *"\"app-grafana.$WONTON_DEV_DOMAIN\""* ]] || fail "核心没有以插件自己的地址加入它的网络：$aliases"
ok "核心也以插件自己的地址加入网络，插件能访问自己"
out="$(docker exec "$grafana" wget -S -q -O /dev/null "$grafana_url/" 2>&1 || true)"
[[ "$out" =~ HTTP/[0-9.]+[[:space:]]+[23][0-9][0-9] ]] || fail "插件经自己的地址访问自己失败：$out"
[[ "$out" == *"/_wonton/sso"* ]] && fail "插件经自己的地址访问自己仍要会话：$out"
ok "插件经自己的地址访问自己，不要会话（第一跳是它自己的页面，不是核心的会话交接）"
if docker exec "$grafana" wget -q -T 3 -O /dev/null http://hydra.wonton.internal:4445/health/ready 2>/dev/null; then
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
  "$dev" list | awk -v id="$1" '$1 == id { print $2; found = 1 } END { if (!found) print "absent" }'
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
  spec: '0.4'
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
# 核心同时在后台装默认 Agent，网关里可能已经有 Hermes 的凭据，这里只看使用方的。
expect "网关里的凭据以使用方命名" "$(gateway_keys | grep -cx app.model-client)" 1
expect "卸载使用方" "$("$dev" uninstall app.model-client --purge | json data_removed)" true
expect "卸载时在网关里吊销凭据" "$(gateway_keys | grep -cx app.model-client)" 0
# 吊销后旧凭据换不到上游：网关前面加一个假上游（来什么都回 200 和固定的模型列表、记下收到的凭据），用旧凭据请求，
# 假上游不能收到网关替它配的上游凭据。不断言网关回哪个状态码：magpie 0.1.1327 碰到不认识的凭据不在本地回 401，
# 转发出去回 502（2026-11-07 实测），把客户端自己的凭据原样转给上游也说得通；那是 magpie 自己的行为，
# 这里只测吊销以后旧凭据不能再用网关替使用方配的上游凭据。
fake_upstream="$magpie-smoke-upstream"
fake_key="sk-smoke-upstream-$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')"
fake_sh='auth=
while IFS= read -r line; do
  line=$(printf "%s" "$line" | tr -d "\r")
  [ -z "$line" ] && break
  case "$line" in [Aa]uthorization:*) auth=${line#*: } ;; esac
done
echo "$auth" >>/tmp/auth.log
body="{\"object\":\"list\",\"data\":[{\"id\":\"smoke-fake\",\"object\":\"model\",\"owned_by\":\"smoke\"}]}"
printf "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s" "${#body}" "$body"'
docker run -d --rm --name "$fake_upstream" --network "container:$magpie" -e FAKE_SH="$fake_sh" "$busybox_image" \
  sh -c 'printf "%s\n" "$FAKE_SH" >/fake.sh && chmod +x /fake.sh && while :; do nc -l -p 18099 -e /fake.sh; done' >/dev/null \
  || fail "起不了假上游"
docker exec -e no_proxy=127.0.0.1 -e NO_PROXY=127.0.0.1 "$magpie" magpie provider add smoke-fake url=http://127.0.0.1:18099/v1 key="$fake_key" models=smoke-fake >/dev/null \
  || fail "网关加不上假上游"
docker exec "$fake_upstream" sh -c ': >/tmp/auth.log'
revoked_code="$(docker run --rm --network "container:$core" "$curl_image" --noproxy '*' -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $model_key" "http://$magpie:3425/v1/models")"
expect "吊销后旧凭据换不到网关的上游凭据（网关回 HTTP $revoked_code）" "$(docker exec "$fake_upstream" grep -c "$fake_key" /tmp/auth.log)" 0
docker exec "$magpie" magpie provider rm smoke-fake >/dev/null || fail "删不掉假上游"
docker rm -f "$fake_upstream" >/dev/null

step "已配对设备经隧道打开插件：不用登录，直接是主人"
build --target tunnel-client --build-arg WONTON_BUILD_COMMIT -t "$phone_image" -f "$here/Dockerfile" "$repo"
server_id="$("$dev" tunnel | json endpoint_id)"
# 手机：隧道客户端示例跑在核心的网络命名空间里，经回环地址直连隧道入口，本地代理在 127.0.0.1:18008，
# 和 App 一样只接受带本次会话凭据的请求。私钥放在数据目录：重启手机进程（模拟 App 回前台触发重连，
# 隧道客户端被拒后按 ADR-0032 等 retry_now，不自动重拨）后还是同一个设备。客户端对已存在的空文件不生成
# 私钥，所以先用一次性容器把它生成出来。
phone_key="$WONTON_DEV_DATA/phonekeys/phone.key"
[[ -s "$phone_key" ]] || docker run --rm -v "$WONTON_DEV_DATA/phonekeys:/k" --entrypoint sh "$phone_image" \
  -c "tunnel-client --key /k/phone.key id" >/dev/null
start_phone() {
  docker rm -f "$phone" >/dev/null 2>&1 || true
  docker run -d --name "$phone" --network "container:$core" -v "$phone_key:/tmp/phone.key" --entrypoint sh "$phone_image" -c \
    "tunnel-client --key /tmp/phone.key id && exec tunnel-client --key /tmp/phone.key proxy --auth \
     --domain $WONTON_DEV_DOMAIN --server $server_id --addr 127.0.0.1:$WONTON_DEV_TUNNEL_PORT \
     --listen 127.0.0.1:18008" >/dev/null
  for _ in $(seq 30); do
    # 不用 grep -q：它匹配就退出，docker logs 会吃 SIGPIPE，pipefail 下会杀掉整个脚本。
    docker logs "$phone" 2>/dev/null | grep '^本地代理' >/dev/null && break
    sleep 1
  done
  credential="$(docker logs "$phone" 2>/dev/null | sed -n 's/^会话凭据：//p')"
}
start_phone
device_id="$(docker logs "$phone" 2>/dev/null | sed -n '1p')"
[[ "$device_id" =~ ^[0-9a-f]{64}$ && -n "$credential" ]] || fail "手机没有启动：$(docker logs "$phone" 2>&1)"
expect "家里服务器以外的地址不进隧道" \
  "$(t -o /dev/null -w '%{http_code}' http://cdn.example.com/app.js)" 403
expect "同机其他程序不带会话凭据，被本地代理拒绝" \
  "$(docker exec "$browser" curl --noproxy '' -sS -o /dev/null -w '%{http_code}' -x http://127.0.0.1:18008 "$grafana_url/api/user")" 407
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "未登记的设备被隧道拒绝" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
grep -qi '^x-wonton-tunnel-error: unauthorized' <<<"$reply" || fail "缺少 unauthorized 原因：$reply"
# 局域网里的浏览器用 IP 打开状态页，上面有首次配对码；手机拿它经 wonton/pair/0 成为主人的设备。
lan_page() { docker exec "$browser" curl --noproxy '*' -sS "http://127.0.0.1:$WONTON_DEV_PORT/"; }
pairing_code="$(lan_page | sed -n 's/.*<code class="pairing-code">\([^<]*\)<\/code>.*/\1/p' | sed 's/\&amp;/\&/g')"
[[ "$pairing_code" == wonton://pair\?* ]] || fail "状态页上没有首次配对码：$(lan_page)"
ok "局域网状态页上有首次配对码"
expect "手机用首次配对码配对" \
  "$(docker exec "$phone" tunnel-client --key /tmp/phone.key pair "$pairing_code" --name 冒烟的手机 | json result)" paired
expect "配对后状态页不再出码" "$(lan_page | grep -c 'pairing-code')" 0
expect "同一个码不能再用" \
  "$(docker exec "$phone" tunnel-client --key /tmp/other.key pair "$pairing_code" | json reason)" used
# 配对成功后重启手机进程，模拟 App 回前台触发重连：同一个设备身份，新连接应被接受。
start_phone
[[ -n "$credential" ]] || fail "重连后的手机没有会话凭据：$(docker logs "$phone" 2>&1)"
ok "配对后重连（同一个设备身份）"
reply="$(t -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective} %{num_redirects}' "$grafana_url/")"
read -r code final redirects <<<"$reply"
expect "直接落在 Grafana 首页（经过 $redirects 次跳转：只有 OIDC，没有登录和会话交接）" "$code $final" "200 $grafana_url/"
expect "Grafana 里的用户是主人" "$(t "$grafana_url/api/user" | json login)" owner
reply="$(tc -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective}' "$grafana_url/")"
expect "经 CONNECT（iOS WKWebView 的方式）同样直接进入 Grafana" "$reply" "200 $grafana_url/"
expect "经 CONNECT 时用户也是主人" "$(tc "$grafana_url/api/user" | json login)" owner
! docker exec "$browser" grep -q 'wonton_' /tmp/phone-jar /tmp/ios-jar || fail "经隧道时网关不应发会话 Cookie"
ok "手机没有拿到任何网关 Cookie"
expect "已配对设备能调插件管理接口" "$(t -o /dev/null -w '%{http_code}' "$core_url/_wonton/api/plugins")" 200
expect "插件网页经隧道也调不了" \
  "$(t -o /dev/null -w '%{http_code}' -H "Origin: $grafana_url" "$core_url/_wonton/api/plugins")" 403
expect "devices 接口经隧道也认已配对设备（ADR-0046 起 App 要读设备列表）" \
  "$(t -o /dev/null -w '%{http_code}' "$core_url/_wonton/api/devices")" 200
expect "经隧道的已配对设备能读 tunnel 接口（ADR-0032：App 连上后跟着服务器地址走）" \
  "$(t -o /dev/null -w '%{http_code}' "$core_url/_wonton/api/tunnel")" 200

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
owner_api() { t -H "Authorization: Bearer $access" "$matrix_url/_matrix/client/v3$1"; }
# 主人 m.direct 里和这个用户的私聊，一行一个。
direct_rooms() {
  owner_api "/user/@owner:$WONTON_DEV_DOMAIN/account_data/m.direct" |
    python3 -c 'import json, sys; print("\n".join(json.load(sys.stdin).get(sys.argv[1], [])))' "$1"
}
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
build --target echo-bot --build-arg WONTON_BUILD_COMMIT -t "$echo_image" -f "$here/Dockerfile" "$repo"
install_echo() {
  local installed
  installed="$("$dev" install "$repo/plugins/examples/echo-bot" --yes 2>/dev/null)" ||
    fail "回声 Bot 安装失败：$installed"
  expect "回声 Bot 安装完成，同步上了 Matrix" \
    "$(json status.state <<<"$installed") $(json status.health <<<"$installed")" "running healthy"
}
# 核心注入 Bot 容器的变量。
bot_env() { docker inspect "$echo_bot" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
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
expect "卸载时吊销 Bot 的设备" "$(json bots.0.devices_removed <<<"$report") $(json bots.0.deactivated <<<"$report")" "1 false"
expect "Bot 的令牌立即失效" \
  "$(t -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $bot_token" "$matrix_url/_matrix/client/v3/account/whoami")" 401
install_echo
expect "不删数据重装：同一个账号、同一个主房间" \
  "$(bot_env WONTON_MATRIX_USER_ID) $(direct_rooms "$bot_user")" "$bot_user $home_room"
report="$("$dev" uninstall agent.echo --purge)"
expect "删除数据时停用 Bot 账号" "$(json bots.0.deactivated <<<"$report")" true
members="$(owner_api "/rooms/$home_room/joined_members")"
expect "停用的 Bot 退出了主房间" \
  "$(python3 -c 'import json, sys; print(sys.argv[1] in json.load(sys.stdin)["joined"])' "$bot_user" <<<"$members")" False

step "Hermes（默认 Agent）：核心自己装好，就绪后私聊才出现；没有模型时提示，加了 provider 就能对话"
# 核心启动后在后台装 Hermes（WONTON_DEFAULT_PLUGINS），比主人配对、登录 Matrix 早：冒烟没有手动安装它。
# 本机没有它的镜像时要先拉约 4 GB；装不上时核心会自动重试，插件列表里写着原因。
for _ in $(seq 900); do [[ "$(plugin_state com.nousresearch.hermes)" == running ]] && break; sleep 1; done
[[ "$(plugin_state com.nousresearch.hermes)" == running ]] ||
  fail "核心没有装好 Hermes：$("$dev" list | grep -F com.nousresearch.hermes || echo absent)"
ok "核心在后台装好了 Hermes，没有手动安装"
hermes_user="@hermes:$WONTON_DEV_DOMAIN"
hermes_env() { docker inspect "$hermes" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
expect "Hermes 是一个 Bot" "$(hermes_env MATRIX_USER_ID)" "$hermes_user"
hermes_room="$(direct_rooms "$hermes_user")"
[[ "$hermes_room" == '!'* && "$hermes_room" != *$'\n'* ]] ||
  fail "主人的 m.direct 里应有一个和 Hermes 的私聊：$hermes_room"
ok "主人的 m.direct 里已经有和 Hermes 的私聊"
expect "主人和 Hermes 都已经在私聊里" \
  "$(owner_api "/rooms/$hermes_room/joined_members" | python3 -c 'import json, sys; print(" ".join(sorted(json.load(sys.stdin)["joined"])))')" \
  "$hermes_user @owner:$WONTON_DEV_DOMAIN"
expect "私聊是加密的" "$(owner_api "/rooms/$hermes_room/state/m.room.encryption/" | json algorithm)" m.megolm.v1.aes-sha2
# 主人加入私聊的时间（Tuwunel 的事件时间）和网关容器第一次过健康检查的时间（Docker 事件）在同一台主机上。
ready_ns="$(awk '$2 == "health_status:" && $3 == "healthy" { print $1; exit }' "$hermes_events")"
[[ -n "$ready_ns" ]] || fail "没有记到 Hermes 的网关容器过健康检查：$(cat "$hermes_events")"
joined_ms="$(owner_api "/rooms/$hermes_room/members" | python3 -c 'import json, sys
print(next(e["origin_server_ts"] for e in json.load(sys.stdin)["chunk"]
           if e["state_key"] == sys.argv[1] and e["content"]["membership"] == "join"))' "@owner:$WONTON_DEV_DOMAIN")"
after_ms=$((joined_ms - ready_ns / 1000000))
((after_ms >= 0)) || fail "Hermes 就绪之前 $((-after_ms)) 毫秒，主人就进了私聊"
((after_ms < 30000)) || fail "Hermes 就绪 $((after_ms / 1000)) 秒后私聊才出现"
ok "Hermes 就绪后 $(printf '%d.%03d' $((after_ms / 1000)) $((after_ms % 1000))) 秒，私聊出现（主人加入了它）"
expect "Hermes 的凭据以它的 Bot 命名" "$(gateway_keys)" com.nousresearch.hermes/default
# Hermes 用它的凭据经核心问网关有哪些模型，和它的提示插件看到的一样。
hermes_models() {
  docker exec "$hermes" sh -c 'curl -fsS -H "Authorization: Bearer $WONTON_MODEL_API_KEY" "$WONTON_MODEL_BASE_URL/models"' |
    python3 -c 'import json, sys; print(" ".join(sorted(m["id"] for m in json.load(sys.stdin)["data"])))'
}
expect "还没有 provider：网关里一个模型也没有" "$(hermes_models)" ""
# 主人的一台新设备经隧道在私聊里发话，收 Hermes 的回答：$1 输出文件，$2 话。
talk_to_hermes() {
  local jwt
  jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
  docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
    --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$hermes_room" --bot "$hermes_user" \
    --reply any --wait 180 --text "$2" >"$1" 2>&1
}
hermes_reply() {
  python3 -c 'import sys
print(sys.stdin.read().partition("第 1 轮：收到加密的回复：")[2].strip())' <"$1"
}
model_usage() { docker exec "$magpie" magpie usage --last 15m --csv; }
expect_model_reply() {
  local usage
  usage="$(model_usage)" || fail "读取 magpie 用量失败"
  # 同一轮可调模型多次；逐轮取增量，不能拿第一轮的成功记录替第二轮作证。
  python3 -c 'import csv, io, sys
def hermes_rows(raw):
    return [r for r in csv.DictReader(io.StringIO(raw))
            if r["caller_key_name"] == "com.nousresearch.hermes/default"]
before = {r["request_id"] for r in hermes_rows(sys.argv[1]) if r["request_id"]}
rows = [r for r in hermes_rows(sys.stdin.read()) if r["request_id"] not in before]
valid = any(r["request_id"] and 200 <= int(r["status"]) < 300
            and r["error"] == "false" and int(r["output_tokens"]) > 0 for r in rows)
sys.exit(not valid)' "$2" <<<"$usage" || fail "模型没有正常回答：$(hermes_reply "$1")"
  ok "第 $3 轮模型调用成功且有输出 token，用量记在 Hermes 的凭据名下"
}
expect_hermes_memory() {
  local reply
  reply="$(hermes_reply "$1")"
  [[ "$reply" == *介绍* || "$reply" == *自己* ]] || fail "第二轮没有记住第一轮的请求：$reply"
  ok "第二轮记得第一轮让它介绍自己"
}
said="$(mktemp)"
talk_to_hermes "$said" "你好" || fail "Hermes 没有回答：$(cat "$said")"
grep '^第' "$said" | sed 's/^/   · /'
grep -q '模型设置' "$said" || fail "没有模型时 Hermes 应提示去“模型设置”添加：$(cat "$said")"
ok "没有模型时，Hermes 提示主人去“模型设置”添加模型服务商"
# 模型只经开发机上的 magpie：冒烟的 magpie 容器直连宿主 3425，不读取、不复制主人 magpie 的配置和密钥。
# 地址按 Docker 的种类定：OrbStack 和 Docker Desktop 的网络网关在虚拟机里，到不了宿主上的 magpie，
# 用 host.docker.internal；其余用插件网络的网关地址。凭据是本冒烟新建的一次性网关凭据：限定到这次的
# 模型，cleanup 按建 key 时记下的 id 删掉（失败退出也删），凭据不进日志、不写进仓库。
case "$(docker info --format '{{.OperatingSystem}}')" in
  *"Docker Desktop"* | *OrbStack*) model_host=host.docker.internal ;;
  *) model_host="$(docker network inspect "$magpie_network" --format '{{(index .IPAM.Config 0).Gateway}}')" ;;
esac
model_url="http://$model_host:3425/v1"
model_no_proxy="host.docker.internal,localhost,127.0.0.1,$model_host"
model_key="$(openssl rand -hex 24)"
host_model_key_name="wonton-smoke-core-$$"
magpie gateway-key add "$host_model_key_name" --key - <<<"$model_key" >/dev/null \
  || fail "宿主 magpie 建不了一次性网关凭据"
host_model_key_id="$(magpie gateway-key list 2>/dev/null | awk -v n="$host_model_key_name" '$2 == n { print $1; exit }')" \
  || fail "建了凭据但读不到凭据列表"
[[ -n "$host_model_key_id" ]] || fail "建了凭据但在列表里找不到它"
magpie gateway-key models "$host_model_key_id" "$model" >/dev/null || fail "一次性凭据限定模型失败"

# 只加 provider，不建路由组：默认路由组由 magpie 插件建好，成员跟着 provider 变。
docker exec -e no_proxy="$model_no_proxy" -e NO_PROXY="$model_no_proxy" "$magpie" \
  magpie provider add upstream url="$model_url" key="$model_key" \
  models="$model" >/dev/null || fail "magpie 连不上开发机上的 magpie（$model_url）"
[[ " $(hermes_models) " == *" group/wonton-default "* ]] ||
  fail "加了 provider，默认路由组应当可用：$(hermes_models)"
ok "加了 provider，默认路由组自动有了它的模型，不用手动建组"
round=0
for text in "你好，用一句话介绍你自己。" "我刚才让你做什么？"; do
  round=$((round + 1))
  usage_before="$(model_usage)" || fail "读取 magpie 用量失败"
  talk_to_hermes "$said" "$text" || fail "和 Hermes 的对话失败：$(cat "$said")"
  grep '^第' "$said" | sed "s/^第 1 轮/第 $round 轮/; s/^/   · /"
  expect_model_reply "$said" "$usage_before" "$round"
  ! grep -q '模型设置' "$said" || fail "有模型了，Hermes 不该再提示去“模型设置”：$(cat "$said")"
done
expect_hermes_memory "$said"
ok "经隧道和 Hermes 加密对话两轮"
rm -f "$said"

step "让 Agent 动手（MCP 总线）：Hermes 请主人确认后装上文件搜索，再用它在授权的文件夹里找文件"
hermes_mcp="$(hermes_env WONTON_MCP_URL)"
case "$hermes_mcp" in
  "http://mcp.wonton.internal:$WONTON_DEV_PORT/mcp/"?*) ;;
  *) fail "Hermes 拿到的 MCP 总线地址应带总线令牌：$hermes_mcp" ;;
esac
# 经隧道在和 Hermes 的私聊里说一句话，收它的回答，直到它 90 秒不再说话（它先说一句再去调要等确认的工具）：
# $1 输出文件，$2 话。
ask_hermes() {
  local jwt
  jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)"
  docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
    --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" --room "$hermes_room" --bot "$hermes_user" \
    --reply any --wait 300 --settle 90 --text "$2" >"$1" 2>&1
}
# 核心和主人的通知房间里最新的一张确认卡片（编辑过的以最新内容为准），输出“ID 状态”。
latest_card() {
  local room
  room="$(direct_rooms "@wonton-core:$WONTON_DEV_DOMAIN" | tail -1)"
  [[ -n "$room" ]] || return 0
  owner_api "/rooms/$room/messages?dir=b&limit=30" | python3 -c 'import json, sys
field = "app.wonton.confirmation"
edits, cards = {}, []
for event in json.load(sys.stdin).get("chunk", []):
    content = event.get("content", {})
    relation = content.get("m.relates_to", {})
    if relation.get("rel_type") == "m.replace":
        edits.setdefault(relation["event_id"], content.get("m.new_content", {}).get(field))
    elif field in content:
        cards.append((event["event_id"], content[field]))
if cards:
    event, card = cards[0]
    card = edits.get(event) or card
    print(card["id"], card["state"]["type"])'
}
answer() { "$@" -sS -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{"answer": "approve"}'; }
asked="$(mktemp)"
ask_hermes "$asked" "帮我装一下官方插件里的“文件搜索”。" &
asking=$!
card=""
for _ in $(seq 240); do
  card="$(latest_card)"
  [[ "$card" == *" pending" ]] && break
  sleep 1
done
[[ "$card" == *" pending" ]] || fail "Wonton 房间里没有出现待确认的卡片（$card）：$(cat "$asked")"
card_id="${card%% *}"
ok "Hermes 发起安装，核心在 Wonton 房间里发了确认卡片，还没有装"
expect "插件还没装" "$(plugin_state app.wonton.files)" absent
confirm_url="$core_url/_wonton/confirmations/$card_id"
expect "Hermes 的插件网络里来的请求答不了确认" "$(answer docker run --rm --network "$hermes_network" \
  "$curl_image" --noproxy '*' -H "Host: $WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" "http://$core:$WONTON_DEV_PORT/_wonton/confirmations/$card_id")" 401
expect "局域网浏览器（有核心会话）答不了确认" "$(answer b "$confirm_url")" 401
expect "主人的手机经隧道确认" "$(answer t "$confirm_url")" 200
for _ in $(seq 180); do [[ "$(plugin_state app.wonton.files)" == running ]] && break; sleep 1; done
expect "主人确认后装好了文件搜索" "$(plugin_state app.wonton.files)" running
for _ in $(seq 30); do [[ "$(latest_card)" == *" succeeded" ]] && break; sleep 1; done
expect "卡片更新成装好了" "$(latest_card)" "$card_id succeeded"
wait "$asking" || fail "Hermes 没有回答安装的结果：$(cat "$asked")"
grep '^第' "$asked" | sed 's/^/   · /'
[[ "$(grep -c '^第' "$asked")" -ge 1 ]] || fail "Hermes 没有说话：$(cat "$asked")"
ok "Hermes 在自己的私聊里回了话（内容见上面）"
docker exec "$core" sh -c "mkdir -p '$WONTON_DEV_DATA/shares/documents/税务' &&
  echo '2025 年度个人所得税汇算：应退税 1234 元' > '$WONTON_DEV_DATA/shares/documents/税务/2025-报税表.txt'"
ask_hermes "$asked" "在我的共享文件夹里找一下 2025 年的报税表，告诉我它的完整路径。" ||
  fail "Hermes 没有回答找文件的结果：$(cat "$asked")"
grep '^第' "$asked" | sed 's/^/   · /'
grep -q '2025-报税表' "$asked" || fail "Hermes 的回答里没有找到的文件：$(cat "$asked")"
ok "Hermes 经 MCP 总线在授权的文件夹里找到了文件"
ask_hermes "$asked" "把这个报税表文件发给我。" || fail "Hermes 没有回答发文件的结果：$(cat "$asked")"
grep '^第' "$asked" | sed 's/^/   · /'
audit="$(docker exec "$core" cat "$WONTON_DEV_DATA/core/audit.jsonl")"
# 审计里有没有一行：工具名匹配 $1（正则），决定是 $2。
audited() {
  python3 -c 'import json, re, sys
lines = [json.loads(line) for line in sys.stdin if line.strip()]
sys.exit(not any(re.fullmatch(sys.argv[1], l["tool"]) and l["decision"] == sys.argv[2] for l in lines))' "$1" "$2" <<<"$audit"
}
audited wonton__install_plugin approved || fail "审计里没有主人确认的安装：$audit"
audited 'app-wonton-files__[a-z_]+' allowed || fail "审计里没有找文件：$audit"
audited wonton__share_file allowed || fail "审计里没有分享文件：$audit"
ok "审计日志记下了确认过的安装、找文件和分享文件"
grep -q 'wonton://file/' "$asked" || fail "Hermes 的回答里没有文件链接（App 显示不成文件卡片）：$(cat "$asked")"
ok "Hermes 发出了文件链接，App 显示成文件卡片"

step "Bot 存文件（ADR-0027）：主人按路径批准一次，Hermes 就能往里存，Navidrome 看得到"
navidrome_src="$(mktemp -d)"
git clone -q --depth 1 --branch v0.1.1 -c advice.detachedHead=false https://github.com/geraldpeng6/wonton-plugin-navidrome "$navidrome_src"
installed="$("$dev" install "$navidrome_src" --yes 2>&1)" || fail "Navidrome 安装失败：$installed"
rm -rf "$navidrome_src"
expect "Navidrome 装好了" "$(plugin_state org.navidrome.navidrome)" running
shares="$WONTON_DEV_DATA/shares"
outbox="$WONTON_DEV_DATA/apps/$(tr . - <<<com.nousresearch.hermes)/outbox"
# WONTON_SMOKE_SONG_URL：换成真的下载一首歌，经 Hermes 出网。默认由它自己生成，不依赖外面的服务。
song_task() {
  if [[ -n "${WONTON_SMOKE_SONG_URL:-}" ]]; then
    printf '下载 %s 保存成 %s' "$WONTON_SMOKE_SONG_URL" "$1"
  else
    printf '用 Python 标准库的 wave 模块生成一个 1 秒的静音 WAV 文件 %s' "$1"
  fi
}
song_ext=wav
if [[ -n "${WONTON_SMOKE_SONG_URL:-}" ]]; then song_ext="${WONTON_SMOKE_SONG_URL##*.}"; fi
# 等 Wonton 房间里出现待确认的卡片，输出卡片 ID。
pending_card() {
  local card=""
  for _ in $(seq 240); do
    card="$(latest_card)"
    [[ "$card" == *" pending" ]] && { echo "${card%% *}"; return 0; }
    sleep 1
  done
  return 1
}
answer_with() { t -sS -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d "{\"answer\": \"$1\"}" "$core_url/_wonton/confirmations/$2"; }
save_grants() {
  t "$core_url/_wonton/api/plugins/com.nousresearch.hermes" | python3 -c 'import json, sys; print(" ".join(json.load(sys.stdin)["save_grants"]))'
}
ask_hermes "$asked" "$(song_task "smoke-a.$song_ext")，放在 outbox 目录里，然后存进共享文件夹 music 的子目录 冒烟。" &
asking=$!
card_id="$(pending_card)" || fail "存文件没有发确认卡片：$(cat "$asked")"
expect "回答之前文件没有放进去" "$(docker exec "$core" ls "$shares/music")" ""
expect "主人批准这个路径" "$(answer_with approve "$card_id")" 200
wait "$asking" || fail "Hermes 没有回答存文件的结果：$(cat "$asked")"
grep '^第' "$asked" | sed 's/^/   · /'
expect "文件放进了 music/冒烟" "$(docker exec "$core" ls "$shares/music/冒烟")" "smoke-a.$song_ext"
expect "outbox 里的原文件删掉了" "$(docker exec "$core" ls "$outbox")" ""
expect "Hermes 的详情页列出这个路径" "$(save_grants)" "music/冒烟"
ask_hermes "$asked" "$(song_task "smoke-b.$song_ext")，放在 outbox 目录里，然后存进共享文件夹 music 的子目录 冒烟/第二张。" ||
  fail "Hermes 没有回答第二次存文件的结果：$(cat "$asked")"
expect "子文件夹不再问，直接放进去" "$(docker exec "$core" ls "$shares/music/冒烟/第二张")" "smoke-b.$song_ext"
[[ "$(latest_card)" == "$card_id succeeded" ]] || fail "第二次存文件又发了卡片：$(latest_card)"
expect "撤销授权" "$(t -o /dev/null -w '%{http_code}' -X DELETE "$core_url/_wonton/api/plugins/com.nousresearch.hermes/save-grants?folder=music%2F%E5%86%92%E7%83%9F")" 200
expect "详情页不再列它" "$(save_grants)" ""
ask_hermes "$asked" "$(song_task "smoke-c.$song_ext")，放在 outbox 目录里，然后存进共享文件夹 music 的子目录 冒烟。" &
asking=$!
card_id="$(pending_card)" || fail "撤销之后存文件没有重新发卡片：$(cat "$asked")"
expect "这次批准整个共享文件夹" "$(answer_with approve_wider "$card_id")" 200
wait "$asking" || fail "Hermes 没有回答第三次存文件的结果：$(cat "$asked")"
expect "授权变成 music" "$(save_grants)" "music"
# 代登录的账号在第一次打开网页时才建，没打开过的 Subsonic 接口回“用户名或密码不对”。
t -L -o /dev/null "http://org-navidrome-navidrome.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT/"
for _ in $(seq 150); do
  found="$(t "http://org-navidrome-navidrome.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT/rest/search3.view?v=1.16.1&c=smoke&f=json&query=&songCount=50" |
    python3 -c 'import json, sys; print(len(json.load(sys.stdin)["subsonic-response"]["searchResult3"].get("song", [])))' 2>/dev/null || true)"
  [[ "${found:-0}" -ge 3 ]] && break
  sleep 2
done
expect "Navidrome 扫到了 3 首" "${found:-0}" 3
audit="$(docker exec "$core" cat "$WONTON_DEV_DATA/core/audit.jsonl")"
audited wonton__save_file approved || fail "审计里没有批准过的存文件：$audit"
audited wonton__save_file allowed || fail "审计里没有命中授权的存文件：$audit"
audited wonton__revoke_save_grant approved || fail "审计里没有撤销：$audit"
ok "审计日志记下了批准、命中授权和撤销"
expect "卸载 Navidrome" "$("$dev" uninstall org.navidrome.navidrome --purge | json data_removed)" true
rm -f "$asked"
expect "卸载文件搜索" "$("$dev" uninstall app.wonton.files --purge | json data_removed)" true
hermes_token="$(hermes_env MATRIX_ACCESS_TOKEN)"
report="$("$dev" uninstall com.nousresearch.hermes --purge)"
expect "卸载 Hermes：吊销 Bot 的设备、停用账号" \
  "$(json bots.0.devices_removed <<<"$report") $(json bots.0.deactivated <<<"$report")" "1 true"
expect "Hermes 的 Matrix 令牌失效" \
  "$(t -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $hermes_token" "$matrix_url/_matrix/client/v3/account/whoami")" 401
expect "网关里 Hermes 的凭据吊销了" "$(gateway_keys)" ""
expect "卸载 Hermes：存文件的授权清掉了" "$(docker exec "$core" find "$WONTON_DEV_DATA" -path '*save-grants*' -name 'com.nousresearch.hermes.json')" ""

step "“模型设置”网页（sso: proxy）：主人不用输网页密钥就能打开，密钥不出网关"
magpie_url="http://ai-usemagpie-magpie.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
# 核心生成、注入 magpie 容器的网页密钥，只用来检查它没有出现在任何响应和 Cookie 罐里。
web_key="$(docker inspect "$magpie" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MAGPIE_WEB_KEY=//p')"
[[ "$web_key" =~ ^[0-9a-f]{64}$ ]] || fail "magpie 没有拿到核心生成的网页密钥"
upstream_off() { [[ "$(docker exec "$magpie" magpie providers)" == *'switched off'* ]]; }
# 没有任何 Cookie 的浏览器。
anon() { docker exec "$browser" curl --noproxy '*' -sS "$@"; }
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
installed="$("$dev" install "$repo/plugins/examples/whoami" --yes 2>/dev/null)"
expect "whoami 安装完成" "$(json status.state <<<"$installed")" running
echoed="$(b -L -H 'Accept: text/html' -H 'X-Forwarded-For: 6.6.6.6' "$whoami_url/")"
grep -qi "^X-Forwarded-Host: app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" <<<"$echoed" || fail "缺少 X-Forwarded-Host：$echoed"
! grep -q 'wonton_app=\|wonton_session=' <<<"$echoed" || fail "网关 Cookie 泄露给了插件：$echoed"
! grep -q '6.6.6.6' <<<"$echoed" || fail "伪造的 X-Forwarded-For 被转发：$echoed"
ok "插件收到转发头，收不到网关 Cookie 和伪造的转发头"
# 从 whoami 的插件网络里冒充 Bot 和模型网关的使用方：网关按来源地址认出插件，它两项权限都没申请。
from_whoami() {
  docker run --rm --network "$whoami_network" "$curl_image" --noproxy '*' -sS -o /dev/null -D - \
    -H "Host: $1:$WONTON_DEV_PORT" "http://$core:$WONTON_DEV_PORT$2"
}
for internal in "matrix.wonton.internal /_matrix/client/versions" "models.wonton.internal /v1/models" "mcp.wonton.internal /mcp"; do
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
expect "插件状态恢复为运行中" "$("$dev" list | awk 'NR == 1 { print $2 }')" running
start_browser
b -o /dev/null -X POST --data-urlencode "token=$("$dev" token)" "$core_url/_wonton/login"
# 新浏览器没有任何 Cookie：打开首页会重新走一遍会话交接和 OIDC。
expect "重新登录后打开插件仍自动登录" \
  "$(b -L -o /dev/null -w '%{http_code}' -H 'Accept: text/html' "$grafana_url/")" 200
expect "核心重启后插件仍认出主人" "$(b "$grafana_url/api/user" | json login)" owner
expect "核心重启后“模型设置”仍免密码打开（代登录的密钥从密钥库读回）" \
  "$(b -L -o /dev/null -w '%{http_code} %{url_effective}' -H 'Accept: text/html' "$magpie_url/")" "200 $magpie_url/"
# 第一次启动时 Hermes 还没装过，这条日志只会在重启后出现。不用 grep -q：它找到就退出，docker logs 接着写就收到
# SIGPIPE，pipefail 下整条管道算失败。
for _ in $(seq 60); do docker logs "$core" 2>&1 | grep '默认插件装过一次' >/dev/null && break; sleep 1; done
docker logs "$core" 2>&1 | grep '默认插件装过一次' >/dev/null || fail "核心重启后没有核对默认插件"
expect "主人卸掉的 Hermes，核心重启后不装回来" "$(plugin_state com.nousresearch.hermes) $(labeled com.nousresearch.hermes)" "absent "

step "卸载干净"
started=$(date +%s)
report="$("$dev" uninstall app.grafana --purge)"
uninstall_seconds=$(($(date +%s) - started))
expect "删除了容器（${uninstall_seconds}s）" "$(json containers <<<"$report")" "[\"$grafana\"]"
expect "删除了插件网络" "$(json networks <<<"$report")" "[\"$grafana_network\"]"
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
