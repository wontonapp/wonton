#!/usr/bin/env bash
# smoke.sh 里 mcp 之后的三步（ADR-0061 第 10 节）：停链打断 Hermes 正在跑的一轮；Bot 发私聊（wonton__send_message）
# 经主人批准、由出箱真的发到家人手里，拒绝的和停了链的不发；核心按工具调用带的说话人请求头认人，没带时退回按这一轮的
# 会话和在跑的轮次认。由 smoke.sh source，用它前面几步的变量和函数（t、probe_in_hermes_room、pending_card 等）。
# 只断言对外的行为：App 的链列表、Wonton 房间里的卡片、家人手机收到的消息、审计日志、Hermes 容器里的进程、总线给的工具。

marker() { printf 'smoke-%s-%s' "$1" "$(head -c 6 /dev/urandom | od -An -tx1 | tr -d ' \n')"; }
mxid() { printf '@%s:%s' "$1" "$WONTON_DEV_DOMAIN"; }
# App 的链列表（只列在跑的和到上限的），一行一条：链 ID、代表的人、起点房间、状态。
# 探针刚重启时隧道可能还没连上，代理回一句纯文本；等到能解析的列表再往下。
chains() {
  local body="" tries="${1:-30}"
  for _ in $(seq "$tries"); do
    body="$(t "$core_url/_wonton/api/agent/chains" 2>/dev/null)" && [[ $body == \{* ]] && break
    sleep 1
  done
  python3 -c 'import json, sys
for row in json.load(sys.stdin)["chains"]:
    print(row["chain"], row["user"], row["origin"]["room_id"], row["status"])' <<<"$body"
}
chain_ids() { chains "${1:-30}" | awk '{ print $1 }'; }
# 等列表里出现一条不在 $1（已有的链 ID）里、代表 $2、从房间 $3 开始的链，输出链 ID。
new_chain() {
  local found
  for _ in $(seq 240); do
    found="$(chains | awk -v seen=" $1 " -v user="$2" -v room="$3" \
      '$2 == user && $3 == room && index(seen, " " $1 " ") == 0 { print $1; exit }')"
    [[ -n "$found" ]] && { echo "$found"; return 0; }
    sleep 1
  done
  return 1
}
stop_chain() { t -X POST "$core_url/_wonton/api/agent/chains/$1/stop" | json status; }
# 卡片在确认列表里的状态（latest_card 是“ID 状态 ”连成的一行，按 ID 取它后面的状态）。
card_state() { latest_card | tr ' ' '\n' | awk -v id="$1" 'prev == id { print; exit } { prev = $0 }'; }
# 还在等回答的卡片 ID，一行一个。
pending_cards() { latest_card | tr ' ' '\n' | awk 'prev != "" && $0 == "pending" { print prev } { prev = $0 }' | sort; }
# 分段跑对着保留下来的栈时，前面几步的变量都没有：凭据、Hermes 的账号和私聊、主人的 Matrix 令牌现查一次。
# 放在这里而不是加载时：它要用 start_phone 起出来的凭据，也要用上面的 chains。
seed_smoke_only() {
  # 没有手机凭据（假 docker 的用例里隧道起不来）就查不到，留着空给跳过的步骤用。
  if [[ -z "${credential:-}" ]]; then
    hermes_user="${hermes_user:-}"
    hermes_room="${hermes_room:-}"
    return
  fi
  # 房间先看链的记录（核心一侧，不用 Matrix 令牌），再看主人的 m.direct，最后看 Hermes 的日志。
  if [[ -z "${hermes_room:-}" ]]; then
    # 手机重起过，上一轮的会话 Cookie 还留在罐里，会让 Matrix 请求按旧会话认人（M_UNKNOWN_TOKEN）。
    docker exec "$browser" rm -f /tmp/phone-jar || true
    hermes_user="$(hermes_env MATRIX_USER_ID)"
    hermes_room="$(chains 200 | awk -v user="$hermes_user" '$2 == user { room = $3 } END { print room }')"
    direct_body=""
    for _ in $(seq 5); do
      [[ -n "$hermes_room" ]] && break
      direct_body="$(owner_api "/user/@owner:$WONTON_DEV_DOMAIN/account_data/m.direct")"
      hermes_room="$(python3 -c 'import json, sys
  print("\n".join(json.load(sys.stdin).get(sys.argv[1], [])))' "$hermes_user" <<<"$direct_body" 2>/dev/null || true)"
      [[ -n "$hermes_room" ]] && break
      sleep 2
    done
    [[ -n "$hermes_room" ]] ||
      hermes_room="$(docker logs "$hermes" 2>&1 | grep -oE '![A-Za-z0-9_-]{10,}:[A-Za-z0-9.-]+' | tail -1)"
    [[ -n "$hermes_room" ]] || hermes_room="$(hermes_env MATRIX_HOME_ROOM)"
    [[ -n "$hermes_room" ]] ||
      fail "查不到和 $hermes_user 的私聊：链 $(chains 20)；m.direct $direct_body"
  fi
}
# 失败时附上的证据：Hermes 报链、取出箱的日志。
hermes_chain_log() { docker logs "$hermes" 2>&1 | grep -E '调用链|出箱|chain|outbox' | tail -5 || true; }
# Hermes 容器里命令行正好是 `sleep <秒数>` 的进程有几个（终端工具起的）。
sleeping() {
  docker exec "$hermes" /opt/hermes/.venv/bin/python -c 'import glob, sys
want = ("sleep\0" + sys.argv[1] + "\0").encode()
count = 0
for path in glob.glob("/proc/[0-9]*/cmdline"):
    try:
        count += open(path, "rb").read() == want
    except OSError:
        pass
print(count)' "$1"
}
# 让 Hermes 调一次发私聊的工具，收件人是家人 mom，正文是 $1。
send_text() { printf '调用工具 wonton__send_message 给家人发一条私聊：recipient 填 mom，body 只填 %s。只调用这一次，不做别的事。' "$1"; }
# 审计里正文含 $1 的发私聊记录，一行一条：决定 结果。
sent_audit() {
  docker exec "$core" cat "$WONTON_DEV_DATA/core/audit.jsonl" | python3 -c 'import json, sys
for line in sys.stdin:
    if not line.strip():
        continue
    entry = json.loads(line)
    if entry.get("tool") == "wonton__send_message" and sys.argv[1] in json.dumps(entry.get("arguments"), ensure_ascii=False):
        print(entry["decision"], entry.get("outcome") or "")' "$1"
}

# 妈妈：经隧道的另一台手机（本地代理在 127.0.0.1:18009），她的请求和探针都走它。
m() { curl_in "$browser" --noproxy '' -sS -c /tmp/mom-jar -b /tmp/mom-jar -x http://127.0.0.1:18009 -U "$mom_credential" "$@"; }
mom_api() { m -H "Authorization: Bearer $mom_access" "$matrix_url/_matrix/client/v3$1" "${@:2}"; }
# 妈妈的一台新设备在她和 Hermes 的私聊里发话、收回复：$1 输出文件，其余是探针参数。
probe_as_mom() {
  local out="$1" jwt
  shift
  jwt="$(m -X POST "$core_url/_wonton/matrix/login-token" | json token)" || return 1
  docker run --rm --network "container:$core" -v "$mom_probe_store:/probe" "$echo_image" probe \
    --homeserver "$matrix_url" --proxy "http://$mom_credential@127.0.0.1:18009" --login-token "$jwt" \
    --room "$mom_room" --bot "$hermes_user" --device-id "$mom_probe_device" --store /probe \
    --reply any "$@" >"$out" 2>&1
}
mom_pair() {
  docker run --rm --network "container:$core" -v "$mom_key:/tmp/mom.key" --entrypoint tunnel-client "$phone_image" \
    --key /tmp/mom.key pair "$1" --name 妈妈的手机 2>/dev/null || true
}
# 妈妈作为家人加入，Hermes 加她为联系人：主人邀请（给 AI 只读的角色）、她的手机扫码、主人同意、她第一次登录 Matrix、
# 主人发联系人请求、她进 Hermes 发来的私聊；等 Hermes 按新的名单重建、过健康检查。定下 mom_room、mom_credential、mom_access。
# SMOKE_ONLY 对着保留下来的栈跑时妈妈已经在了：跳过邀请和配对，只把她的手机起起来、复用已有的私聊房间。
family_join() {
  local roles role code request contacts_url state invited
  : "${matrix_url:=http://matrix.$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT}"
  contacts_url="$core_url/_wonton/api/bots/com.nousresearch.hermes/default/contacts"
  mom_key="$WONTON_DEV_DATA/phonekeys/mom.key"
  if t "$core_url/_wonton/api/members" | grep -q '"username": *"mom"'; then
    docker rm -f "$mom_phone" >/dev/null 2>&1 || true
    docker run -d --name "$mom_phone" --network "container:$core" -v "$mom_key:/tmp/phone.key" --entrypoint tunnel-client \
      "$phone_image" --key /tmp/phone.key proxy --auth --domain "$WONTON_DEV_DOMAIN" --server "$server_id" \
      --addr "127.0.0.1:$WONTON_DEV_TUNNEL_PORT" --listen 127.0.0.1:18009 >/dev/null
    for _ in $(seq 60); do
      docker logs "$mom_phone" 2>/dev/null | grep '^本地代理' >/dev/null && break
      sleep 1
    done
    docker logs "$mom_phone" 2>/dev/null | grep -q '^本地代理' || fail "妈妈的手机没起来：$(docker logs "$mom_phone" 2>&1 | tail -3)"
    mom_credential="$(docker logs "$mom_phone" 2>/dev/null | sed -n 's/^会话凭据：//p')"
    [[ -n "$mom_credential" ]] || fail "妈妈的手机没有启动：$(docker logs "$mom_phone" 2>&1)"
    mom_access="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
      --proxy "http://$mom_credential@127.0.0.1:18009" --login-token "$(m -X POST "$core_url/_wonton/matrix/login-token" | json token)" \
      --room "$hermes_room" --bot "$hermes_user" --token-only 1 | sed -n 's/^令牌 //p')"
    [[ -n "$mom_access" ]] || fail "妈妈登录不了 Matrix"
    t -sS -o /dev/null -X POST -H 'content-type: application/json' -d "{\"user_id\": \"$(mxid mom)\", \"kind\": \"person\"}" \
      "$contacts_url"
    # 上一次对着这个栈跑到了配对、没走到进房间时，房间里只有邀请：先找已进的，找不到就收下邀请进房间。
    mom_room=""
    for _ in $(seq 12); do
      mom_room="$(mom_api '/sync?timeout=10000' | python3 -c 'import json, sys
print("\n".join(json.load(sys.stdin).get("rooms", {}).get("join", {})))')"
      [[ -n "$mom_room" ]] && break
    done
    if [[ -z "$mom_room" ]]; then
      mom_room="$(mom_api '/sync?timeout=0' | python3 -c 'import json, sys
print("\n".join(json.load(sys.stdin).get("rooms", {}).get("invite", {})))')"
      [[ -z "$mom_room" ]] || mom_api "/join/$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$mom_room")" \
        -X POST -H 'content-type: application/json' -d '{}' >/dev/null
    fi
    [[ -n "$mom_room" && "$mom_room" != *$'\n'* ]] || fail "妈妈的私聊房间没找到：$mom_room"
  else
  roles="$(t "$core_url/_wonton/api/roles")"
  role="$(python3 -c 'import json, sys
print(next(r["id"] for r in json.load(sys.stdin)["roles"] if r["permissions"]["ai"] == "read_only"))' <<<"$roles")" ||
    fail "没有 AI 只读的角色：$roles"
  expect "主人邀请妈妈，角色的 AI 只读" "$(t -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' \
    -d "{\"display_name\": \"妈妈\", \"username\": \"mom\", \"private_bot\": false, \"role_id\": \"$role\"}" \
    "$core_url/_wonton/api/members")" 201
  code="$(t -X POST -H 'content-type: application/json' -d "{\"direct_addrs\": [\"127.0.0.1:$WONTON_DEV_TUNNEL_PORT\"]}" \
    "$core_url/_wonton/api/members/mom/pairing-code" | json code)"
  docker run --rm -v "$WONTON_DEV_DATA/phonekeys:/k" --entrypoint sh "$phone_image" -c "tunnel-client --key /k/mom.key id" >/dev/null
  expect "妈妈的手机扫码，等管理员同意" "$(mom_pair "$code" | json outcome)" pending
  request="$(t "$core_url/_wonton/api/members/join-requests" | python3 -c 'import json, sys
print(next(r["id"] for r in json.load(sys.stdin)["requests"] if r["username"] == "mom"))')" || fail "主人看不到妈妈的加入请求"
  expect "主人同意" "$(t -o /dev/null -w '%{http_code}' -X POST "$core_url/_wonton/api/members/join-requests/$request/approve")" 204
  expect "妈妈的手机再交一次码，配上了" "$(mom_pair "$code" | json result)" paired
  docker rm -f "$mom_phone" >/dev/null 2>&1 || true
  docker run -d --name "$mom_phone" --network "container:$core" -v "$mom_key:/tmp/phone.key" --entrypoint tunnel-client \
    "$phone_image" --key /tmp/phone.key proxy --auth --domain "$WONTON_DEV_DOMAIN" --server "$server_id" \
    --addr "127.0.0.1:$WONTON_DEV_TUNNEL_PORT" --listen 127.0.0.1:18009 >/dev/null
  for _ in $(seq 60); do
    docker logs "$mom_phone" 2>/dev/null | grep '^本地代理' >/dev/null && break
    sleep 1
  done
  mom_credential="$(docker logs "$mom_phone" 2>/dev/null | sed -n 's/^会话凭据：//p')"
  [[ -n "$mom_credential" ]] || fail "妈妈的手机没有启动：$(docker logs "$mom_phone" 2>&1)"
  expect "妈妈的手机拿到的 Matrix 登录令牌是她的" "$(m -X POST "$core_url/_wonton/matrix/login-token" | json user_id)" "$(mxid mom)"
  # 第一次登录时 Tuwunel 才建她的账号；联系人请求要对方的账号已经在。
  mom_access="$(docker run --rm --network "container:$core" "$echo_image" probe --homeserver "$matrix_url" \
    --proxy "http://$mom_credential@127.0.0.1:18009" --login-token "$(m -X POST "$core_url/_wonton/matrix/login-token" | json token)" \
    --room "$hermes_room" --bot "$hermes_user" --token-only 1 | sed -n 's/^令牌 //p')"
  [[ -n "$mom_access" ]] || fail "妈妈登录不了 Matrix"
  t -sS -o /dev/null -X POST -H 'content-type: application/json' -d "{\"user_id\": \"$(mxid mom)\", \"kind\": \"person\"}" \
    "$contacts_url"
  for _ in $(seq 60); do
    invited="$(mom_api '/sync?timeout=0' | python3 -c 'import json, sys
