#!/usr/bin/env bash
# M0 端到端冒烟：一条命令装好示例插件 → 健康检查通过 → 未登录的请求被拒 →
# 像浏览器一样经核心网关打开插件并自动登录 → 核心不预装模型网关，申请 models 的插件照样装得上，请求报网关不可用，
# 装上后转发、停掉报 503 →
# 已配对设备经隧道打开插件，直接是主人 → 经隧道登录 Matrix，和回声 Bot 收发 →
# 核心自己装好的默认 Agent Hermes 就绪后出现在主人的私聊里：还没选模型时提示去它网页的 Models 页，给它的
# Bot 配一个指向宿主 magpie 的服务后和主人对话两轮 → 停链打断 Hermes 正在跑的一轮；批准的私聊经出箱发到家人手里；核心按说话人请求头认人 →
# 网关代登录打开“模型设置”网页 → 核心重启后恢复、不装回卸掉的 Hermes → 卸载干净。Hermes 一步要真实模型：开发机上要有在运行的 magpie，模型名经 WONTON_SMOKE_MODEL 传入
# （magpie 里一个能对话的模型），宿主 magpie 不在 3425 时用 WONTON_SMOKE_MODEL_RELAY=<宿主上的地址:端口>。
# Hermes 容器解析不到 host.docker.internal 时（Linux 的 Docker 不给插件容器加这条）经它自己网络网关上的 socat
# 转发连，见 Hermes 段。凭据是冒烟按 Bot 名字新建的一次性网关凭据：限定到这次的模型，退出时按建 key 时记下的 id
# 删掉（失败退出也删），凭据不进日志、不写进仓库。
#
#   deploy/dev/core/smoke.sh    本机 Docker；要设 WONTON_SMOKE_MODEL，见上
#   DOCKER_HOST=ssh://$DEV_HOST deploy/dev/core/smoke.sh  远端 Linux 主机（远端只需要 Docker，DEV_HOST 是它的 SSH 目标）
#
# 使用独立的 compose 项目、端口和临时数据目录，结束时清理全部容器、网络、卷和数据；KEEP=1 保留现场。
# 分段跑（步骤名见 smoke-lib.sh）：SMOKE_UNTIL=<步骤> 跑到这一步为止（配 KEEP=1 留下栈）；SMOKE_ONLY=<步骤>[,<步骤>…]
# 只跑列出的步骤，对着已经在跑的栈（KEEP=1 留下的，或用 WONTON_DEV_PROJECT、WONTON_DEV_INSTANCE、WONTON_DEV_DOMAIN、
# WONTON_DEV_PORT 指定的别的开发栈），不起栈也不拆栈；用到前面步骤里的变量、函数的步骤单跑会直接报错。
# 本机需要 bash、docker、python3；HTTP 请求都在与核心共享网络的 curl 容器里发出。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
dev="$here/dev.sh"

# shellcheck source=deploy/dev/core/smoke-lib.sh
source "$here/smoke-lib.sh"
# shellcheck source=deploy/dev/core/dev-guard.sh
source "$here/dev-guard.sh"

# 带实例名：下面的清理只动这个实例的插件，同一台 Docker 上别的核心不受影响。冒烟一次只跑一个。
# 只有 SMOKE_ONLY 对着已有的栈跑时才认外面给的项目、实例、域名和数据目录。
if [[ -z "${SMOKE_ONLY:-}" ]]; then
  WONTON_DEV_PROJECT=wonton-smoke WONTON_DEV_INSTANCE=smoke WONTON_DEV_DOMAIN=wonton.localhost
  WONTON_DEV_DATA="/tmp/wonton-smoke-$(date +%s)-$$"
