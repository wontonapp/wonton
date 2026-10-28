#!/usr/bin/env bash
# M0-4 冒烟：在 dev.sh up 起好的环境里验证两件事，任何一项不符就以非零退出。
#
# 1. magpie 鉴权（每个插件独立凭据）：
#    - 其他容器不带凭据、带错凭据都是 401；
#    - Hermes 的凭据只看得到默认路由组，点名其他模型被 403 拒绝；
#    - 新发的凭据立即可用，吊销后立即 401，不用重启 magpie；
#    - magpie 容器内的回环请求不需要凭据（上游设计如此，记录在案）。
# 2. 主人在一个新私聊里和 Bot 两轮对话：第二轮必须答出第一轮给的随机暗号，证明上下文连续，
#    再从 magpie 的用量记录确认这些请求都记在 Hermes 的凭据名下。
#
# 用法：dev.sh smoke（或直接运行本脚本）。依赖：docker compose、curl、jq。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
secrets="$here/.secrets"
env_file="${WONTON_AGENT_ENV:-$here/.env}"
if [[ -f "$env_file" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$env_file"
  set +a
fi
server_name="${WONTON_SERVER_NAME:-wonton.test}"
base_url="http://127.0.0.1:${WONTON_AGENT_MATRIX_PORT:-18008}"
bot="@hermes:$server_name"
group="group/wonton-default"
hermes_key_name="com.nousresearch.hermes"
reply_timeout="${SMOKE_REPLY_TIMEOUT:-180}"
failures=0

get_var() { sed -n "s/^$2=//p" "$1" | tail -n 1; }

compose() { docker compose -f "$here/compose.yaml" --project-directory "$here" "$@"; }

ok() { echo "  ✓ $*"; }

bad() {
  echo "  ✗ $*"
  failures=$((failures + 1))
}

expect() {
  local name="$1" want="$2" got="$3"
  if [[ "$got" == "$want" ]]; then ok "$name（$got）"; else bad "$name：期望 $want，实际 $got"; fi
}

# 从 Hermes 容器（另一个容器，magpie 视为远程）请求 magpie 网关。凭据经环境变量传入，
# 不出现在命令行里。输出“状态码 响应体”。
from_hermes() {
  local key="$1" method="$2" path="$3" body="${4:-}"
  # shellcheck disable=SC2016 # 变量在容器里的 sh 展开
  GW_KEY="$key" GW_BODY="$body" compose exec -T -e GW_KEY -e GW_BODY hermes sh -c '
    set -- -s -o /tmp/gw-body -w "%{http_code}" -X "$1" "http://magpie:3425$2"
    [ -n "$GW_KEY" ] && set -- "$@" -H "Authorization: Bearer $GW_KEY"
    [ -n "$GW_BODY" ] && set -- "$@" -H "content-type: application/json" -d "$GW_BODY"
    code="$(curl "$@")"; printf "%s " "$code"; cat /tmp/gw-body; rm -f /tmp/gw-body' sh "$method" "$path"
}

check_magpie_auth() {
  local hermes_key resp temp_key temp_id models model
  echo "magpie 鉴权"
  hermes_key="$(get_var "$secrets/hermes.env" WONTON_MODEL_API_KEY)"
  resp="$(from_hermes "" GET /v1/models)"
  expect "其他容器不带凭据" 401 "${resp%% *}"
  resp="$(from_hermes "wtn-not-issued-$RANDOM$RANDOM" GET /v1/models)"
  expect "其他容器带未签发的凭据" 401 "${resp%% *}"
  resp="$(from_hermes "$hermes_key" GET /v1/models)"
  expect "Hermes 的凭据列模型" 200 "${resp%% *}"
  models="$(jq -r '[.data[].id] | join(",")' <<<"${resp#* }")"
  expect "Hermes 的凭据只看得到默认路由组" "$group" "$models"
  # 任取一个 provider 直接提供的模型（不经路由组），Hermes 的凭据点名它应被拒绝。
  model="$(compose exec -T magpie curl -s http://127.0.0.1:3425/v1/models |
    jq -r '[.data[].id | select(startswith("group/") | not)][0] // empty')"
  if [[ -n "$model" ]]; then
    resp="$(from_hermes "$hermes_key" POST /v1/chat/completions \
      "$(jq -nc --arg m "$model" '{model: $m, messages: [{role: "user", content: "hi"}], max_tokens: 1}')")"
    expect "Hermes 的凭据点名路由组之外的模型 $model" 403 "${resp%% *}"
  else
    bad "magpie 里还没有任何 provider 模型，无法检查越权"
  fi
  temp_key="$(compose exec -T magpie magpie gateway-key add smoke-temp | tail -n 1)"
  temp_id="$(compose exec -T magpie magpie gateway-key list | awk '$2 == "smoke-temp" { print $1; exit }')"
  resp="$(from_hermes "$temp_key" GET /v1/models)"
  expect "新签发的凭据立即可用" 200 "${resp%% *}"
  compose exec -T magpie magpie gateway-key remove "$temp_id" >/dev/null
  resp="$(from_hermes "$temp_key" GET /v1/models)"
  expect "吊销的凭据立即失效" 401 "${resp%% *}"
  resp="$(compose exec -T magpie curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3425/v1/models)"
  expect "magpie 容器内回环不带凭据（上游的本机例外）" 200 "$resp"
}

mx() {
  local method="$1" path="$2"
  shift 2
  curl -sS -X "$method" "$base_url/_matrix/client/v3$path" \
    -H "Authorization: Bearer $owner_token" -H 'content-type: application/json' "$@"
}

owner_login() {
  local password body
  owner_token="$(get_var "$secrets/owner.env" OWNER_ACCESS_TOKEN)"
  if [[ -n "$owner_token" ]] && mx GET /account/whoami | jq -e .user_id >/dev/null; then
    return
  fi
  password="$(get_var "$secrets/owner.env" OWNER_PASSWORD)"
  body="$(jq -nc --arg p "$password" \
    '{type: "m.login.password", identifier: {type: "m.id.user", user: "owner"}, password: $p,
      initial_device_display_name: "wonton smoke"}')"
  owner_token="$(curl -sS -X POST "$base_url/_matrix/client/v3/login" \
    -H 'content-type: application/json' -d "$body" | jq -er .access_token)"
  printf 'OWNER_ACCESS_TOKEN=%s\n' "$owner_token" >>"$secrets/owner.env"
}

send_text() {
  local txn="smoke-$$-$RANDOM$RANDOM"
  mx PUT "/rooms/$room/send/m.room.message/$txn" \
    -d "$(jq -nc --arg t "$1" '{msgtype: "m.text", body: $t}')" | jq -er .event_id >/dev/null
}

# 等 Bot 在房间里发出下一条消息，放进全局变量 reply。
wait_reply() {
  local deadline=$((SECONDS + reply_timeout)) resp
  reply=""
  while ((SECONDS < deadline)); do
    resp="$(mx GET "/sync?since=$since&timeout=20000&filter=$filter")"
    since="$(jq -r .next_batch <<<"$resp")"
    reply="$(jq -r --arg r "$room" --arg b "$bot" \
      '[.rooms.join[$r].timeline.events[]? | select(.sender == $b and .type == "m.room.message")
        | .content.body] | join("\n")' <<<"$resp")"
    [[ -n "$reply" ]] && return 0
  done
  return 1
}

