#!/usr/bin/env bash
# 通话冒烟：Matrix 登录/API、房间与成员、配对（矩阵侧步骤）；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2154 # 这些变量由别的库赋值，单文件分析看不见。

# matrix.<base> 只认经隧道进来的已配对设备，宿主机上的 owner 令牌进不去；
# 冒烟直接拿核心的 Ed25519 私钥在 <主机> 上签一次性 JWT 登 Tuwunel，再直接打 Tuwunel（容器地址当场查，别写死）。
matrix_env() {
    local cache="$EVIDENCE/matrix-env" url
    # 容器重建后 IP 会变，缓存里的地址可能被别的服务接手（404 但仍连得上）：只有 200 才复用，其它一律重登。
    if [[ -s $cache ]]; then
        url=$(cut -d' ' -f1 <"$cache")
        if [[ $(timeout 25 ssh "$HOST" "curl -s -m 5 -o /dev/null -w '%{http_code}' $url/_matrix/client/versions" || true) != 200 ]]; then
            rm -f "$cache"
        fi
    fi
    if [[ ! -s $cache ]]; then
        timeout 60 ssh "$HOST" python3 - "$STACK" <<'PY' >"$cache"
import json, subprocess, sys, time, urllib.request
import jwt
stack = sys.argv[1]
def inspect(name, fmt):
    return subprocess.check_output(["docker", "inspect", name, "--format", fmt], text=True).strip()
key = next(v for v in json.loads(inspect(f"wonton-{stack}", "{{json .Config.Env}}"))
           if v.startswith("WONTON_MATRIX_JWT_KEY=")).split("=", 1)[1]
# compose v2 的容器名带 -1 后缀，按前缀找，不写死全名。
names = subprocess.check_output(["docker", "ps", "--format", "{{.Names}}"], text=True).split()
tuwunel = next(n for n in names if n.startswith(f"wonton-{stack}-tuwunel"))
ip = inspect(tuwunel, "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}")
claims = {"sub": "owner", "iss": "wonton-core", "aud": "tuwunel",
          "iat": int(time.time()), "exp": int(time.time()) + 3600}
login = json.dumps({"type": "org.matrix.login.jwt",
                    "token": jwt.encode(claims, key, algorithm="EdDSA")}).encode()
req = urllib.request.Request(
    f"http://{ip}:8008/_matrix/client/v3/login", data=login,
    headers={"Content-Type": "application/json"})
print(f"http://{ip}:8008", json.load(urllib.request.urlopen(req))["access_token"])
PY
        chmod 600 "$cache"
    fi
    cat "$cache"
}
api() {
    local method=$1 path=$2 body=${3:-} url tok
    read -r url tok < <(matrix_env)
    timeout 25 ssh "$HOST" "curl -sS --fail-with-body -X $method \
        -H 'Authorization: Bearer $tok' -H 'Content-Type: application/json' \
        '$url$path'${body:+ --data-binary @-}" <<<"${body-}"
}