fi
export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-smoke}" WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-smoke}"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18481}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18483}"
export WONTON_DEV_DOMAIN="${WONTON_DEV_DOMAIN:-wonton.localhost}" WONTON_DEV_DATA="${WONTON_DEV_DATA:-/tmp/wonton-smoke}"
if [[ -z "${SMOKE_ONLY:-}" ]]; then dev_guard_check "$WONTON_DEV_PROJECT"; fi
# 失败时最要紧的是卡片认没认到链：这三行是 debug 级，冒烟自己给核心开 debug（info 看不到），
# 别的级别照旧由外面给。
export RUST_LOG="${RUST_LOG:-wonton_core=debug}"
# 核心镜像和隧道客户端镜像用同一个提交号构建，共用 Dockerfile 里的编译阶段。
WONTON_BUILD_COMMIT="$("$dev" commit)"
export WONTON_BUILD_COMMIT
core="$WONTON_DEV_PROJECT"
browser="$WONTON_DEV_PROJECT-browser"
phone="$WONTON_DEV_PROJECT-phone"
# 家人（妈妈）的手机，停链、出箱、说话人几步用。
mom_phone="$WONTON_DEV_PROJECT-mom"
phone_image="wonton-tunnel-client:dev"
echo_image="wonton-echo-bot:dev"
# 探针设备：加密库和登录状态存在数据目录里，跨步骤、跨续跑都是同一台设备，对方的设备表里一直是它。
# 每次冒烟换数据目录，也就换新设备；同一个 ID 换新密钥时对方的 SDK 会报签名密钥变了，所以固定 ID 只和数据目录一起用。
probe_device="wonton-smoke-probe"
mom_probe_device="wonton-smoke-mom-probe"
probe_store="$WONTON_DEV_DATA/probe-store"
mom_probe_store="$WONTON_DEV_DATA/mom-probe-store"
mkdir -p "$probe_store" "$mom_probe_store"
curl_image="curlimages/curl:8.23.0"
busybox_image="public.ecr.aws/docker/library/busybox:1.37"
socat_image="alpine/socat:1.8.0.3"
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
model_relay="$WONTON_DEV_PROJECT-model-relay"
model_client="wonton-$WONTON_DEV_INSTANCE--app-model-client--client"
# Hermes 声明了 bots（x-wonton.bots），gateway 是 Bot 服务：容器按 Bot 一份。
hermes="$hermes_network--gateway--default"
# 排在 matrix 步骤外面：SMOKE_ONLY 跳过那一步时后面也要用。
hermes_env() { docker inspect "$hermes" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
plugins=(app.grafana app.whoami agent.echo app.model-client ai.usemagpie.magpie com.nousresearch.hermes app.wonton.files org.navidrome.navidrome)

# shellcheck source=deploy/dev/core/smoke-cleanup.sh
source "$here/smoke-cleanup.sh"

# 构建镜像经 dev.sh：远端主机上经它自己的代理、限制并发（WONTON_DEV_BUILD_PROXY、WONTON_DEV_BUILD_JOBS）。
build_failed() {
  docker buildx version >/dev/null 2>&1 || fail "构建镜像需要 Docker buildx，请安装 Docker buildx 插件后重试"
  fail "$1（详情见上面的输出）"
}
build() { "$dev" build -q "$@" >/dev/null || build_failed "镜像构建失败"; }
# 真实模型：冒烟的 magpie 容器直连宿主上的 magpie（地址见模型段的按 Docker 种类区分），凭据见文件头。
if reaches hermes; then
  model="${WONTON_SMOKE_MODEL:?未设 WONTON_SMOKE_MODEL：Hermes 一步要真实模型，填开发机 magpie 里能对话的模型名}"
fi

# 各步骤体里用到的接口封装：排在步骤外面，SMOKE_ONLY 跳过前一步时后面也要用。
owner_api() { t -H "Authorization: Bearer $access" "$matrix_url/_matrix/client/v3$1"; }
direct_rooms() {
  owner_api "/user/@owner:$WONTON_DEV_DOMAIN/account_data/m.direct" |
    python3 -c 'import json, sys; print("\n".join(json.load(sys.stdin).get(sys.argv[1], [])))' "$1"
}
# 从 Hermes 自己的容器真连一次：地址、凭据、防火墙都得通。
hermes_models() {
  docker exec "$hermes" sh -c "curl -fsS --max-time 30 -H 'Authorization: Bearer $hermes_key' '$model_url/models'" |
    python3 -c 'import json, sys; print(" ".join(sorted(m["id"] for m in json.load(sys.stdin)["data"])))'
}
# 主人的一台新设备经隧道在私聊里发话，收 Hermes 的回答：$1 输出文件，其余是探针参数。
# 新设备偶发 404 M_NOT_FOUND（登录后服务器说没见过这台设备）：用同一台重发一次，不重发已发过的旧消息。
probe_in_hermes_room() {
  local out="$1" jwt rc=0 attempt
  shift
  for attempt in 1 2 3; do
    rc=0
    (( attempt == 1 )) || echo '   · 探针设备被服务器丢了，用同一台重试' >&2
    jwt="$(t -X POST "$core_url/_wonton/matrix/login-token" | json token)" || return 1
    docker run --rm --network "container:$core" -v "$probe_store:/probe" "$echo_image" probe \
      --homeserver "$matrix_url" --proxy "http://$credential@127.0.0.1:18008" --login-token "$jwt" \
      --room "$hermes_room" --bot "$hermes_user" --device-id "$probe_device" --store /probe \
      --reply any "$@" >"$out" 2>&1 || rc=$?
    # 有一句正常回复就算过：探针偶发 404 当空回复、退出码还是 0。
    if (( rc == 0 )) && grep -q '收到明文的回复：' "$out"; then return 0; fi
    # 不是 404 的失败（含探针自己的报错）不再重来。
    if ! grep -q M_NOT_FOUND "$out"; then return "$rc"; fi
  done
  return 1
}
talk_to_hermes() { probe_in_hermes_room "$1" --wait 180 --text "$2"; }
# Hermes 在网关里缓存配置，它自己保存配置（装插件换工具时实测）会把 model.context_length 从磁盘盖回去，
# 之后一轮就按 32768 撞它 64k 的下限。让它动手前对一遍，不是 64k 就重新写；会话令牌每轮重取（网关可能重启）。
ensure_hermes_context() {
  hermes_token="$(docker exec "$hermes" sh -c 'curl -fsS http://127.0.0.1:9119/' |
    sed -n 's/.*__HERMES_SESSION_TOKEN__="\([^"]*\)".*/\1/p')"
  [[ -n "$hermes_token" ]] || fail "Hermes 的页面没有给出会话令牌"
  [[ "$(docker exec "$hermes" sh -c 'curl -sS --max-time 30 http://127.0.0.1:9119/api/model/info' | json effective_context_length)" == 65536 ]] && return 0
  docker exec "$hermes" sh -c "curl -sS --max-time 30 -X PUT http://127.0.0.1:9119/api/config \
    -H 'X-Hermes-Session-Token: $hermes_token' -H 'content-type: application/json' \
    -H 'Origin: http://127.0.0.1:9119' -d '{\"config\":{\"model\":{\"context_length\":65536}}}'" >/dev/null ||
    fail "Hermes 的上下文设不进去"
}
ask_hermes() { ensure_hermes_context; probe_in_hermes_room "$1" --wait 1200 --settle 180 --text "$2"; }
# 确认卡片（ADR-0016）：卡片不再进 Matrix 房间，从核心的“等你确认”列表取；等人回答的在前，已经结束的在
# 后面，拼成一行“ID 状态 ”（每项后面带空格，只有一张卡时 `*" pending "*` 也匹配得上）。
latest_card() {
  t "$core_url/_wonton/api/confirmations" | python3 -c 'import json, sys
d = json.load(sys.stdin)
cards = d.get("confirmations", []) + d.get("finished", [])
print(" ".join("{} {}".format(c["id"], c["state"]["type"]) for c in cards) + " ")'
}
answer() { "$@" -sS -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{"answer": "approve"}'; }
# 等核心的待确认列表里出现卡片，输出卡片 ID。
pending_card() {
  local card=""
  for _ in $(seq 900); do
    card="$(latest_card)"
    [[ "$card" == *" pending "* ]] && { echo "${card%% *}"; return 0; }
    sleep 1
  done
  return 1
}
answer_with() { t -sS -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d "{\"answer\": \"$1\"}" "$core_url/_wonton/confirmations/$2"; }
audited() {
  python3 -c 'import json, re, sys
lines = [json.loads(line) for line in sys.stdin if line.strip()]
sys.exit(not any(l.get("tool") and re.fullmatch(sys.argv[1], l["tool"]) and l.get("decision") == sys.argv[2] for l in lines))' "$1" "$2" <<<"$audit"
}
song_task() {
  if [[ -n "${WONTON_SMOKE_SONG_URL:-}" ]]; then
    printf '下载 %s 保存成 %s' "$WONTON_SMOKE_SONG_URL" "$1"
  else
    printf '用 Python 标准库的 wave 模块生成一个 1 秒的静音 WAV 文件 %s' "$1"
  fi
}
save_grants() {
  t "$core_url/_wonton/api/plugins/com.nousresearch.hermes" | python3 -c 'import json, sys; print(" ".join(json.load(sys.stdin)["save_grants"]))'
}
upstream_off() { [[ "$(docker exec "$magpie" magpie providers)" == *'switched off'* ]]; }
anon() { curl_in "$browser" --noproxy '*' -sS "$@"; }
page_post() { b -D - -H "Origin: $1" -H 'content-type: application/json' -d '{"id":"upstream"}' "$magpie_url/api/provider/$2"; }
from_whoami() {
  docker run --rm --network "$whoami_network" "$curl_image" --noproxy '*' \
    --max-time "$SMOKE_WEB_MAX_TIME" --connect-timeout "$SMOKE_CONNECT_TIMEOUT" -sS -o /dev/null -D - \
    -H "Host: $1:$WONTON_DEV_PORT" "http://$core:$WONTON_DEV_PORT$2"
}

# 从冒烟一开始就记下 Hermes 网关容器的健康检查事件，量它变健康后多久私聊出现。分段跑（SMOKE_ONLY）
# 跳过启动核心，事件也照记；事件早在上一段跑的时候发生过，由 hermes 一步退回容器的健康日志。
hermes_events="$(mktemp)"
docker events --filter "container=$hermes" --filter event=health_status --format '{{.TimeNano}} {{.Action}}' \
  >"$hermes_events" 2>/dev/null &
events_pid=$!

if step preflight "预检"; then
docker version --format '   Docker {{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}'
for id in "${plugins[@]}"; do
  # 默认 Agent 由核心启动时自己装：对着已有的栈跑（SMOKE_ONLY）时它已经在，是正常的。
  [[ $id != com.nousresearch.hermes ]] || continue
  [[ -z "$(labeled "$id")" ]] || fail "Docker 上已经有插件 $id 的资源，先卸载它再跑冒烟"
done
ok "没有残留的示例插件资源"
fi
# 对着已有的栈跑时不拆栈，只删自己起的探针容器。
if [[ -n "${SMOKE_ONLY:-}" ]]; then
  trap '[[ -z "${events_pid:-}" ]] || kill "$events_pid" 2>/dev/null || true; [[ -z "${host_model_key_id:-}" ]] || ( host_magpie gateway-key remove "$host_model_key_id" ) >/dev/null 2>&1 || true; docker rm -f "$browser" "$phone" "$mom_phone" "$model_relay" >/dev/null 2>&1 || true' EXIT
else trap cleanup EXIT; fi

if step core "启动核心和身份提供方（项目 $core，端口 $WONTON_DEV_PORT）"; then
# 核心一启动就在后台装 Hermes（健康事件由上面的监听记下）。
# 每次全新跑都换一个新核心，但身份提供方的卷不跟数据目录走：上一个栈留下的 @mom 会让家人那步的邀请撞重名，
# 换掉它才和「新数据目录 = 新核心」一致。
if [[ -z "${SMOKE_ONLY:-}" ]]; then
  if docker volume ls -q | grep -qx "${core}_tuwunel"; then docker volume rm "${core}_tuwunel" >/dev/null; fi
  "$dev" up >/dev/null || build_failed "核心启动失败"
fi
start_browser
expect "核心容器健康" "$(docker inspect -f '{{.State.Health.Status}}' "$core")" healthy
expect "网关应答（$core_url）" "$(b -o /dev/null -w '%{http_code}' "$core_url/_wonton/health")" 200

fi
# 对着保留下来的栈分段跑时，上一次结束已经删掉了探测容器：每段都补一个浏览器和手机。
[[ -n "${SMOKE_ONLY:-}" ]] && start_browser
# 分段跑用栈上现成的客户端镜像，不再排一次 cargo 名额。
if [[ -z "${SMOKE_ONLY:-}" ]] || [[ -z "$(docker images -q "$phone_image")" ]]; then
  build --target tunnel-client --build-arg WONTON_BUILD_COMMIT -t "$phone_image" -f "$here/Dockerfile" "$repo"
fi
# 假 docker 的用例里（dev-guard_test）隧道起不来，拿不到 id 就空着：后面只有 docker 命令用得到它。
server_id="$("$dev" tunnel 2>/dev/null | json endpoint_id 2>/dev/null || true)"
# 手机：隧道客户端示例跑在核心的网络命名空间里，经回环地址直连隧道入口，本地代理在 127.0.0.1:18008，
# 和 App 一样只接受带本次会话凭据的请求。私钥放在数据目录：重启手机进程（模拟 App 回前台触发重连，
# 隧道客户端被拒后按 ADR-0032 等 retry_now，不自动重拨）后还是同一个设备。客户端对已存在的空文件不生成
# 私钥，所以先用一次性容器把它生成出来。
credential=""
phone_key="$WONTON_DEV_DATA/phonekeys/phone.key"
[[ -s "$phone_key" ]] || docker run --rm -v "$WONTON_DEV_DATA/phonekeys:/k" --entrypoint sh "$phone_image" \
  -c "tunnel-client --key /k/phone.key id" >/dev/null
start_phone() {
  docker rm -f "$phone" >/dev/null 2>&1 || true
  run_container "$phone" --network "container:$core" -v "$phone_key:/tmp/phone.key" --entrypoint sh "$phone_image" -c \
    "tunnel-client --key /tmp/phone.key id && exec tunnel-client --key /tmp/phone.key proxy --auth \
     --domain $WONTON_DEV_DOMAIN --server $server_id --addr 127.0.0.1:$WONTON_DEV_TUNNEL_PORT \
     --listen 127.0.0.1:18008" >/dev/null
  for _ in $(seq 30); do
    # 不用 grep -q：它匹配就退出，docker logs 会吃 SIGPIPE，pipefail 下会杀掉整个脚本。
    # 代理的连接是第一次请求时才建的；这里只等它的监听出来。
    docker logs "$phone" 2>/dev/null | grep '^本地代理' >/dev/null && break
    sleep 1
  done
  docker logs "$phone" 2>/dev/null | grep -q '^本地代理' || fail "手机没起来：$(docker logs "$phone" 2>&1 | tail -3)"
  credential="$(docker logs "$phone" 2>/dev/null | sed -n 's/^会话凭据：//p')"
  # 监听先出来、隧道才连得上，核心端口也是随后才通：这两样没好时经代理的请求没有回应，
  # 或者代理报 transport-unavailable / upstream-unavailable（x-wonton-tunnel-error 的码、
  # x-wonton-error 的类别）。核心的 /_wonton/health 是静态 200，隧道通了就是就绪。
  # 后面的「未登记的设备被隧道拒绝」要等这些都过去才问得到 403；分段跑时手机可能已经
  # 配对过，所以不放行只看 403。
  local reply
  proxy_ready() {
    [[ -n "$reply" ]] &&
      ! grep -qE 'x-wonton-(tunnel-)?error: (transport|tunnel|upstream)-unavailable' <<<"$reply"
  }
  for _ in $(seq 60); do
    reply="$(t -o /dev/null -D - "$core_url/_wonton/health" 2>/dev/null || true)"
    proxy_ready && break
    sleep 1
  done
  proxy_ready || fail "手机的隧道没连上或核心没就绪：$(head -1 <<<"$reply")；$(docker logs "$phone" 2>&1 | tail -3)"
  return 0
}
# 拿不到隧道 id（假 docker 的用例里）就起不了手机，整段跳过；真跑的每一步都靠它。
if [[ -n "$server_id" ]]; then
start_phone
device_id="$(docker logs "$phone" 2>/dev/null | sed -n '1p')"
[[ "$device_id" =~ ^[0-9a-f]{64}$ && -n "$credential" ]] || fail "手机没有启动：$(docker logs "$phone" 2>&1)"
# SMOKE_ONLY 直接跑某一步时没走过 start_phone，凭据从探测手机上现读。
credential="${credential:-$(docker logs "$phone" 2>/dev/null | sed -n 's/^会话凭据：//p' | tail -1)}"
fi
# Hermes 的账号和私聊房间在 hermes、matrix 两步里取，分段跑时要现查（access 见 matrix 步上面）。
if step install "一条命令安装 Grafana 示例插件"; then
started=$(date +%s)
installed="$("$dev" install "$repo/plugins/examples/grafana" --yes 2>/dev/null)"
install_seconds=$(($(date +%s) - started))
expect "安装完成且健康检查通过（${install_seconds}s）" "$(json status.state <<<"$installed")" running
expect "网页地址" "$(json web_url <<<"$installed")" "$grafana_url/"

fi
if step isolation "隔离"; then
expect "插件容器不向宿主暴露端口" \
  "$(docker inspect -f '{{len .HostConfig.PortBindings}}' "$grafana")" 0
expect "插件容器只在自己的网络里" \
  "$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' "$grafana")" "$grafana_network "
aliases="$(docker inspect -f "{{json (index .NetworkSettings.Networks \"$grafana_network\").Aliases}}" "$core")"
[[ "$aliases" == *"\"auth.wonton.internal\""* ]] || fail "核心没有以 auth.wonton.internal 加入插件网络：$aliases"
ok "核心加入插件网络，别名 auth.wonton.internal（插件经它访问身份提供方）"
[[ "$aliases" == *"\"app-grafana.$WONTON_DEV_DOMAIN\""* ]] || fail "核心没有以插件自己的地址加入它的网络：$aliases"
ok "核心也以插件自己的地址加入网络，插件能访问自己"
out="$(docker exec "$grafana" wget -S -q -T 30 -O /dev/null "$grafana_url/" 2>&1 || true)"
[[ "$out" =~ HTTP/[0-9.]+[[:space:]]+[23][0-9][0-9] ]] || fail "插件经自己的地址访问自己失败：$out"
[[ "$out" == *"/_wonton/sso"* ]] && fail "插件经自己的地址访问自己仍要会话：$out"
ok "插件经自己的地址访问自己，不要会话（第一跳是它自己的页面，不是核心的会话交接）"
if docker exec "$grafana" wget -q -T 3 -O /dev/null http://hydra.wonton.internal:4445/health/ready 2>/dev/null; then
  fail "插件能访问身份提供方的管理端口"
fi
ok "插件访问不到身份提供方的管理端口"

fi
if step unauth "未登录的请求被拒"; then
reply="$(b -o /dev/null -w '%{http_code} %{redirect_url}' -H 'Accept: text/html' "$grafana_url/")"
expect "页面请求跳到核心换取会话" "${reply%% *}" 303
[[ "${reply#* }" == "$core_url/_wonton/sso?to="* ]] || fail "跳转地址不对：${reply#* }"
expect "API 请求返回 401" "$(b -o /dev/null -w '%{http_code}' -H 'Accept: application/json' "$grafana_url/api/user")" 401
expect "伪造的插件会话无效" \
  "$(b -o /dev/null -w '%{http_code}' -H 'Cookie: wonton_app=forged' "$grafana_url/api/user")" 401

fi
if step sso "像浏览器一样打开插件：登录核心一次，插件自动登录"; then
expect "主人令牌登录核心" \
  "$(b -o /dev/null -w '%{http_code}' -X POST --data-urlencode "token=$("$dev" token)" "$core_url/_wonton/login")" 303
reply="$(b -L -H 'Accept: text/html' -o /dev/null -w '%{http_code} %{url_effective} %{num_redirects}' "$grafana_url/")"
read -r code final redirects <<<"$reply"
expect "落在 Grafana 首页（经过 $redirects 次跳转：会话交接 + OIDC）" "$code $final" "200 $grafana_url/"
user="$(b "$grafana_url/api/user")"
expect "Grafana 里的用户是主人" "$(json login <<<"$user")" owner
expect "邮箱来自核心签发的 ID 令牌" "$(json email <<<"$user")" "owner@wonton.internal"
expect "按 groups 声明映射为管理员" "$(b "$grafana_url/api/user/orgs" | json 0.role)" Admin
printf '%s' "$("$dev" token)" |
  python3 "$here/smoke-oidc.py" "$browser" "$grafana" "$core_url" "$grafana_url"

fi
if step models "不预装模型网关：申请 models 的插件照样装得上；没有网关时请求报网关不可用"; then
expect "核心没有预装模型网关" "$(plugin_state ai.usemagpie.magpie)" absent
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
install_err="$(mktemp)"
installed="$("$dev" install "$client_file" --yes 2>"$install_err")" || fail "使用方安装失败：$(cat "$install_err")"
rm -f "$client_file" "$install_err"
expect "使用方安装完成" "$(json status.state <<<"$installed")" running
client_env() { docker inspect "$model_client" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
expect "没有网关时模型变量是空的" "$(client_env BASE)|$(client_env KEY)|$(client_env MODEL)" "||"
# 凭据按使用方的变量带上：magpie 对非回环、没凭据的请求回 401（契约测试 consumer.rs 同样带 Bearer）。
request_models() {
  local -a auth=()
  local key
  key="$(client_env KEY)"
  [[ -z "$key" ]] || auth=(-H "Authorization: Bearer $key")
  docker exec "$model_client" curl -sS "${auth[@]}" -o /tmp/models.json -w '%{http_code}' \
    http://models.wonton.internal:$WONTON_DEV_PORT/v1/models
}
expect "没有网关：核心报网关不可用" "$(request_models)" 503
expect "错误类别" "$(docker exec "$model_client" cat /tmp/models.json | json error.kind)" models_unavailable
# 从开发索引里把网关按包装一遍：核心现在接受 kind: system，网页和凭据都由核心照常创出来。
magpie_dir="$(mktemp -d)"
python3 - "$here/store" "$magpie_dir" <<'PY'
import json
import pathlib
import re
import sys

store, out = map(pathlib.Path, sys.argv[1:])
blobs = store / "blobs"
entries = json.loads((store / "index.json").read_text())["plugins"]
entry = next(e for e in entries if re.search(r"^\s+id:\s*ai\.usemagpie\.magpie\s*$", e["compose"], re.M))
(out / "docker-compose.yml").write_text(entry["compose"])
for path, digest in entry["files"].items():
    (out / path).parent.mkdir(parents=True, exist_ok=True)
    (out / path).write_bytes((blobs / digest).read_bytes())
PY
installed="$("$dev" install "$magpie_dir" --yes 2>/dev/null)" || fail "模型网关安装失败：$installed"
rm -rf "$magpie_dir"
for _ in $(seq 180); do [[ "$(plugin_state ai.usemagpie.magpie)" == running ]] && break; sleep 1; done
expect "从商店装上了模型网关" "$(plugin_state ai.usemagpie.magpie)" running
expect "有网关：模型列表转给网关" "$(request_models)" 200
# 网关里的使用方凭据。健康探测凭据（`<网关>/health`）是核心自己的，不算使用方。
gateway_keys() { docker exec "$magpie" magpie gateway-key list | awk 'NR > 1 && $2 !~ /\/health$/ { print $2 }'; }
expect "网关里的凭据以使用方命名" "$(gateway_keys | grep -cx app.model-client)" 1
model_key="$(client_env KEY)"
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
revoked_code="$(docker run --rm --network "container:$core" "$curl_image" --noproxy '*' \
  --max-time "$SMOKE_WEB_MAX_TIME" --connect-timeout "$SMOKE_CONNECT_TIMEOUT" -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $model_key" "http://$magpie:3425/v1/models")"
expect "吊销后旧凭据换不到网关的上游凭据（网关回 HTTP $revoked_code）" "$(docker exec "$fake_upstream" grep -c "$fake_key" /tmp/auth.log)" 0
docker exec "$magpie" magpie provider rm smoke-fake >/dev/null || fail "删不掉假上游"
docker rm -f "$fake_upstream" >/dev/null
docker stop "$magpie" >/dev/null
# 核心把停掉的网关记成 degraded 之后才不再转发，否则请求会先撞到已停的地址回 502。
for _ in $(seq 60); do [[ "$(plugin_state ai.usemagpie.magpie)" != running ]] && break; sleep 1; done
expect "网关停了" "$(plugin_state ai.usemagpie.magpie)" degraded
expect "网关没启动：核心报网关不可用" "$(request_models)" 503
expect "错误类别" "$(docker exec "$model_client" cat /tmp/models.json | json error.kind)" models_unavailable
# 后面的 Hermes、MCP、files、model-page、restart 几步还要用这个网关，重新起来放着，最后一步再卸。
docker start "$magpie" >/dev/null
for _ in $(seq 60); do [[ "$(docker inspect -f '{{.State.Health.Status}}' "$magpie")" == healthy ]] && break; sleep 1; done
expect "网关重新启动" "$(docker inspect -f '{{.State.Health.Status}}' "$magpie")" healthy
expect "卸载使用方" "$("$dev" uninstall app.model-client --purge | json data_removed)" true

fi
if step tunnel "已配对设备经隧道打开插件：不用登录，直接是主人"; then
expect "家里服务器以外的地址不进隧道" \
  "$(t -o /dev/null -w '%{http_code}' http://cdn.example.com/app.js)" 403
expect "同机其他程序不带会话凭据，被本地代理拒绝" \
  "$(curl_in "$browser" --noproxy '' -sS -o /dev/null -w '%{http_code}' -x http://127.0.0.1:18008 "$grafana_url/api/user")" 407
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "未登记的设备被隧道拒绝" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
grep -qi '^x-wonton-tunnel-error: unauthorized' <<<"$reply" || fail "缺少 unauthorized 原因：$reply"
# 局域网里的浏览器用 IP 打开状态页，上面有首次配对码；手机拿它经 wonton/pair/0 成为主人的设备。
lan_page() { curl_in "$browser" --noproxy '*' -sS "http://127.0.0.1:$WONTON_DEV_PORT/"; }
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

fi
# 主人的 Matrix 访问令牌：要在 tunnel 一步把手机配对之后才拿得到（未配对的设备被隧道拒绝）。
# 分段跑跳过 tunnel 时，凭据和配对状态都是上一段留在栈里的，这里照样现查。
# 只在分段跑、而且这一轮不跑 matrix 一步时提前取（全量跑里 matrix 就在 hermes 前面，自己会登录；
# 没有手机凭据（假 docker 的用例里）也取不到）。
if [[ -z "${access:-}" && -n "${credential:-}" && -n "${SMOKE_ONLY:-}" && ",$SMOKE_ONLY," != *,matrix,* ]]; then
  matrix_url="${matrix_url:-http://matrix.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT}"
  login="$(t -X POST "$core_url/_wonton/matrix/login-token")"
  body="$(python3 -c 'import json, sys
print(json.dumps({"type": sys.argv[1], "token": sys.argv[2], "initial_device_display_name": "wonton smoke"}))' \
    "$(json login_type <<<"$login")" "$(json token <<<"$login")")"
  # 假 docker 的用例里隧道答不了，拿不到就当空着（用它的都是跳过的步骤）。
  access="$(t -X POST -H 'content-type: application/json' --data-binary "$body" \
    "$matrix_url/_matrix/client/v3/login" 2>/dev/null | json access_token 2>/dev/null || true)"
fi
if step matrix "已配对设备经隧道登录 Matrix（ADR-0009 的 App 接口）"; then
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
# 主人 m.direct 里和这个用户的私聊，一行一个。
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

# 回声 Bot 的安装、加密收发、crypto store 丢失与重装恢复：smoke-echo-bot.sh。
# 本机装了 shellcheck 就顺手检查一遍：它里面的变量都来自冒烟本体，SC2154 免不了，所以不作门槛。
# shellcheck source=deploy/dev/core/smoke-echo-bot.sh
if command -v shellcheck >/dev/null; then shellcheck "$here/smoke-echo-bot.sh" || true; fi
source "$here/smoke-echo-bot.sh"

fi
if step hermes "Hermes（默认 Agent）：核心自己装好，就绪后私聊才出现；没有模型时提示，加了 provider 就能对话"; then
# 核心启动后在后台装 Hermes（WONTON_DEFAULT_PLUGINS），比主人配对、登录 Matrix 早：冒烟没有手动安装它。
# 本机没有它的镜像时要先拉约 4 GB；装不上时核心会自动重试，插件列表里写着原因。
for _ in $(seq 900); do [[ "$(plugin_state com.nousresearch.hermes)" == running ]] && break; sleep 1; done
[[ "$(plugin_state com.nousresearch.hermes)" == running ]] ||
  fail "核心没有装好 Hermes：$("$dev" list | grep -F com.nousresearch.hermes || echo absent)"
ok "核心在后台装好了 Hermes，没有手动安装"
# Tuwunel 的库是 volume、跨栈留着，前几轮的 @hermes 可能还在，核心加 -2 之类的后缀。
hermes_user="$(hermes_env MATRIX_USER_ID)"
[[ "$hermes_user" == @hermes*"$WONTON_DEV_DOMAIN" ]] || fail "Hermes 的账号不对：$hermes_user"
ok "Hermes 是一个 Bot：$hermes_user"
hermes_room="$(direct_rooms "$hermes_user")"
[[ "$hermes_room" == '!'* && "$hermes_room" != *$'\n'* ]] ||
  fail "主人的 m.direct 里应有一个和 Hermes 的私聊：$hermes_room"
ok "主人的 m.direct 里已经有和 Hermes 的私聊"
expect "主人和 Hermes 都已经在私聊里" \
  "$(owner_api "/rooms/$hermes_room/joined_members" | python3 -c 'import json, sys; print(" ".join(sorted(json.load(sys.stdin)["joined"])))')" \
  "$hermes_user @owner:$WONTON_DEV_DOMAIN"
expect "私聊不加密" \
  "$(owner_api "/rooms/$hermes_room/state" | python3 -c 'import json, sys; print(any(e["type"] == "m.room.encryption" for e in json.load(sys.stdin)))')" False
# 主人加入私聊的时间（Tuwunel 的事件时间）和网关容器第一次过健康检查的时间（Docker 事件）在同一台主机上。
# 事件监听从冒烟一开始就在，全新跑时一定记得到。分段跑时这两件事早在上一次跑里发生过，事件文件是空的
# （健康日志只留最近几条，容器起停也不等于 Bot 就绪），私聊是否存在上面已经查过，这里不查时效。
ready_ns="$(awk '$2 == "health_status:" && $3 == "healthy" { print $1; exit }' "$hermes_events")"
if [[ -z "$ready_ns" ]]; then
ok "分段跑：本次没有 Hermes 的健康事件，私聊出现的时效不查"
else
joined_ms="$(owner_api "/rooms/$hermes_room/members" | python3 -c 'import json, sys
print(next(e["origin_server_ts"] for e in json.load(sys.stdin)["chunk"]
           if e["state_key"] == sys.argv[1] and e["content"]["membership"] == "join"))' "@owner:$WONTON_DEV_DOMAIN")"
after_ms=$((joined_ms - ready_ns / 1000000))
((after_ms >= 0)) || fail "Hermes 就绪之前 $((-after_ms)) 毫秒，主人就进了私聊"
((after_ms < 30000)) || fail "Hermes 就绪 $((after_ms / 1000)) 秒后私聊才出现"
ok "Hermes 就绪后 $(printf '%d.%03d' $((after_ms / 1000)) $((after_ms % 1000))) 秒，私聊出现（主人加入了它）"
fi
# Hermes 自 v0.8.0（上游语义版本）起不申请 models：模型走它自己的供应商接入（网页的 Models 页）。这里按主人的做法给它的
# Bot 配一个 OpenAI 兼容服务指向宿主的 magpie；凭据按 Bot 命名建在宿主 magpie 上，限定到这次的模型，
# cleanup 按建 key 时记下的 id 删掉（失败退出也删），凭据不进日志、不写进仓库。不读取、不复制宿主 magpie 的
# 配置和密钥。Hermes 容器能解析 host.docker.internal 就直接连（Docker Desktop、OrbStack 给所有容器这个名字）；
# Linux 的 Docker 不给，解析不到时在宿主机上起一个 socat 转发（监听所有地址），用 Hermes 自己网络的网关地址
# 连，宿主的 magpie 照旧只监听回环也行。
#
# 用 busybox 的 ping 探名字：解析失败打 “bad address”，能解析就先打一行 PING 再等回包（ICMP 被挡不影响
# 判断），所以只看第一行。两样都没有说明探针自己坏了（镜像拉不下来、网络不在），不能悄悄改走转发。
host_name_resolves() {
  local out
  out="$(docker run --rm --network "$hermes_network" "$busybox_image" \
    sh -c 'ping -c1 -W1 host.docker.internal 2>&1' 2>&1)" || true
  if grep -q 'bad address' <<<"$out"; then return 1; fi
  if head -1 <<<"$out" | grep -q '^PING'; then return 0; fi
  fail "探 host.docker.internal 失败：$out"
}

# 宿主 magpie 的 CLI：设了 WONTON_SMOKE_HOST_MAGPIE 就一律进那个容器；否则宿主上装了 magpie 就用它，
# 再不然找发布 3425 的容器。
host_magpie() {
  local container="${WONTON_SMOKE_HOST_MAGPIE:-}"
  if [[ -n "$container" ]]; then docker exec -i "$container" magpie "$@"; return; fi
  if command -v magpie >/dev/null; then magpie "$@"; return; fi
  container="$(docker ps --filter publish=3425 --format '{{.Names}}' | head -1)"
  [[ -n "$container" ]] || fail "找不到宿主 magpie：宿主上放一个发布 3425 的 magpie 容器，或设 WONTON_SMOKE_HOST_MAGPIE=<容器名>"
  docker exec -i "$container" magpie "$@"
}

model_port=3425
if [[ -n "${WONTON_SMOKE_MODEL_RELAY:-}" ]]; then
  # 宿主上已经有转发（magpie 不在 3425）：两个分支都直接用它，容器本身到得了宿主地址。
  model_host="${WONTON_SMOKE_MODEL_RELAY%:*}"
  model_port="${WONTON_SMOKE_MODEL_RELAY##*:}"
  [[ -n "$model_host" && "$model_host" != "$model_port" && "$model_port" =~ ^[0-9]+$ ]] ||
    fail "WONTON_SMOKE_MODEL_RELAY 要写成 <地址>:<端口>：$WONTON_SMOKE_MODEL_RELAY"
elif host_name_resolves; then
  model_host=host.docker.internal
else
  model_host="$(docker network inspect "$hermes_network" --format '{{(index .IPAM.Config 0).Gateway}}')"
  model_port=13425
  # 宿主的防火墙要放行插件网段到这个端口（默认的 DOCKER-USER 链放行 docker 网桥到宿主的流量）。
  # KEEP=1 的上一轮会把转发留着（它不在 compose 项目里），先删掉同名的再起。
  docker rm -f "$model_relay" >/dev/null 2>&1 || true
  run_container "$model_relay" --rm --network host "$socat_image" \
    "TCP-LISTEN:$model_port,fork,reuseaddr" "TCP:127.0.0.1:3425" >/dev/null ||
    fail "起不了到宿主 magpie（127.0.0.1:3425）的转发"
  # 端口被占时 docker run -d 照样返回 0：看容器还在不在。
  [[ "$(docker inspect -f '{{.State.Running}}' "$model_relay" 2>/dev/null)" == true ]] ||
    fail "转发没起来：$(docker logs "$model_relay" 2>&1 | tail -3)"
fi
model_url="http://$model_host:$model_port/v1"
hermes_key="$(openssl rand -hex 24)"
host_model_key_name="com.nousresearch.hermes/default"
# 同名 add 是追加（旧的在前），按前后列表的差集挑出刚建的那条，别动到宿主上原有的同名凭据。
keys_before="$(host_magpie gateway-key list 2>/dev/null | awk 'NR > 1 { print $1 }' | sort)" ||
  fail "建凭据前读不到宿主 magpie 的凭据列表"
host_magpie gateway-key add "$host_model_key_name" --key - <<<"$hermes_key" >/dev/null ||
  fail "宿主 magpie 建不了 Hermes 的网关凭据"
host_model_key_id="$(comm -13 <(printf '%s\n' "$keys_before") \
  <(host_magpie gateway-key list | awk 'NR > 1 { print $1 }' | sort) | grep . | tail -1)" ||
  fail "建了凭据但读不到凭据列表"
[[ -n "$host_model_key_id" ]] || fail "建了凭据但列表里没有新的一条"
host_magpie gateway-key models "$host_model_key_id" "$model" >/dev/null || fail "Hermes 的凭据限定模型失败"
expect "宿主上的凭据以 Hermes 的 Bot 命名" \
  "$(host_magpie gateway-key list | awk -v i="$host_model_key_id" '$1 == i { print $2 }')" "$host_model_key_name"
expect "宿主上的凭据只放行这次的模型" "$(host_magpie gateway-key models "$host_model_key_id" | sed 's/^[^:]*: //')" "only $model"
# 从 Hermes 自己的容器真连一次：地址、凭据、防火墙都得通。
[[ " $(hermes_models) " == *" $model "* ]] || fail "Hermes 容器里列不到宿主 magpie 的模型：$(hermes_models)"
ok "从 Hermes 的容器经 $model_host:$model_port 能列出宿主 magpie 的模型"
# 回复和用量的断言在 smoke-hermes.sh 里。
# shellcheck source=deploy/dev/core/smoke-hermes.sh
source "$here/smoke-hermes.sh"
said="$(mktemp)"
# Hermes 默认说英文；按 v0.22.7 的原文，旧版、中文的说法（“模型设置”）也认。配过模型之后
# （分段跑会这样）靠这个串判断有没有再提示。
no_model_hint='还没有设置模型|模型设置|add an AI provider|No model is available yet|add a model provider'
# 分段跑时上一次已经给 Hermes 配过模型，不再有“没有模型”的提示；这个结论只在全新跑时查。
configured_before="$(docker exec "$hermes" sh -c 'curl -sS --max-time 30 http://127.0.0.1:9119/api/model/info' | json model)"
talk_to_hermes "$said" "你好" || fail "Hermes 没有回答：$(cat "$said")"
grep '^第' "$said" | sed 's/^/   · /'
if [[ -n "$configured_before" ]]; then
ok "分段跑：Hermes 已经配过模型，不再提示添加"
else
grep -qE "$no_model_hint" "$said" || fail "没有模型时 Hermes 应提示去 Models 页添加模型服务商：$(cat "$said")"
ok "没有模型时，Hermes 提示主人去 Models 页添加模型服务商"
fi
# 给 Hermes 配 provider：走它自己网页背后的接口。接口要页面注入的会话令牌，从首页 HTML 里取。
hermes_token="$(docker exec "$hermes" sh -c 'curl -fsS http://127.0.0.1:9119/' |
  sed -n 's/.*__HERMES_SESSION_TOKEN__="\([^"]*\)".*/\1/p')"
[[ -n "$hermes_token" ]] || fail "Hermes 的页面没有给出会话令牌"
# 分段跑时 Hermes 上还留着上一次的同名服务（核心代登录的插件网关也在用这个 id）：再 add 只会并进旧条目，
# 地址和凭据还是旧的。先删掉再建；全新跑时没有它，404 也算过。
docker exec "$hermes" sh -c "curl -sS --max-time 30 -X DELETE http://127.0.0.1:9119/api/providers/custom-endpoints/magpie \
  -H 'X-Hermes-Session-Token: $hermes_token'" >/dev/null || fail "删不掉 Hermes 上旧的模型服务"
endpoint="$(printf '{"name":"宿主 magpie","base_url":"%s","model":"%s","api_key":"%s","api_mode":"chat_completions","context_length":65536,"make_default":true}' \
  "$model_url" "$model" "$hermes_key" |
  docker exec -i "$hermes" sh -c "curl -sS --max-time 120 -X POST http://127.0.0.1:9119/api/providers/custom-endpoints \
    -H 'X-Hermes-Session-Token: $hermes_token' -H 'content-type: application/json' \
    -H 'Origin: http://127.0.0.1:9119' --data-binary @-")"
expect "加了 provider，Hermes 拿它当当前模型" "$(json ok <<<"$endpoint") $(json current.model <<<"$endpoint")" "true $model"
# Hermes 要求模型至少 64k 上下文（v0.22.7 起，工具轮次动手前按 Ollama 式运行窗口拦；网关这种私有地址
# 会被当成本地服务）。magpie 对没标上下文的上游回 32768，所以 context_length 和 ollama_num_ctx 都设成 64k：
# 前者是显示的窗口，后者是真正发出去的 num_ctx，也是拦截的判据。
docker exec "$hermes" sh -c "curl -sS --max-time 30 -X PUT http://127.0.0.1:9119/api/config \
  -H 'X-Hermes-Session-Token: $hermes_token' -H 'content-type: application/json' \
  -H 'Origin: http://127.0.0.1:9119' -d '{\"config\":{\"model\":{\"context_length\":65536,\"ollama_num_ctx\":65536}}}'" >/dev/null ||
  fail "Hermes 的上下文设不进去"
expect "Hermes 用放宽后的上下文" \
  "$(docker exec "$hermes" sh -c 'curl -sS --max-time 30 http://127.0.0.1:9119/api/model/info' | json effective_context_length)" 65536
ok "Hermes 的模型设置指向宿主的 magpie"
# 后面“模型设置”网页那一步要在插件网关里看得到、开关得了一个上游：用 Hermes 这条凭据加一个指向宿主 magpie
# 的 provider（Hermes 自己不经过它，但网页得有内容）。插件网关经自己的网络网关连转发，no_proxy 带上它。
docker exec -e no_proxy="$model_host,localhost,127.0.0.1" -e NO_PROXY="$model_host,localhost,127.0.0.1" "$magpie" \
  magpie provider add upstream url="$model_url" key="$hermes_key" models="${model#*/}" >/dev/null ||
  fail "插件网关加不上指向宿主 magpie 的上游（$model_url）"
round=0
for text in "你好，用一句话介绍你自己。" "我刚才让你做什么？"; do
  round=$((round + 1))
  round_start="$(date +%s)"
  talk_to_hermes "$said" "$text" || fail "和 Hermes 的对话失败：$(cat "$said")"
  grep '^第' "$said" | sed "s/^第 1 轮/第 $round 轮/; s/^/   · /"
  expect_model_reply "$said" "$round_start" "$round"
  ! grep -qE "$no_model_hint" "$said" || fail "有模型了，Hermes 不该再提示去 Models 页设置：$(cat "$said")"
done
expect_hermes_memory "$said"
ok "经隧道和 Hermes 对话两轮"
rm -f "$said"

fi
if step mcp "让 Agent 动手（MCP 总线）：Hermes 请主人确认后装上文件搜索，再用它在授权的文件夹里找文件"; then
hermes_mcp="$(hermes_env WONTON_MCP_URL)"
case "$hermes_mcp" in
  "http://mcp.wonton.internal:$WONTON_DEV_PORT/mcp/"?*) ;;
  *) fail "Hermes 拿到的 MCP 总线地址应带总线令牌：$hermes_mcp" ;;
