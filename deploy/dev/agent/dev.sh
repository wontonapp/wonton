#!/usr/bin/env bash
# M0-4 开发环境：Tuwunel + magpie + Hermes。本脚本扮演核心：生成密钥、注册账号、
# 给 Hermes 签发 Matrix 登录和 magpie 网关凭据、把模型上游写进 magpie。
#
#   dev.sh up         首次生成密钥与账号，配置 magpie，启动全部服务
#   dev.sh providers  按配置文件重新写入模型上游和路由组（改了上游或模型后用）
#   dev.sh smoke      冒烟：magpie 鉴权检查 + 和 Bot 的多轮对话（见 smoke.sh）
#   dev.sh web        打印 magpie 网页（模型设置）的登录链接
#   dev.sh down       停止，保留数据
#   dev.sh reset      停止并删除这个项目的数据卷和 .secrets/
#   dev.sh status | logs [服务]
#
# 配置文件默认是同目录的 .env（参考 .env.example），可用 WONTON_AGENT_ENV 指定别的路径；
# 每条命令都先读它，环境里已经设置的变量优先于文件（见 load-env.sh）。其中 WONTON_AGENT_PROJECT
# 是 compose 项目名（默认 wonton-dev-agent），数据卷和网络按它区分；.secrets/ 属于检出目录，
# 所以同机的第二套环境要放在另一个检出目录（git worktree）里，并换项目名和两个端口。
# 依赖：docker compose、curl、jq、openssl。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=load-env.sh
source "$here/load-env.sh"
secrets="$here/.secrets"
env_file="${WONTON_AGENT_ENV:-$here/.env}"
group_id="wonton-default"
bot_user="hermes"
owner_user="owner"
# magpie 里 Hermes 网关凭据的名字，用插件 id，用量统计按它归属。
hermes_key_name="com.nousresearch.hermes"
local_model="Qwen/Qwen2.5-1.5B-Instruct-GGUF:Q4_K_M"

die() {
  echo "错误：$*" >&2
  exit 1
}

log() { echo "==> $*"; }

load_env() {
  load_env_file "$env_file"
  server_name="${WONTON_SERVER_NAME:-wonton.test}"
  base_url="http://127.0.0.1:${WONTON_AGENT_MATRIX_PORT:-18008}"
  # compose.yaml 的 name: 读这个变量。down、reset 也要靠它找到对的项目，所以每条命令都先读配置。
  project="${WONTON_AGENT_PROJECT:-wonton-dev-agent}"
  export WONTON_AGENT_PROJECT="$project"
}

# 读写 .secrets/ 里的 KEY=VALUE 文件；值都是十六进制或 Matrix 令牌，不含空白和引号。
get_var() {
  [[ -f "$1" ]] || return 0
  sed -n "s/^$2=//p" "$1" | tail -n 1
}

set_var() {
  local file="$1" name="$2" value="$3" tmp
  tmp="$(mktemp "$secrets/.tmp.XXXXXX")"
  { [[ -f "$file" ]] && grep -v "^$name=" "$file" || true; } >"$tmp"
  printf '%s=%s\n' "$name" "$value" >>"$tmp"
  chmod 600 "$tmp"
  mv "$tmp" "$file"
}

ensure_var() {
  [[ -n "$(get_var "$1" "$2")" ]] || set_var "$1" "$2" "$(openssl rand -hex "${3:-24}")"
}