pair() { # pair <a|b> <dev>
    local dev=$2 log code pid
    if ui "$dev" 2>/dev/null | grep -q nexuslauncher; then
        timeout 25 ssh "$HOST" "adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null
        sleep 4
    fi
    # 每轮都从 pm clear 后的空数据起（见准备段），所以无条件配对；不再用页面文本猜“已配对”，
    # 全新安装先出 Loading/相机授权页时猜不准。
    # docker logs --since 把不带时区的时间按本机时区解释，这里必须带 +08:00 这样的偏移，否则会把几小时前的
    # 旧配对日志也圈进来，早退判据永远为真。
    since=$(date +%Y-%m-%dT%H:%M:%S%:z)
    log="$EVIDENCE/pair-$1.raw"
    : > "$log"
    # wonton-core pair 一直挂着等设备连上，不能在命令替换里等：后台起，轮询日志取码。
    dev_sh pair "$pair_addr:$tunnel" > "$log" 2>&1 &
    pid=$!
    for _ in $(seq 1 90); do
        code=$(grep -o 'wonton://pair[^[:space:]]*' "$log" 2>/dev/null | head -1 || true)
        [[ -n $code ]] && break
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
    done
    [[ -n ${code:-} ]] || { cat "$log" >> "$EVIDENCE/pair.log"; kill "$pid" 2>/dev/null; die "没取到配对码（$1）"; }
    # 配对码是一次性的，落盘只留连通用的 URL。
    printf '%s pair 码已取到\n' "$(date +%T)" >> "$EVIDENCE/pair.log"
    # 先 force-stop：设备上残留的旧配对请求会和这次的码撞成 “Both were cancelled”。
    timeout 25 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; \
        adb -s $dev shell pm grant app.wonton.android android.permission.CAMERA; \
        adb -s $dev shell pm grant app.wonton.android android.permission.RECORD_AUDIO; \
        adb -s $dev shell pm grant app.wonton.android android.permission.POST_NOTIFICATIONS; \
        adb -s $dev shell am force-stop app.wonton.android; \
        adb -s $dev shell am start -n app.wonton.android/.MainActivity \
          --es wonton_log 'wonton_client=debug,wonton_tunnel=debug,iroh=info,warn' \
          --es wonton_pairing_code '\"$code\"'" >/dev/null
    # 输码后的弹窗按出现顺序处理：“没有另一台设备”→验证失败“重试”→配好后的备份恢复页。
    # 首次“跳过”只点一次，不 break：跳过引导后还会出现配对确认弹窗。
    local skipped=0
    for _ in $(seq 1 120); do
        # 页面可能一直停在 Loading；核心日志出现「设备扫码配对」就是成功了，不必等页面文字。
        if timeout 30 ssh "$HOST" "docker logs --since '$since' wonton-$STACK 2>&1 | grep -q '设备扫码配对'"; then
            kill "$pid" 2>/dev/null || true
            timeout 25 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; \
                adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null || true
            sleep 4
            return 0
        fi
        xml=$(ui "$dev" 2>/dev/null || true)
        if grep -q 'Saved it somewhere safe' <<<"$xml" || grep -q 'text="Saved"' <<<"$xml"; then
            tap_name "$dev" 'Saved' || true; sleep 2; continue
        fi
        if grep -qE 'text="Reset backup"|text="重置备份"' <<<"$xml"; then timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK"; sleep 2; continue; fi
        if grep -q 'Continue on your other device' <<<"$xml"; then
            # 验证超时页只在已配对设备上出现：这轮码没被消费，直接当配对成功返回。
            kill "$pid" 2>/dev/null || true; return 0
        fi
        if grep -q 'When you switch to a new phone' <<<"$xml"; then
            timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK; sleep 1; adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null
            sleep 2; continue
        fi
        if grep -qE 'Verify with another device|Enter (your )?recovery key|This device.c?an.t see past messages yet|text="Chat backup"' <<<"$xml"; then
            # 配对完成后的备份/恢复码/跨设备验证页：退回聊天列表。
            to_list "$dev"; continue
        fi
        if grep -qE 'text="(聊天|Chat)"|text="call-smoke-|text="Element Call \||Waiting for you|content-desc="In call"' <<<"$xml"; then
            # 配好了但停在备份页（底部已经能看见标签）时退回列表。
            grep -q 'Save your recovery key\|Chat backup is on\|can.t see past messages' <<<"$xml" && { timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK"; sleep 2; continue; }
            break
        fi
        if grep -q 'text="Start verification"' <<<"$xml"; then to_list "$dev"; sleep 1; continue; fi
        if grep -q "I don't have my other device" <<<"$xml"; then tap_name "$dev" "I don't have my other device"; sleep 2; continue; fi
        if grep -q '没有另一台设备' <<<"$xml"; then tap_name "$dev" '我没有另一台设备'; sleep 2; continue; fi
        if grep -q 'text="Try again"\|text="重试"' <<<"$xml"; then tap_name "$dev" 'Try again' 2>/dev/null || tap_name "$dev" '重试'; sleep 1; continue; fi
        if [[ $skipped == 0 ]] && grep -q 'text="跳过"\|text="Skip"' <<<"$xml"; then
            tap_name "$dev" '跳过' 2>/dev/null || tap_name "$dev" 'Skip'; skipped=1; sleep 1; continue
        fi
        sleep 1
    done
    # 核心的配对日志出现「设备扫码配对」才算成功；pair 进程本身会等到码过期才退。
    for _ in $(seq 1 90); do
        if timeout 30 ssh "$HOST" "docker logs --since '$since' wonton-$STACK 2>&1 | grep -q '设备扫码配对'"; then
            kill "$pid" 2>/dev/null || true
            # 配对后的落地页各机型不同（备份、恢复码、跨设备验证）。冷启动重进，
            # 直接回聊天列表，不让后续步骤去猜当前停在哪一页。
            timeout 25 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; \
                adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null || true
            sleep 4
            return 0
        fi
        xml=$(ui "$dev" 2>/dev/null || true)
        if grep -q "I don't have my other device" <<<"$xml"; then tap_name "$dev" "I don't have my other device" || true; fi
        if grep -q 'text="Try again"\|text="重试"' <<<"$xml"; then tap_name "$dev" 'Try again' 2>/dev/null || tap_name "$dev" '重试' || true; fi
        sleep 2
    done
    kill "$pid" 2>/dev/null || true
    die "设备 $dev 没在 180 秒内配对成功（见 $log）"
}

room_id=""

create_room() {
    local body algo
    room_name="call-smoke-$(date +%H%M%S)"
    if [[ ${stage:-} == plain ]]; then
        algo=plain
        body=$(printf '{"name":"%s","preset":"private_chat"}' "$room_name")
    else
        algo=megolm
        body=$(printf '{"name":"%s","preset":"private_chat","initial_state":[{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]}' "$room_name")
    fi
    room_id=$(api POST /_matrix/client/v3/createRoom "$body" | tee -a "$EVIDENCE/room.log" | grep -o '"room_id":"[^"]*"' | cut -d'"' -f4 || true)
    [[ -n $room_id ]] || die "建房间失败"
    # 空房间被 App 列表隐藏（16:30 实测：发一条消息后立刻出现）；种子消息让两端都能看到。
    api PUT "/_matrix/client/v3/rooms/$room_id/send/m.room.message/seed-$$-$(date +%N | tail -c 6)" '{"msgtype":"m.text","body":"seed"}' >/dev/null 2>&1 || log "种子消息失败，房间可能不出现在列表"
    log "房间 $room_id（$algo）"
    # 新房间进 App 列表的延迟实测 9~60 分钟（服务端→App 同步）：建房后立刻冷启动并等到行出现，
    # 别让 open_room 的窗口去赌。复用房间（ROOM_ID）时跳过。
    if [[ -z ${ROOM_ID:-} ]]; then
        wait_room_visible
    fi
}

wait_room_visible() { # 等 room_name 的行在两端列表出现（预算 40 分钟；每 30 秒重启重渲染 + 掉回配对页就重配）
    local d xml ok
    for i in $(seq 1 20); do
        for d in "$DEV_A" "$DEV_B"; do
            timeout 25 ssh "$HOST" "adb -s $d shell am force-stop app.wonton.android; adb -s $d shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
        done
        sleep 8
        # 配对后常停在「另一台设备还没应答」的跨设备验证页，列表被盖住；to_list 会跳过它回列表。
        to_list "$DEV_A" || true
        to_list "$DEV_B" || true
        # 配对后 App 偶尔会被重启抢成「Paste the pairing code」的空配对页（核心日志已配对、App 没落盘）：
        # 直接就地重配一次，别等 40 分钟。
        for d in "$DEV_A" "$DEV_B"; do
            if grep -q 'text="Paste the pairing code"' <<<"$(ui "$d" 2>/dev/null || true)"; then
                pair "$( [[ $d == "$DEV_B" ]] && echo b || echo a )" "$d" >/dev/null 2>&1 || true
            fi
        done
        ok=1
        for d in "$DEV_A" "$DEV_B"; do
            xml=$(ui "$d" 2>/dev/null || true)
            grep -q "text=\"[^\"]*$room_name" <<<"$xml" || ok=0
        done
        if [[ $ok == 1 ]]; then log "房间行两端可见（第 $i 轮）"; return 0; fi
        sleep 30
    done
    log "房间 40 分钟未在两端列表出现，继续尝试（open_room 里还会等）"
}



# 客户端上报的是不稳定前缀 org.matrix.msc3401.call.member，state_key 是成员 user id（可带设备后缀）。
members() { # 房间全量 state 里挑 call.member：一个 state_key 算一个会话（content 为空也算，实测 22:2x 的两条就是空 content）。
    api GET "/_matrix/client/v3/rooms/$1/state" 2>/dev/null | python3 -c '
import json, sys
try:
    events = json.load(sys.stdin)
except Exception:
    raise SystemExit
for e in events:
    if e.get("type") in ("org.matrix.msc3401.call.member", "m.call.member") and e.get("state_key"):
        print(e["state_key"])
' | sort -u
}

active_members() { # 还在通话里的：content 非空的 call.member（Element Call 挂断后写空 content，不删键）
    api GET "/_matrix/client/v3/rooms/$1/state" 2>/dev/null | python3 -c '
import json, sys
try:
    events = json.load(sys.stdin)
except Exception:
    raise SystemExit
for e in events:
    if e.get("type") in ("org.matrix.msc3401.call.member", "m.call.member") and e.get("content") and e.get("state_key"):
        print(e["state_key"])
' | sort -u
}

wait_new_members() { # <房间> <本轮开始前的 keys> <本轮 keys 文件> <期望条数> <秒>：只认本轮新增
    local until after
    until=$((SECONDS + $5))
    while (( SECONDS < until )); do
        deadline_hit && { log "wait_new_members 组级超时"; return 1; }
        after=$(active_members "$1" || true)
        comm -13 "$2" <(printf '%s\n' "$after" | sort -u) | grep '@' > "$3" || true
        if [[ $(wc -l <"$3") -ge $4 ]]; then
            tee -a "$EVIDENCE/members.log" <"$3"
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_left() { # <房间> <keys 文件> <秒>：这些键都不再活跃（content 清空或键没了）
    local i active
    for i in $(seq 1 "$3"); do
        active=$(active_members "$1" || true)
        if ! comm -12 "$2" <(printf '%s\n' "$active" | sort -u) | grep -q '@'; then return 0; fi
        sleep 1
    done
    comm -12 "$2" <(printf '%s\n' "$active" | sort -u) | tee -a "$EVIDENCE/members.log"
    return 1
}