esac
# 经隧道在和 Hermes 的私聊里说一句话，收它的回答，直到它 180 秒不再说话（它先说一句再去调要等确认的工具，
# 工具往返加慢模型会先超过 90 秒的静默窗口）：
# $1 输出文件，$2 话。
# 确认表里最新的一张卡片（卡片不进 Matrix 房间），输出“ID 状态”：有等回答的取最急的那张，否则取最近结束的。
# 等确认表里出现待确认的卡片，输出卡片 ID。
asked="$(mktemp)"
ask_hermes "$asked" "帮我装一下官方插件里的“文件搜索”。" &
asking=$!
card=""
for _ in $(seq 900); do
  card="$(latest_card)"
  [[ "$card" == *" pending "* ]] && break
  sleep 1
done
[[ "$card" == *" pending "* ]] || fail "待确认列表里没有出现卡片（$card）：$(cat "$asked")"
card_id="${card%% *}"
ok "Hermes 发起安装，核心发了确认卡片，还没有装"
expect "插件还没装" "$(plugin_state app.wonton.files)" absent
confirm_url="$core_url/_wonton/confirmations/$card_id"
# 按 IP 打核心接口必须带域名 Host：IP 只回局域网状态页，API 一律 404（防 DNS 重绑定，dev.sh 头部有完整说明）。
expect "Hermes 的插件网络里来的请求答不了确认" "$(answer docker run --rm --network "$hermes_network" \
  "$curl_image" --noproxy '*' --max-time "$SMOKE_WEB_MAX_TIME" --connect-timeout "$SMOKE_CONNECT_TIMEOUT" \
  -H "Host: $WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" "http://$core:$WONTON_DEV_PORT/_wonton/confirmations/$card_id")" 401