print("\n".join(json.load(sys.stdin).get("rooms", {}).get("invite", {})))')"
    [[ -n "$invited" ]] && break
    sleep 1
  done
  [[ -n "$invited" && "$invited" != *$'\n'* ]] || fail "妈妈应收到一个 Hermes 的私聊邀请：$invited"
  mom_room="$invited"
  mom_api "/join/$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$mom_room")" \
    -X POST -H 'content-type: application/json' -d '{}' >/dev/null
  fi
  for _ in $(seq 120); do
    state="$(t "$contacts_url" | python3 -c 'import json, sys
print(next((c["state"] for c in json.load(sys.stdin)["contacts"] if c["user_id"] == sys.argv[1]), "absent"))' "$(mxid mom)")"
    [[ "$state" == active ]] && break
    sleep 1
  done
  expect "妈妈进了 Hermes 的私聊，成了它的联系人" "$state" active
  for _ in $(seq 300); do
    [[ ",$(hermes_env MATRIX_ALLOWED_USERS 2>/dev/null)," == *",$(mxid mom),"* &&
      "$(docker inspect -f '{{.State.Health.Status}}' "$hermes" 2>/dev/null)" == healthy ]] && break
    sleep 1
  done
  [[ ",$(hermes_env MATRIX_ALLOWED_USERS)," == *",$(mxid mom),"* ]] ||
    fail "Hermes 的名单里没有妈妈：$(hermes_env MATRIX_ALLOWED_USERS)"
  expect "Hermes 按新名单重建后过了健康检查" "$(docker inspect -f '{{.State.Health.Status}}' "$hermes")" healthy
}