check_conversation() {
  local code started
  echo "和 Bot 多轮对话"
  owner_login
  room="$(mx POST /createRoom -d "$(jq -nc --arg b "$bot" \
    '{preset: "trusted_private_chat", is_direct: true, invite: [$b], name: "M0-4 smoke"}')" | jq -er .room_id)"
  for _ in $(seq 1 60); do
    mx GET "/rooms/$room/joined_members" | jq -e --arg b "$bot" '.joined[$b]' >/dev/null && break
    sleep 1
  done
  if mx GET "/rooms/$room/joined_members" | jq -e --arg b "$bot" '.joined[$b]' >/dev/null; then
    ok "Bot 已自动加入私聊 $room"
  else
    bad "Bot 60 秒内没有加入私聊 $room"
    return
  fi
  filter="$(jq -rn --arg r "$room" \
    '{room: {rooms: [$r], timeline: {limit: 50}, state: {types: []}, ephemeral: {types: []},
      account_data: {types: []}}, presence: {types: []}, account_data: {types: []}} | tojson | @uri')"
  since="$(mx GET "/sync?timeout=0&filter=$filter" | jq -er .next_batch)"
  code="$((RANDOM % 9000 + 1000))"

  started=$SECONDS
  # shellcheck disable=SC1111 # 中文引号是消息内容
  send_text "请记住这个暗号数字：$code。现在只需要回复“记住了”。"
  if wait_reply; then ok "第一轮回复（$((SECONDS - started)) 秒）：$reply"; else bad "第一轮 ${reply_timeout} 秒内没有回复"; return; fi

  started=$SECONDS
  send_text "我刚才让你记住的暗号数字是多少？只回复这个数字。"
  if ! wait_reply; then bad "第二轮 ${reply_timeout} 秒内没有回复"; return; fi
  if [[ "$reply" == *"$code"* ]]; then
    ok "第二轮回复（$((SECONDS - started)) 秒）答出了暗号 $code：$reply"
  else
    bad "第二轮回复没有答出暗号 $code：$reply"
  fi
}

check_usage() {
  local rows
  echo "magpie 用量归属"
  rows="$(compose exec -T magpie magpie usage --last 15m --csv |
    jq -Rr --arg k "$hermes_key_name" 'select(contains($k))' | wc -l | tr -d ' ')"
  if ((rows > 0)); then
    ok "最近 15 分钟有 $rows 条请求记在 $hermes_key_name 名下"
  else
    bad "magpie 用量里没有记在 $hermes_key_name 名下的请求"
  fi
}

check_magpie_auth
check_conversation
check_usage
if ((failures > 0)); then
  echo "冒烟失败：$failures 项不符"
  exit 1
fi
echo "冒烟通过"