expect "局域网浏览器（有核心会话）答不了确认" "$(answer b "$confirm_url")" 401
expect "主人的手机经隧道确认" "$(answer t "$confirm_url")" 200
for _ in $(seq 180); do [[ "$(plugin_state app.wonton.files)" == running ]] && break; sleep 1; done
expect "主人确认后装好了文件搜索" "$(plugin_state app.wonton.files)" running
for _ in $(seq 30); do [[ "$(latest_card)" == *"$card_id succeeded"* ]] && break; sleep 1; done
[[ "$(latest_card)" == *"$card_id succeeded"* ]] || fail "卡片没有更新成装好了：$(latest_card)"
ok "卡片更新成装好了"
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
audited wonton__install_plugin approved || fail "审计里没有主人确认的安装：$audit"
audited 'app-wonton-files__[a-z_]+' allowed || fail "审计里没有找文件：$audit"
audited wonton__share_file allowed || fail "审计里没有分享文件：$audit"
ok "审计日志记下了确认过的安装、找文件和分享文件"
grep -q 'wonton://file/' "$asked" || fail "Hermes 的回答里没有文件链接（App 显示不成文件卡片）：$(cat "$asked")"
ok "Hermes 发出了文件链接，App 显示成文件卡片"

fi
# 停链、出箱和说话人（ADR-0061 第 10 节）：smoke-turns.sh。放在 files 之前：files 一步最后卸载 Hermes。
# shellcheck source=deploy/dev/core/smoke-turns.sh
source "$here/smoke-turns.sh"
hermes_user="${hermes_user:-}"
hermes_room="${hermes_room:-}"
if step files "Bot 存文件（ADR-0027）：主人按路径批准一次，Hermes 就能往里存，Navidrome 看得到"; then
navidrome_src="$(mktemp -d)"
git clone -q --depth 1 --branch v0.1.1 -c advice.detachedHead=false https://github.com/geraldpeng6/wonton-plugin-navidrome "$navidrome_src"
installed="$("$dev" install "$navidrome_src" --yes 2>&1)" || fail "Navidrome 安装失败：$installed"
rm -rf "$navidrome_src"
expect "Navidrome 装好了" "$(plugin_state org.navidrome.navidrome)" running
shares="$WONTON_DEV_DATA/shares"
outbox="$WONTON_DEV_DATA/apps/$(tr . - <<<com.nousresearch.hermes)/outbox"
# WONTON_SMOKE_SONG_URL：换成真的下载一首歌，经 Hermes 出网。默认由它自己生成，不依赖外面的服务。
song_ext=wav
if [[ -n "${WONTON_SMOKE_SONG_URL:-}" ]]; then song_ext="${WONTON_SMOKE_SONG_URL##*.}"; fi

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
[[ "$(latest_card)" == *"$card_id succeeded"* ]] || fail "第二次存文件又发了卡片：$(latest_card)"
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
# 凭据是冒烟按 Bot 的名字直接建在宿主 magpie 上的（Hermes 不申请 models，核心不管它），冒烟自己删。
# 宿主上可能还有同名旧条：按 id 认，删完旧条还在也不能算失败。
host_magpie gateway-key remove "$host_model_key_id" >/dev/null || fail "删不掉 Hermes 在宿主 magpie 上的凭据"
expect "宿主 magpie 上只删了冒烟建的凭据" \
  "$(host_magpie gateway-key list | awk -v i="$host_model_key_id" '$1 == i')" ""