ensure_secrets() {
  mkdir -p "$secrets"
  chmod 700 "$secrets"
  ensure_var "$secrets/tuwunel.env" TUWUNEL_REGISTRATION_TOKEN
  ensure_var "$secrets/magpie.env" MAGPIE_WEB_KEY
  ensure_var "$secrets/owner.env" OWNER_PASSWORD 12
  ensure_var "$secrets/bot.env" BOT_PASSWORD
  touch "$secrets/hermes.env"
  chmod 600 "$secrets"/*.env
}

# 总是带上 local-model 这个 profile，这样 down/reset 也会清掉它；启动时都点名服务，不会顺带启动它。
compose() {
  docker compose -f "$here/compose.yaml" --project-directory "$here" --profile local-model "$@"
}

magpie() {
  compose exec -T magpie magpie "$@"
}

# 秘密值经标准输入进容器，再替换参数里的 {secret}：不出现在宿主机的命令行和 shell 历史里。
# magpie 的 provider 命令只收参数形式的 key，所以它在容器内的进程参数里会短暂可见。
magpie_with_secret() {
  local secret="$1"
  shift
  # shellcheck disable=SC2016 # 变量在容器里的 sh 展开
  printf '%s\n' "$secret" | compose exec -T magpie sh -c '
    IFS= read -r secret
    for arg do
      shift
      case "$arg" in *"{secret}"*) arg="${arg%%\{secret\}*}$secret${arg#*\{secret\}}" ;; esac
      set -- "$@" "$arg"
    done
    exec magpie "$@"' sh "$@"
}

matrix() {
  local method="$1" path="$2"
  shift 2
  curl -sS -X "$method" "$base_url/_matrix/client/v3$path" -H 'content-type: application/json' "$@"
}

# 服务器里是否已经有这个用户名。
account_exists() {
  local resp
  resp="$(matrix GET "/register/available?username=$1")"
  [[ "$(jq -r '.errcode // empty' <<<"$resp")" == "M_USER_IN_USE" ]]
}

# 用注册令牌走 UIA 注册；用户已存在时返回 1。
register() {
  local user="$1" password="$2" token resp session body
  token="$(get_var "$secrets/tuwunel.env" TUWUNEL_REGISTRATION_TOKEN)"
  account_exists "$user" && return 1
  body="$(jq -nc --arg u "$user" --arg p "$password" '{username: $u, password: $p, inhibit_login: true}')"
  session="$(matrix POST /register -d "$body" | jq -r '.session // empty')"
  [[ -n "$session" ]] || die "注册 $user 时没有拿到 UIA 会话"
  body="$(jq -nc --arg u "$user" --arg p "$password" --arg s "$session" --arg t "$token" \
    '{username: $u, password: $p, inhibit_login: true,
      auth: {type: "m.login.registration_token", token: $t, session: $s}}')"
  resp="$(matrix POST /register -d "$body")"
  jq -e '.user_id' <<<"$resp" >/dev/null || die "注册 $user 失败：$resp"
  log "已注册 $(jq -r .user_id <<<"$resp")"
}

# 账号已经在服务器里，登录却被拒：.secrets/ 里记的密码不是建这个账号时用的那一个，
# 也就是 .secrets/ 和数据卷不是同一套。
secrets_mismatch() {
  die "$1 登录失败：$2
@$1:$server_name 已经在 Tuwunel 里，但 .secrets/ 里的密码登录不上：.secrets/ 和项目 $project 的数据卷不是同一套。
通常是 .secrets/ 被重新生成了（换了检出目录，或只删了 .secrets/），数据卷却还在。
- 数据不要了：先运行 $0 reset（删除项目 $project 的数据卷和 .secrets/，聊天记录会丢，已登录的客户端要重新登录），再运行 $0 up。
- 数据要留：把原来那份 .secrets/ 放回 $secrets；或者让这个检出目录用自己的项目名和端口（WONTON_AGENT_PROJECT 等，见 README），不去动原来那套。"
}

# 密码登录一个新设备，输出 access_token。
login() {
  local body resp
  body="$(jq -nc --arg u "$1" --arg p "$2" --arg d "$3" \
    '{type: "m.login.password", identifier: {type: "m.id.user", user: $u}, password: $p,
      initial_device_display_name: $d}')"
  resp="$(matrix POST /login -d "$body")"
  jq -er '.access_token // empty' <<<"$resp" && return
  if [[ "$(jq -r '.errcode // empty' <<<"$resp")" == "M_FORBIDDEN" ]] && account_exists "$1"; then
    secrets_mismatch "$1" "$resp"
  fi
  die "$1 登录失败：$resp"
}

token_valid() {
  [[ -n "$1" ]] && matrix GET /account/whoami -H "Authorization: Bearer $1" | jq -e '.user_id' >/dev/null
}

ensure_accounts() {
  local owner_password bot_password token
  owner_password="$(get_var "$secrets/owner.env" OWNER_PASSWORD)"
  bot_password="$(get_var "$secrets/bot.env" BOT_PASSWORD)"
  # Tuwunel 把第一个注册的用户设为管理员，所以先注册主人。
  register "$owner_user" "$owner_password" || true
  register "$bot_user" "$bot_password" || true
  token="$(get_var "$secrets/bot.env" BOT_ACCESS_TOKEN)"
  if ! token_valid "$token"; then
    token="$(login "$bot_user" "$bot_password" "Hermes (wonton dev)")"
    set_var "$secrets/bot.env" BOT_ACCESS_TOKEN "$token"
    matrix PUT "/profile/@$bot_user:$server_name/displayname" -H "Authorization: Bearer $token" \
      -d '{"displayname":"Hermes"}' >/dev/null
    log "已为 @$bot_user:$server_name 签发新的登录令牌"
  fi
  # 相当于核心安装 Hermes 插件时注入的 Matrix 变量：Hermes 只拿到令牌，拿不到密码。
  set_var "$secrets/hermes.env" MATRIX_USER_ID "@$bot_user:$server_name"
  set_var "$secrets/hermes.env" MATRIX_ACCESS_TOKEN "$token"
  set_var "$secrets/hermes.env" MATRIX_ALLOWED_USERS "@$owner_user:$server_name"
}

# 核心替主人建好和 Bot 的加密私聊，并把它设为 Bot 的主房间（定时任务结果发到这里）。
# 没有主房间时 Hermes 会在每个新会话的第一条回复里插一段设置提示。
ensure_home_room() {
  local owner="@$owner_user:$server_name" bot="@$bot_user:$server_name" token room body
  room="$(get_var "$secrets/owner.env" OWNER_DM_ROOM)"
  if [[ -z "$room" ]]; then
    token="$(login "$owner_user" "$(get_var "$secrets/owner.env" OWNER_PASSWORD)" "wonton dev.sh")"
    body="$(jq -nc --arg b "$bot" \
      '{preset: "trusted_private_chat", is_direct: true, invite: [$b],
        initial_state: [{type: "m.room.encryption", state_key: "",
          content: {algorithm: "m.megolm.v1.aes-sha2"}}]}')"
    room="$(matrix POST /createRoom -H "Authorization: Bearer $token" -d "$body" | jq -er .room_id)" ||
      die "建主人和 Bot 的私聊失败"
    # m.direct 让客户端把它显示成私聊。
    matrix PUT "/user/$owner/account_data/m.direct" -H "Authorization: Bearer $token" \
      -d "$(jq -nc --arg b "$bot" --arg r "$room" '{($b): [$r]}')" >/dev/null
    matrix POST /logout -H "Authorization: Bearer $token" >/dev/null
    set_var "$secrets/owner.env" OWNER_DM_ROOM "$room"
    log "已建主人和 Bot 的加密私聊 $room"
  fi
  set_var "$secrets/hermes.env" MATRIX_HOME_ROOM "$room"
}

key_id() {
  magpie gateway-key list | awk -v name="$1" 'NR > 1 && $2 == name { print $1; exit }'
}

# 给 Hermes 发一个只属于它的 magpie 网关凭据，并只允许它用默认路由组。
ensure_gateway_key() {
  local key id
  key="$(get_var "$secrets/hermes.env" WONTON_MODEL_API_KEY)"
  id="$(key_id "$hermes_key_name")"
  if [[ -z "$id" || -z "$key" ]]; then
    [[ -n "$id" ]] && magpie gateway-key remove "$id" >/dev/null
    key="wtn-$(openssl rand -hex 24)"
    printf '%s\n' "$key" | magpie gateway-key add "$hermes_key_name" --key - >/dev/null
    id="$(key_id "$hermes_key_name")"
    [[ -n "$id" ]] || die "magpie 里没找到刚建的网关凭据 $hermes_key_name"
    set_var "$secrets/hermes.env" WONTON_MODEL_API_KEY "$key"
    log "已在 magpie 为 Hermes 建网关凭据（id $id）"
  fi
  magpie gateway-key models "$id" "group/$group_id" >/dev/null
}

# 模型上游：一个 OpenAI 兼容的地址，注册成容器 magpie 里的自定义 provider upstream，
# UPSTREAM_MODELS 按顺序加进路由组。每次按配置文件重建，地址、key、模型都以配置文件为准。
ensure_upstream() {
  [[ -n "${UPSTREAM_URL:-}" ]] || return 0
  local models="${UPSTREAM_MODELS:?配置文件里缺少 UPSTREAM_MODELS（逗号分隔的上游模型 ID）}" list model
  [[ -n "${UPSTREAM_KEY:-}" ]] ||
    die "配置文件里缺少 UPSTREAM_KEY：设了 UPSTREAM_URL 就不能留空（上游是 OrbStack 上开发机里的 magpie 时填任意值，见 .env.example）"
  if magpie provider upstream >/dev/null 2>&1; then
    magpie provider rm upstream >/dev/null
  fi
  magpie_with_secret "$UPSTREAM_KEY" provider add upstream "url=$UPSTREAM_URL" 'key={secret}' \
    "models=$models" >/dev/null
  IFS=, read -ra list <<<"$models"
  for model in "${list[@]}"; do
    members+=("upstream/$model")
  done
}

# 可选的本地小模型：像本地模型插件那样注册成 magpie 的 provider，排在路由组最后。
ensure_local_model() {
  [[ "${WONTON_AGENT_LOCAL_MODEL:-0}" == 1 ]] || return 0
  log "启动本地小模型（首次要下载约 1 GB）"
  compose up -d --wait local-model
  if ! magpie provider local-model >/dev/null 2>&1; then
    # llama.cpp 不校验 key，但 magpie 的自定义 provider 必须填一个。
    magpie provider add local-model url=http://local-model:8080/v1 key=unused "models=$local_model" >/dev/null
  fi
  members+=("local-model/$local_model")
}

configure_providers() {
  members=()
  ensure_upstream
  ensure_local_model
  if ((${#members[@]} == 0)); then
    log "没有可用的模型：在配置文件里设置 UPSTREAM_*（或 WONTON_AGENT_LOCAL_MODEL=1）后运行 $0 providers"
    return
  fi
  local list
  list="$(IFS=,; echo "${members[*]}")"
  if magpie group "$group_id" >/dev/null 2>&1; then
    magpie group set "$group_id" "models=$list" routing=order >/dev/null
  else
    magpie group add "$group_id" "models=$list" routing=order >/dev/null
  fi
  log "路由组 group/$group_id：$list（按顺序尝试）"
}

up() {
  ensure_secrets
  compose up -d --wait tuwunel magpie
  ensure_accounts
  ensure_home_room
  ensure_gateway_key
  configure_providers
  compose up -d --wait hermes
  log "项目 $project 已就绪"
  log "Tuwunel：$base_url（局域网里用本机 IP，端口 ${WONTON_AGENT_MATRIX_PORT:-18008}）"
  log "主人账号：$owner_user，密码在 $secrets/owner.env；和 @$bot_user:$server_name 私聊即可"
  log "冒烟：$0 smoke    模型设置：$0 web"
}

web() {
  echo "http://127.0.0.1:${WONTON_AGENT_MAGPIE_WEB_PORT:-13430}/?k=$(get_var "$secrets/magpie.env" MAGPIE_WEB_KEY)"
}

load_env
case "${1:-}" in
  up) up ;;
  providers) configure_providers ;;
  smoke) WONTON_AGENT_ENV="$env_file" exec "$here/smoke.sh" ;;
  web) web ;;
  down) compose down ;;
  reset)
    log "删除项目 $project 的容器、数据卷，以及 $secrets"
    compose down --volumes
    rm -rf "$secrets"
    ;;
  status) compose ps ;;
  logs) compose logs -f "${@:2}" ;;
  *) die "用法：$0 up|providers|smoke|web|down|reset|status|logs [服务]" ;;
esac