# 分段跑只跑下面某几步时，前面几步铺好的东西没人补，先按需补齐：和 Hermes 的私聊查出来（已经查过就什么都不做）；
# 给宿主 magpie 的转发容器上一轮结束时删了，Hermes 的 provider 还指着那个地址，缺了照原样再起一个。
if [[ -n "${SMOKE_ONLY:-}" ]]; then
seed_smoke_only
# 上一次被中断的轮次没报结束，会话还挂在跑着的链上，再说一句话就会被核心拒（“已登记在别的链上”）：先都停掉。
# 用 owner 令牌问一次就够（假 docker 的用例里拿不到，按没有链处理）。
running="$("$dev" api GET /_wonton/api/agent/chains 2>/dev/null || true)"
for chain in $(python3 -c 'import json, sys
try:
    rows = json.load(sys.stdin)["chains"]
except Exception:
    rows = []
print("\n".join(r["chain"] for r in rows if r["status"] == "running"))' <<<"$running"); do
  "$dev" api POST "/_wonton/api/agent/chains/$chain/stop" >/dev/null ||
    fail "停不掉上一次留下的链 $chain"
done
if ! docker inspect "$model_relay" >/dev/null 2>&1; then
model_port=13425
model_host="$(docker network inspect "$hermes_network" --format '{{(index .IPAM.Config 0).Gateway}}')"
docker run -d --rm --name "$model_relay" --network host "$socat_image" \
  "TCP-LISTEN:$model_port,fork,reuseaddr" "TCP:127.0.0.1:3425" >/dev/null ||
  fail "起不了到宿主 magpie（127.0.0.1:3425）的转发"
fi
fi

if step stop "停链：主人在 App 里停掉 Hermes 正在跑的一轮，这一轮真的停下，链如实显示已停"; then
said="$(mktemp)"
done_file="/tmp/$(marker stop)"
before="$(chain_ids)"
probe_in_hermes_room "$said" --wait 240 --text "用终端工具在前台执行这条命令，跑完再回复我：sleep 90 && touch $done_file" &
asking=$!
chain="$(new_chain "$before" "$(mxid owner)" "$hermes_room")" ||
  fail "主人说话后 App 的链列表里没有这一轮：$(chains)；Hermes 的日志：$(hermes_chain_log)"
ok "主人说话后，App 的链列表里有这一轮，代表主人、从和 Hermes 的私聊开始"
for _ in $(seq 180); do (($(sleeping 90) > 0)) && break; sleep 1; done
(($(sleeping 90) > 0)) || fail "Hermes 没有在终端里跑起 sleep 90：$(cat "$said")"
ok "Hermes 在终端里跑起了要 90 秒的命令"
started="$(date +%s)"
expect "主人停链" "$(stop_chain "$chain")" stopped
for _ in $(seq 30); do (($(sleeping 90) == 0)) && break; sleep 1; done
expect "这一轮在终端里起的命令被停掉" "$(sleeping 90)" 0
[[ " $(chain_ids) " != *" $chain "* ]] || fail "停了的链还显示在跑：$(chains)"
ok "链不再显示在跑"
wait "$asking" || true
# 命令要跑满 90 秒才建文件：从看到它在跑算起过了这么久还没有，才说明它没跑完，不是还没到点。
sleep "$((started + 95 - $(date +%s) > 0 ? started + 95 - $(date +%s) : 0))"
marker_state() { docker exec "$hermes" sh -c "test -e '$done_file' && echo 跑完了 || echo 没跑完"; }
# 探针掉登录会重发一次；停了的链不再挂回去，重发按会话另开一条链、把同一条命令再跑一遍。
# 同一轮里"它有没有再跑命令"看不出确定性，重发过就只记一笔，"文件没建" 由没重发的运行断言。
retried=""
for _ in $(seq 60); do
  # 探针被服务器丢过会重新配对，凭据跟着换；每轮重读，别拿旧的去问。
  credential="$(docker logs "$phone" 2>/dev/null | sed -n 's/^会话凭据：//p' | tail -1)"
  retried="$(comm -13 <(printf '%s\n' "$before" | sort) <(chain_ids 3 | grep -vxF "$chain" | sort) | head -1)"
  # 重发时探针正在重连，隧道可能整段都答不了，链列表看不到；文件建出来同样是重发跑完的信号。
  [[ -z $retried && "$(marker_state)" == 跑完了 ]] && retried=跑完了
  [[ -n $retried ]] && break
  sleep 2
done
if [[ -n $retried ]]; then
  ok "重发那一轮又跑了一遍同一条命令（$(marker_state)），文件没建与否不论"
else
  expect "命令没有跑完" "$(marker_state)" 没跑完
fi
talk_to_hermes "$said" "用一句话回答：1 加 1 等于几？" || fail "停链以后 Hermes 不回话了：$(cat "$said")"
grep '^第' "$said" | sed 's/^/   · /'
ok "停链以后主人再说一句，Hermes 照常回答"
rm -f "$said"

fi
# 单独重跑这一步要在新栈上、或离开上一次超过 30 分钟：放行时段还在时，第一条（本来要拒绝的）会直接放行、
# 不出卡片，pending_card 会白等；speaker 开头也按“没有放行时段”断言不发卡片。
if step outbox "Bot 发私聊（wonton__send_message）：主人批准后经出箱发到家人手里，拒绝的和停了链的不发"; then
family_join
asked="$(mktemp)"
mom_heard="$(mktemp)"
sent_ok="$(marker approved)"
sent_rejected="$(marker rejected)"
sent_stopped="$(marker stopped)"
before="$(chain_ids)"
# 妈妈的手机一直开着等这条私聊：设备要在 Hermes 发之前就在，Hermes 才会发到它。她先说的一句也开了她自己的一轮。
probe_as_mom "$mom_heard" --wait 1500 --match "$sent_ok" --text "你好" &
mom_listening=$!
mom_chain="$(new_chain "$before" "$(mxid mom)" "$mom_room")" ||
  fail "妈妈说话后 App 的链列表里没有她的一轮：$(chains)；Hermes 的日志：$(hermes_chain_log)"
ok "妈妈和 Hermes 说话，链列表里这一轮代表妈妈、从她和 Hermes 的私聊开始"

probe_in_hermes_room "$asked" --wait 300 --settle 60 --text "$(send_text "$sent_rejected")" &
asking=$!
card_id="$(pending_card)" || fail "Hermes 发私聊没有发确认卡片：$(cat "$asked")"
expect "主人拒绝" "$(answer_with reject "$card_id")" 200
for _ in $(seq 30); do [[ "$(card_state "$card_id")" == rejected ]] && break; sleep 1; done
expect "卡片显示拒绝了" "$(card_state "$card_id")" rejected
wait "$asking" || true

before="$(chain_ids)"
probe_in_hermes_room "$asked" --wait 300 --settle 60 --text "$(send_text "$sent_stopped")" &
asking=$!
card_id="$(pending_card)" || fail "Hermes 发私聊没有发确认卡片：$(cat "$asked")"
chain="$(new_chain "$before" "$(mxid owner)" "$hermes_room")" || fail "链列表里没有这一轮：$(chains)"
expect "卡片等着时主人停掉这一轮" "$(stop_chain "$chain")" stopped
for _ in $(seq 30); do [[ "$(card_state "$card_id")" != pending ]] && break; sleep 1; done
[[ "$(card_state "$card_id")" != pending ]] || fail "停链以后卡片还在等主人回答（停的链 $chain，卡片 $card_id，链列表：$(chains)；发消息的审计：$(docker exec "$core" sh -c "grep '\"send_message\"' '$WONTON_DEV_DATA/core/audit.jsonl' | tail -3")）"
ok "停链以后卡片撤回了（$(latest_card)）"
[[ "$(answer_with approve "$card_id")" != 200 ]] || fail "撤回的卡片还能批准"
ok "撤回的卡片批准不了"
wait "$asking" || true

probe_in_hermes_room "$asked" --wait 300 --settle 60 --text "$(send_text "$sent_ok")" &
asking=$!
card_id="$(pending_card)" || fail "Hermes 发私聊没有发确认卡片：$(cat "$asked")"
expect "主人批准，并在一段时间内放行发给妈妈的私聊" "$(answer_with allow_window "$card_id")" 200
wait "$mom_listening" ||
  fail "妈妈没收到批准的私聊：$(cat "$mom_heard")；审计：$(sent_audit "$sent_ok")；Hermes 的日志：$(hermes_chain_log)"
grep -q "收到明文的回复：.*$sent_ok" "$mom_heard" || fail "妈妈收到的不是批准的那句：$(cat "$mom_heard")"
ok "妈妈的手机收到了 Hermes 发来的私聊，正文是主人批准的那句"
wait "$asking" || true
# 出箱按先后发：批准的那条到了，前面拒绝的、撤回的要发也早发了。
expect "审计：批准的那条发出了一次" "$(sent_audit "$sent_ok" | grep -c '^allowed 已发出')" 1
expect "审计：拒绝的那条记为拒绝" "$(sent_audit "$sent_rejected" | grep -c '^rejected')" 1
expect "审计：拒绝的那条没发" "$(sent_audit "$sent_rejected" | grep -c '已发出' || true)" 0
expect "审计：停了链的那条没发" "$(sent_audit "$sent_stopped" | grep -c '已发出' || true)" 0
rm -f "$mom_heard"

fi
if step speaker "说话人：核心按请求头认出这一轮是谁说的，家人说的按家人的权限算；没带请求头时退回按在跑的轮次认"; then
mom_said="$(mktemp)"
mom_heard="$(mktemp)"
sent_window="$(marker window)"
sent_by_mom="$(marker by-mom)"
# 妈妈这一轮一直在跑（终端里 sleep 120），这时主人让 Hermes 发私聊：核心要是没认出说话人、按在跑的轮次里最严的算，
# 就是妈妈，放行时段不放宽、又发卡片。
probe_as_mom "$mom_heard" --wait 600 --match "$sent_window" --text "用终端工具在前台执行 sleep 120，跑完告诉我。" &
mom_listening=$!
for _ in $(seq 180); do (($(sleeping 120) > 0)) && break; sleep 1; done
(($(sleeping 120) > 0)) || fail "妈妈让 Hermes 跑的命令没有跑起来"
last_cards="$(pending_cards)"
asked="$(mktemp)"
probe_in_hermes_room "$asked" --wait 300 --settle 60 --text "$(send_text "$sent_window")" ||
  fail "Hermes 没回答：$(cat "$asked")"
expect "主人说的、收件人在放行时段里：不发卡片" "$(pending_cards)" "$last_cards"
wait "$mom_listening" || fail "妈妈没收到放行时段里的私聊：$(cat "$mom_heard")；审计：$(sent_audit "$sent_window")"
ok "妈妈收到了主人让发的私聊，没有再问主人"
expect "审计：这条按放行时段直接发出" "$(sent_audit "$sent_window" | grep -c '^allowed 已发出')" 1

for _ in $(seq 150); do (($(sleeping 120) == 0)) && break; sleep 1; done
probe_as_mom "$mom_said" --wait 300 --settle 60 --text "$(send_text "$sent_by_mom")" &
asking=$!
card_id="$(pending_card)" || fail "妈妈让 Hermes 发私聊，收件人在主人的放行时段里，也要发卡片：$(cat "$mom_said")"
ok "妈妈说的，放行时段不替她放宽，卡片照发"
expect "主人拒绝" "$(answer_with reject "$card_id")" 200
wait "$asking" || true
expect "审计：妈妈让发的那条没发" "$(sent_audit "$sent_by_mom" | grep -c '已发出' || true)" 0

# 直接以 Hermes 的身份问总线（不经模型），看核心按谁的权限给工具：只读封顶的人拿不到 wonton__save_file。
bus() { docker exec -i "$hermes" /opt/hermes/.venv/bin/python - "$@" <"$here/smoke-bus-as-hermes.py"; }
expect "说话人头是主人：按主人的权限" "$(bus tools "$(mxid owner)" -)" yes
expect "说话人头是妈妈：按她的角色，只读" "$(bus tools "$(mxid mom)" -)" no
expect "说话人头不是 Hermes 听的人：按最严" "$(bus tools '@stranger:elsewhere.example' -)" no
expect "没带说话人头、也没有在跑的轮次：按 Bot 的归属人" "$(bus tools - -)" yes
session="$(marker session)"
fake_chain="$(bus start "$session" "$(mxid mom)" "$mom_room")" || fail "以 Hermes 的身份报不了妈妈的一轮"
[[ "$(chains)" == *"$fake_chain $(mxid mom) $mom_room running"* ]] || fail "报的这一轮在链列表里不对：$(chains)"
expect "没带说话人头、带这一轮的会话：退回按这一轮认出妈妈" "$(bus tools - "$session")" no
expect "什么都没带：退回按在跑的轮次里最严的，妈妈" "$(bus tools - -)" no
expect "说话人头优先于在跑的轮次" "$(bus tools "$(mxid owner)" "$session")" yes
bus end "$session"
expect "这一轮结束后什么都没带：又按归属人" "$(bus tools - -)" yes
rm -f "$mom_said" "$mom_heard" "$asked"

fi