host_model_key_id=""
expect "卸载 Hermes：存文件的授权清掉了" "$(docker exec "$core" find "$WONTON_DEV_DATA" -path '*save-grants*' -name 'com.nousresearch.hermes.json')" ""

fi
if step model-page "“模型设置”网页（sso: proxy）：主人不用输网页密钥就能打开，密钥不出网关"; then
magpie_url="http://ai-usemagpie-magpie.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"
# 核心生成、注入 magpie 容器的网页密钥，只用来检查它没有出现在任何响应和 Cookie 罐里。
web_key="$(docker inspect "$magpie" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MAGPIE_WEB_KEY=//p')"
[[ "$web_key" =~ ^[0-9a-f]{64}$ ]] || fail "magpie 没有拿到核心生成的网页密钥"
# 没有任何 Cookie 的浏览器。
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

fi
if step revoke "吊销手机"; then
"$dev" revoke "$device_id" >/dev/null
reply="$(t -o /dev/null -w '%{http_code}' -D - "$grafana_url/api/user")"
expect "吊销后立即被拒" "$(head -1 <<<"$reply" | cut -d' ' -f2)" 403
docker rm -f "$phone" >/dev/null

fi
if step whoami "没有 OIDC 的插件：网关转发并剥离网关 Cookie"; then
installed="$("$dev" install "$repo/plugins/examples/whoami" --yes 2>/dev/null)"
expect "whoami 安装完成" "$(json status.state <<<"$installed")" running
echoed="$(b -L -H 'Accept: text/html' -H 'X-Forwarded-For: 6.6.6.6' "$whoami_url/")"
grep -qi "^X-Forwarded-Host: app-whoami.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT" <<<"$echoed" || fail "缺少 X-Forwarded-Host：$echoed"
! grep -q 'wonton_app=\|wonton_session=' <<<"$echoed" || fail "网关 Cookie 泄露给了插件：$echoed"
! grep -q '6.6.6.6' <<<"$echoed" || fail "伪造的 X-Forwarded-For 被转发：$echoed"
ok "插件收到转发头，收不到网关 Cookie 和伪造的转发头"
# 从 whoami 的插件网络里冒充 Bot 和模型网关的使用方：网关按来源地址认出插件，它两项权限都没申请。
for internal in "matrix.wonton.internal /_matrix/client/versions" "models.wonton.internal /v1/models" "mcp.wonton.internal /mcp"; do
  read -r host path <<<"$internal"
  denied="$(from_whoami "$host" "$path")"
  expect "没申请权限的插件进不了 $host" "$(head -1 <<<"$denied" | cut -d' ' -f2)" 403
  grep -qi '^x-wonton-error: permission_denied' <<<"$denied" || fail "缺少 permission_denied：$denied"
done
expect "whoami 卸载" "$("$dev" uninstall app.whoami --purge | json data_removed)" true

fi
if step restart "核心重启后恢复插件"; then
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

fi
if step uninstall "卸载干净"; then
# 从镜像里现装的网关后面几步一直在用，到这一步才卸。
gateway_report="$("$dev" uninstall ai.usemagpie.magpie --purge)"
expect "删除了网关容器" "$(json containers <<<"$gateway_report" | grep -c magpie)" 1
expect "删除了网关网络" "$(json networks <<<"$gateway_report")" "[\"$magpie_network\"]"
expect "删除了网关数据" "$(json data_removed <<<"$gateway_report")" true
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
if docker exec "$core-hydra-1" wget -q -T 10 -O /dev/null http://127.0.0.1:4445/admin/clients/app.grafana 2>/dev/null; then
  fail "身份提供方里还有 app.grafana 客户端"
fi
ok "身份提供方里没有 app.grafana 客户端"
expect "网关不再路由这个插件" \
  "$(b -o /dev/null -w '%{http_code}' -H 'Accept: application/json' "$grafana_url/")" 404
fi

[[ -n "${SMOKE_ONLY:-}" ]] && { printf '\n选中的步骤通过：%s\n' "$SMOKE_ONLY"; exit 0; }
printf '\n端到端冒烟通过：安装 %ss，卸载 %ss\n' "$install_seconds" "$uninstall_seconds"
