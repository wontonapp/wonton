#!/bin/bash
# 通话端到端冒烟（ADR-0096）。两台设备装 Debug App、同一加密房间，走三条媒体路径
# （UDP 直连、只剩插件 TCP 7881、只剩 n0 中继），每组核对双方 m.call.member 与
# 相机/麦克风权限、留屏和 WebRTC 统计，最后走挂断与强杀后的 delayed leave。
#
# 前提：栈已起好、核心和插件在跑、两台设备已装 App 且能连到栈。脚本跑在能执行
# `ssh <主机>` 的机器上，所有 adb 都在 <主机> 上执行（共用 adb server，不 kill-server）。
#
# 用法：
#   STACK=<远程栈名> HOST=<起栈的主机> \
#     DEV_A=<设备 A 的 adb 串号> DEV_B=<设备 B 的 adb 串号> \
#     EVIDENCE=<证据目录> deploy/dev/calls/smoke.sh
#
# 只跑某一步：STAGE=udp|tcp|relay|kill（默认 all：udp→kill→tcp→relay）。默认每轮新建加密房间，
# ROOM_ID=<房间> ROOM_NAME=<名字> 可复用已有房间（旧房间会积着上轮成员，判定要看本轮新增的两条）。
# 插件包：PKG（worktree 内的相对路径，默认 deploy/dev/calls/plugin，由调用方放好）。
set -euo pipefail
shopt -s inherit_errexit

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${STACK:?}" "${HOST:?}" "${DEV_A:?}" "${DEV_B:?}" "${EVIDENCE:?}"
stage="${STAGE:-all}"
pkg="${PKG:-deploy/dev/calls/plugin}"
mkdir -p "$EVIDENCE"
chmod 700 "$EVIDENCE"

exec 9>"$EVIDENCE/.lock"
flock -n 9 || { echo "另一份冒烟在跑（$EVIDENCE）" >&2; exit 1; }

# cdp.py / premain.js 在 <主机> 上跑：随栈副本推过去（remote-dev 维护 stacks/dev-<栈>）。
RCD="\$HOME/wonton-dev/stacks/dev-$STACK/deploy/dev/calls"
timeout 25 ssh "$HOST" "true; mkdir -p $RCD"
for f in cdp.py premain.js; do timeout 25 ssh "$HOST" "cat > $RCD/$f" < "$here/$f"; done

log() { printf '%s %s\n' "$(date +%T)" "$*" | tee -a "$EVIDENCE/run.log" >&2; }
die() { log "失败：$*"; exit 1; }

adb_a() { timeout 25 ssh "$HOST" "adb -s $DEV_A $*"; }
adb_b() { timeout 25 ssh "$HOST" "adb -s $DEV_B $*"; }
dev_sh() { "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" "$@"; }
stack() { timeout 60 ssh "$HOST-stack" "$1" "$STACK" "${@:2}"; }
# matrix.<base> 只认经隧道进来的已配对设备，宿主机上的 owner 令牌进不去；
# 冒烟直接拿核心的 Ed25519 私钥在 <主机> 上签一次性 JWT 登 Tuwunel，再直接打 Tuwunel（容器地址当场查，别写死）。
matrix_env() {
    local cache="$EVIDENCE/matrix-env" url
    # 容器重建后 IP 会变：先用缓存的 URL 探一下，通不了就重新登录换地址。
    if [[ -s $cache ]]; then
        url=$(cut -d' ' -f1 <"$cache")
        if [[ $(timeout 25 ssh "$HOST" "curl -s -m 5 -o /dev/null -w '%{http_code}' $url/_matrix/client/versions" || true) == 000 ]]; then
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

# dump 常在冷启动/转场后返回空壳：循环到拿到超过 3000 字节的层级再回，拿到就够。
ui() { timeout 90 ssh "$HOST" "for i in 1 2 3 4 5 6 7 8; do adb -s $1 shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && sleep 0.3 && n=\$(adb -s $1 shell \"wc -c < /sdcard/ui.xml\" | tr -d \"\\r\") && [ \"\$n\" -gt 3000 ] 2>/dev/null && adb -s $1 shell cat /sdcard/ui.xml && exit 0; sleep 5; done; exit 1"; }
# App 是否在前台。只看窗口焦点，不信 dump 文本：部分系统界面（配对页的输入框焦点、
# 桌面）在 dump 里没有可认的字，靠文本判断会把它们当成 App 页面继续空转。
app_fg() { timeout 25 ssh "$HOST" "adb -s $1 shell dumpsys window 2>/dev/null | grep -q 'mCurrentFocus=.*app.wonton.android'" >/dev/null 2>&1; }
# 把页面带回聊天列表：焦点不在 App 就冷启动，在就 BACK 退出设置/备份这类页面。
to_list() { # to_list <dev>：一路清掉验证/备份页直到聊天列表出现
    local dev=$1 xml
    for _ in 1 2 3 4 5 6 7 8; do
        xml=$(ui "$dev" 2>/dev/null || true)
        if grep -qE 'content-desc="Sidebar"|text="call-smoke-|text="Waiting for you"' <<<"$xml"; then return 0; fi
        # 验证超时页：只有跳过一条路，BACK 会退出 App。
        if grep -q "Continue on your other device" <<<"$xml"; then
            tap_text "$dev" "I don't have my other device" || true; sleep 2; continue
        fi
        # 抽屉盖在列表上：点 Chats 直接到聊天标签。
        if grep -q 'content-desc="Collapse sidebar"' <<<"$xml"; then tap_name "$dev" 'Chats' || true; sleep 2; continue; fi
        # 备份页与确认框：确认框点 Cancel，页面本体 BACK（BACK 不会退出 App 的页面才用 BACK）。
        if grep -q 'text="Reset chat backup?"' <<<"$xml"; then
            tap_text "$dev" "Cancel" || true; sleep 1
        fi
        if grep -qE 'Verify with another device|Enter (your )?recovery key|can.t see past messages|text="Chat backup"|Reset backup' <<<"$xml"; then
            if app_fg "$dev"; then timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK" >/dev/null 2>&1 || true; fi
            sleep 2; continue
        fi
        # 都不是：不在前台就冷启动，在前台就 BACK 退一层。
        if app_fg "$dev"; then
            timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK" >/dev/null 2>&1 || true
        else
            timeout 25 ssh "$HOST" "adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
        fi
        sleep 2
    done
}
# 点一个无障碍可见的名字：先等它出现（最多 90 秒），再按它的 bounds 中心点。
tap_name() {
    local dev=$1 text=$2 xml coords x y
    for _ in $(seq 1 10); do
        xml=$(ui "$dev" 2>/dev/null || true)
        # 取最后一个匹配：弹窗节点排在底页同文案之后。
        line=$(printf '%s' "$xml" | grep -oE "text=\"$text\"[^>]*bounds=\"[^\"]+\"" | tail -1) || true
        bounds=${line##*bounds=\"}
        coords=""
        [[ -n $bounds ]] && coords=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${bounds%\"}")
        # 取包围盒两个角的中点点击。
        if [[ $coords =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
            read -r x1 y1 x2 y2 <<<"$coords"
            coords="$(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
        else
            coords=""
        fi
        [[ -n $coords ]] && break
        sleep 1
    done
    [[ -n ${coords:-} ]] || { echo "设备 $dev 上看不到「$text」" >&2; return 1; }
    read -r x y <<<"$coords"
    timeout 25 ssh "$HOST" "adb -s $dev shell input tap $x $y"
    # 第二下只在目标还在时补：双击会把刚弹出的确认框点到背景上关掉。
    sleep 1
    ui "$dev" 2>/dev/null | grep -q "text=\"$text\"" && timeout 25 ssh "$HOST" "adb -s $dev shell input tap $x $y"
    true
}
shot() { timeout 25 ssh "$HOST" "adb -s $1 exec-out screencap -p" > "$EVIDENCE/$2.png"; }
rec() { timeout 25 ssh "$HOST" "adb -s $1 shell screenrecord --bit-rate 4000000 --time-limit 300 /sdcard/$2.mp4" & }
fetch_rec() { # 视频经 exec-out 回本地（adb pull 会把文件写到 HOST 上，不是本机）
    timeout 25 ssh "$HOST" "adb -s $1 shell pkill -INT screenrecord" >/dev/null 2>&1 || true
    sleep 2
    timeout 25 ssh "$HOST" "adb -s $1 exec-out cat /sdcard/$2.mp4" > "$EVIDENCE/$2.mp4"
    timeout 25 ssh "$HOST" "adb -s $1 shell rm -f /sdcard/$2.mp4" >/dev/null 2>&1 || true
    [[ -s $EVIDENCE/$2.mp4 ]] || log "录屏为空：$2"
}
pair() { # pair <a|b> <dev>
    local dev=$2 log code pid
    if ui "$dev" 2>/dev/null | grep -q nexuslauncher; then
        timeout 25 ssh "$HOST" "adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null
        sleep 4
    fi
    local pre; pre=$(ui "$dev" 2>/dev/null || true)
    # 反转判据：只在明确看到配对入口时才走配对；其它页面（引导页、列表、抽屉…）都当已配对，
    # 由 to_list 负责清到列表。否则弹窗循环会在已配对设备上搅几十分钟。
    if grep -q 'Paste the pairing code' <<<"$pre" || grep -q 'Not connected' <<<"$pre"; then
        :
    else
        log "设备 $1 已配对，跳过"; return 0
    fi
    since=
    since=$(date -u +%Y-%m-%dT%H:%M:%S)
    log="$EVIDENCE/pair-$1.raw"
    : > "$log"
    # wonton-core pair 一直挂着等设备连上，不能在命令替换里等：后台起，轮询日志取码。
    dev_sh pair "10.0.2.2:$tunnel" > "$log" 2>&1 &
    pid=$!
    for _ in $(seq 1 90); do
        code=$(grep -o 'wonton://pair[^[:space:]]*' "$log" 2>/dev/null | head -1 || true)
        [[ -n $code ]] && break
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
    done
    [[ -n ${code:-} ]] || { cat "$log" >> "$EVIDENCE/pair.log"; kill "$pid" 2>/dev/null; die "没取到配对码（$1）"; }
    # 设备经 adb reverse 只能连 127.0.0.1，码里的 10.0.2.2 在 194 上到不了。
    code=${code//10.0.2.2/192.168.0.194}
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
tap_text() { # 按精确文本点中心，不做祖先回退
    local dev=$1 text=$2 xml c b x1 y1 x2 y2
    xml=$(ui "$dev" 2>/dev/null || true)
    c=$(grep -oE "text=\"$text\"[^>]*bounds=\"[^\"]+\"" <<<"$xml" | tail -1) || true
    [[ -n $c ]] || return 1
    b=${c##*bounds=\"}
    c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
    read -r x1 y1 x2 y2 <<<"$c"
    timeout 25 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
}

clear_dialogs() { # 设备验证、备份提示这类会话级弹层
    local d xml
    for d in "$DEV_A" "$DEV_B"; do
        for _ in 1 2 3 4 5 6 7 8; do
            xml=$(ui "$d" 2>/dev/null || true)
            if grep -q 'text="Reset chat backup?"' <<<"$xml"; then tap_text "$d" "Cancel" || true; sleep 1; timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true; sleep 2; continue; fi
            # 插件媒体权限弹窗：答一次 Allow，核心记下决定，后续 join 不再询问。
            if grep -qE 'text="(Allow|允许)"' <<<"$xml" && grep -qiE 'camera|microphone|摄像头|麦克风' <<<"$xml"; then
                tap_text "$d" "Allow" 2>/dev/null || tap_text "$d" "允许" 2>/dev/null || true; sleep 2; continue
            fi
            if grep -q 'text="Start verification"' <<<"$xml"; then
                timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true
            elif grep -q "I don't have my other device" <<<"$xml"; then
                tap_text "$d" "I don't have my other device" >/dev/null 2>&1 || true
            elif grep -q 'Enter your recovery key' <<<"$xml"; then
                timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK"
            elif grep -q 'text="Later"' <<<"$xml"; then
                tap_text "$d" "Later" >/dev/null 2>&1 || true
            elif grep -qE 'Reset backup|can.t see past messages' <<<"$xml"; then
                # 备份页本体（含 Verify/Enter recovery key/Reset backup）与"看不到历史"页：
                # BACK 退回列表；点 Reset backup 只会弹确认框把流程拖死。
                timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true
            else
                break
            fi
            sleep 3
        done
    done
}

create_room() {
    local body
    room_name="call-smoke-$(date +%H%M%S)"
    body=$(printf '{"name":"%s","preset":"private_chat","initial_state":[{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]}' "$room_name")
    room_id=$(api POST /_matrix/client/v3/createRoom "$body" | tee -a "$EVIDENCE/room.log" | grep -o '"room_id":"[^"]*"' | cut -d'"' -f4 || true)
    [[ -n $room_id ]] || die "建加密房间失败"
    # 空房间被 App 列表隐藏（16:30 实测：发一条消息后立刻出现）；种子消息让两端都能看到。
    api PUT "/_matrix/client/v3/rooms/$room_id/send/m.room.message/seed-$$-$(date +%N | tail -c 6)" '{"msgtype":"m.text","body":"seed"}' >/dev/null 2>&1 || log "种子消息失败，房间可能不出现在列表"
    log "房间 $room_id"
    # 新房间进 App 列表的延迟实测 9~60 分钟（服务端→App 同步）：建房后立刻冷启动并等到行出现，
    # 别让 open_room 的窗口去赌。复用房间（ROOM_ID）时跳过。
    if [[ -z ${ROOM_ID:-} ]]; then
        wait_room_visible
    fi
}

wait_room_visible() { # 等 room_name 的行在两端列表出现（预算 40 分钟，每 2 分钟重启重渲染）
    local d xml ok
    for i in $(seq 1 20); do
        if (( i % 2 == 1 )); then
            for d in "$DEV_A" "$DEV_B"; do
                timeout 25 ssh "$HOST" "adb -s $d shell am force-stop app.wonton.android; adb -s $d shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
            done
        fi
        sleep 115
        ok=1
        for d in "$DEV_A" "$DEV_B"; do
            xml=$(ui "$d" 2>/dev/null || true)
            grep -q "text=\"[^\"]*$room_name" <<<"$xml" || ok=0
        done
        if [[ $ok == 1 ]]; then log "房间行两端可见（等了 $((i*2)) 分钟内）"; return 0; fi
    done
    log "房间 40 分钟未在两端列表出现，继续尝试（open_room 里还会等）"
}


open_room() { # open_room <dev> <显示名片段>
    # 列表行按房间名找；房间名带时间戳，用前缀匹配。
    local dev=$1 name=$2 xml row port=9331 b cs x1 y1 x2 y2
    [[ $dev == "$DEV_B" ]] && port=9334
    for i in $(seq 1 240); do
        # 列表 UI 不实时刷新：新建房间的行要等 App 重启才渲染出来（17:19 实测）。
        # 周期性重启一次，重渲染列表；房间行出现就往下走。
        if (( i % 60 == 0 )); then
            timeout 25 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
            sleep 12
        fi
        xml=$(ui "$dev" 2>/dev/null || true)
        # 焦点掉到桌面/系统 UI 时 dump 里常常认不出字，不能只靠页面文本判断。
        if ! app_fg "$dev" && ! grep -qE 'content-desc="(Sidebar|Collapse sidebar)"' <<<"$xml"; then
            timeout 25 ssh "$HOST" "adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
            sleep 2
            continue
        fi
        if grep -q 'content-desc="Collapse sidebar"' <<<"$xml"; then
            # 展开的侧栏会盖住列表，先按真实 bounds 收起，别用写死的坐标。
            b=$(grep -oE 'content-desc="Collapse sidebar"[^>]*bounds="[^"]+"' <<<"$xml" | tail -1) || true
            b=${b##*bounds=\"}
            cs=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
            if [[ $cs =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
                read -r x1 y1 x2 y2 <<<"$cs"
                timeout 25 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))" >/dev/null 2>&1 || true
            else
                timeout 25 ssh "$HOST" "adb -s $dev shell input tap 983 1200" >/dev/null 2>&1 || true
            fi
            sleep 2
            continue
        fi
        # 房名在就先匹配：横幅与房名可同页共存（04:46 实测），recovery 分支先走会让
        # 匹配器永远轮不到；横幅留给进房后处理。
        row=$(printf '%s' "$xml" | grep -oE 'text="[^"]*'"$name"'[^"]*"' | head -1) || true
        [[ -n $row ]] && break
        if grep -qE 'Verify with another device|Enter (your )?recovery key|This device.c?an.t see past messages yet|text="Chat backup"' <<<"$xml"; then
            to_list "$dev"; continue
        fi
        if (( i % 15 == 0 )); then log "open_room i=$i len=${#xml} fg=$(app_fg "$dev" && echo fg || echo bg)"; fi
        if grep -q 'text="End call"' <<<"$xml" || grep -q 'content-desc="Close page"' <<<"$xml"; then
            timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent BACK" || true
            sleep 2
            continue
        fi
        if grep -q 'text="Later"' <<<"$xml"; then tap_text "$dev" "Later" >/dev/null 2>&1 || true; sleep 2; continue; fi
        if grep -q "I don't have my other device" <<<"$xml"; then tap_text "$dev" "I don't have my other device" >/dev/null 2>&1 || true; sleep 2; continue; fi
        # 验证向导只退不进：开始验证会引入需要人守的多步流程，冒烟一律 BACK。
        if grep -q 'text="Start verification"' <<<"$xml"; then to_list "$dev"; continue; fi
        if [[ $name != call-smoke ]] && grep -q 'text="Element Call |' <<<"$xml"; then return 0; fi
        row=$(printf '%s' "$xml" | grep -oE 'text="[^"]*'"$name"'[^"]*"' | head -1) || true
        [[ -n $row ]] && break
        timeout 25 ssh "$HOST" "adb -s $dev shell input swipe 250 1400 250 700 300" >/dev/null 2>&1 || true
        sleep 1
    done
    if [[ -z ${row:-} ]]; then
        # 死前留档：最后一次 dump 原文与前台状态，别再盲猜循环里看到的是什么。
        xml=$(ui "$dev" 2>/dev/null || true)
        { echo "== $(date +%T) $dev fg=$(ssh "$HOST" "adb -s $dev shell dumpsys window 2>/dev/null | grep -o 'mCurrentFocus=[^ ]*'" | tr -d '\r')"; printf '%s\n' "$xml" | head -40; } >> "$EVIDENCE/openroom-debug.log" 2>&1 || true
        die "房间 $name 不在设备 $dev 的列表里"
    fi
    local label=${row#text=\"}; label=${label%\"}
    tap_name "$dev" "$label"
    sleep 5
    grep -qF "$name" <<<"$(ui "$dev")" || die "设备 $dev 没打开房间 $name"
}

open_call() { # 顶栏的入口，通话中时是“通话中”
    local dev=$1 port=9331
    [[ $dev == "$DEV_B" ]] && port=9334
    for i in $(seq 1 20); do
        local xml desc bounds coords x1 y1 x2 y2 page
        xml=$(ui "$dev" 2>/dev/null || true)
        if grep -q 'text="End call"\|text="Element Call |' <<<"$xml"; then return 0; fi
        page=$(cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" pick 2>/dev/null || true)
        if [[ -n $page ]] && cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" eval "$page" 'document.readyState' 2>/dev/null | grep -q complete; then return 0; fi
        if grep -q 'content-desc="In call"' <<<"$xml"; then desc='In call'; else desc='Video call'; fi
        coords=$(grep -oE "content-desc=\"$desc\"[^>]*bounds=\"[^\"]+\"" <<<"$xml" | tail -1) || true
        if [[ -n $coords ]]; then
            bounds=${coords##*bounds=\"}
            coords=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${bounds%\"}")
        else
            # 入口没有 content-desc 时：若退到了列表（房间行可见），先进房再找入口；
            # 盲按顶栏会落空，B 就永远进不了通话页。
            row=$(printf '%s' "$xml" | grep -oE 'text="[^"]*call-smoke-[^"]*"' | head -1) || true
            if [[ -n $row ]]; then
                label=${row#text=\"}; label=${label%\"}
                tap_name "$dev" "$label" >/dev/null 2>&1 || true
                sleep 3
                continue
            fi
            coords="1000 207 1000 207" # 顶栏座标兜底：列表也没有时的最后手段
        fi
        if [[ $coords =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
            read -r x1 y1 x2 y2 <<<"$coords"
            timeout 25 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
            sleep 4
            return 0
        fi
        sleep 1
    done
    die "设备 $dev 上没有通话入口"
}

# 网页里的 Join：页面在 WebView 里，按钮靠 uiautomator 的文本也能看到。
join_call() {
    local dev=$1 port=9331 page txt i rect x y
    [[ $dev == "$DEV_B" ]] && port=9334
    # 核心重启后隧道重连要几分钟，40 轮（~3 分钟）不够（relay 组实测），放宽到 100。
    for i in $(seq 1 100); do
        page=$(cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" pick 2>/dev/null || true)
        if [[ -n $page ]]; then
            # pipefail 下 cdp/ssh 瞬断会让赋值非零触发 set -e 静默死，轮询赋值一律容错。
            txt=$(ptext "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" "$page") || true
            grep -q 'End call' <<<"$txt" && return 0
            if grep -q 'Join call' <<<"$txt"; then
                # 底栏是 WebView 内容：矩形由 premain 的 __joinRect 给出，点击走 CDP（合成点击对底栏无效）。
                rect=$(cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" eval "$page" 'JSON.stringify(window.__joinRect ? window.__joinRect() : null)') || true
                x=$(python3 -c 'import json,sys
try: print(json.loads(sys.argv[1])["x"])
except Exception: pass' "$rect" 2>/dev/null || true)
                y=$(python3 -c 'import json,sys
try: print(json.loads(sys.argv[1])["y"])
except Exception: pass' "$rect" 2>/dev/null || true)
                if [[ -n $x && -n $y ]]; then
                    cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" click "$page" "$x" "$y" >/dev/null 2>&1 || true
                else
                    timeout 25 ssh "$HOST" "adb -s $dev shell input tap 540 1906" >/dev/null 2>&1 || true
                fi
                sleep 8; continue
            fi
        fi
        (( i % 8 == 0 )) && open_call "$dev"
        sleep 2
    done
    die "设备 $dev 没进通话（$page）"
}

# CDP 客户端跑在 $HOST 上（栈副本目录里，见上方推送）；ssh -L 在部分机器上被禁，不用转发端口。
cdp() { # cdp <a|b> <端口> <子命令> [参数]
    local dev port pids sock="" cand q=""
    if [[ $1 == a ]]; then dev=$DEV_A; else dev=$DEV_B; fi
    port=$2
    shift 2
    # 套接字名里是持有 WebView 的进程 pid，pidof 可能同时列出多个（主进程 + 隔离进程），
    # 按 pid 逐个比对，别赌第一个。
    pids=$(timeout 25 ssh "$HOST" "adb -s $dev shell pidof app.wonton.android" | tr -d '\r' | tr '\n' ' ')
    for cand in $(timeout 25 ssh "$HOST" "adb -s $dev shell cat /proc/net/unix" | grep -o 'webview_devtools_remote_[0-9]*' | sort -u); do
        if [[ " $pids " == *" ${cand##*_} "* ]]; then sock=$cand; fi
    done
    [[ -n $sock ]] || sock="webview_devtools_remote_${pids%% *}"
    timeout 25 ssh "$HOST" "adb -s $dev forward --remove tcp:$port; adb -s $dev forward tcp:$port localabstract:$sock" >/dev/null 2>&1 || true
    printf -v q ' %q' "$@"
    # 194 的系统 python 没有 websocket-client（重启后 pip 模块丢失）；纯 Python 包随栈副本带上。
    timeout 90 ssh "$HOST" "cd $RCD && no_proxy=* PYTHONPATH=$RCD/pylib python3 cdp.py --port $port$q"
}
# widget 页上的可见文本（CDP 一次 eval，不碰 uiautomator）。
ptext() { cdp "$1" "$2" eval "$3" 'document.body.innerText' 2>/dev/null | tail -1; }

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

wait_members() { # wait_members <房间> <期望人数> <秒>
    local i out n
    for i in $(seq 1 "$3"); do
        out=$(members "$1" || true)
        n=$(grep -c '@' <<<"$out" || true)
        if [[ $n -ge $2 ]]; then
            printf '%s\n' "$out" | tee -a "$EVIDENCE/members.log"
            return 0
        fi
        sleep 1
    done
    printf '%s\n' "$out" | tee -a "$EVIDENCE/members.log"
    return 1
}

stats() { # stats <a|b> <端口> <组名>：一端 10 个采样落盘，按最后一个采样汇总候选对和收发字节
    local page
    page=$(cdp "$1" "$2" pick 2>/dev/null || true)
    [[ -n $page ]] || { log "统计：设备 $1 没有 widget 页"; return 1; }
    # 同连接装钩子+采样：target 换代后旧注入是死信，采样时现装才落在本页上。
    cdp "$1" "$2" session "$page" premain.js 15 > "$EVIDENCE/webrtc-$1-$3.jsonl" 2>>"$EVIDENCE/webrtc-$1.err" || true
    python3 - "$EVIDENCE/webrtc-$1-$3.jsonl" <<'PEOF' | tee -a "$EVIDENCE/run.log"
import json, sys
samples = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
if not samples:
    print("统计为空"); raise SystemExit
last = samples[-1]
for pc in last.get("pcs") or []:
    cands = {c.get("id"): c for c in pc.get("candidates") or []}
    sel = pc.get("selected") or {}
    local = cands.get(sel.get("local")) or {}
    remote = cands.get(sel.get("remote")) or {}
    out = pc.get("outbound", {}).get("video") or {}
    inn = pc.get("inbound", {}).get("video") or {}
    print(f"state={pc.get('state')} pair={local.get('type')}/{local.get('protocol')}->{remote.get('type')}"
          f" rtt={sel.get('rtt')} sent={out.get('bytes')} recv={inn.get('bytes')} frames={inn.get('frames')} lost={inn.get('lost')}")
PEOF
}
configure_udp_only() { log "路径：UDP 直连（插件默认）"; }
configure_tcp_only() {
    log "路径：只留 TCP 7881"
    local dc=$pkg/docker-compose.yml
    [[ -f $dc.orig ]] || cp "$dc" "$dc.orig"
    sed -i 's/^      UDP_PORT: "7882"$/      UDP_PORT: "0"/' "$dc"
    # 插件 compose 里 livekit 的 UDP 端口映射也去掉：隧道不再转发 7882。
    sed -i '/service: livekit, port: 7882, protocol: udp/d' "$dc"
    # remote-dev install 只收 worktree 相对路径，pkg 保持相对（绝对化会被拒）。
    # 同版本已装会被拒：先幂等卸载（未装时静默），ssh 闪断已由落盘+响亮 die 兜住。
    dev_sh uninstall app.element-call >/dev/null 2>&1 || true
    # install 的审查确认读 stdin，setsid 的 /dev/null 会让它 EOF 静默弃装——喂 y。
    echo y | "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" install "$pkg" >"$EVIDENCE/install-tcp.log" 2>&1 ||
        die "插件重装失败（组 tcp，见 install-tcp.log）"
    sleep 20
}
restore_compose() { # 把就地改过的插件 compose 还原（副本在插件仓库外，git checkout 无效）
    local dc=$pkg/docker-compose.yml
    if [[ -f $dc.orig ]]; then cp "$dc.orig" "$dc" && rm "$dc.orig"; log "已还原插件 compose"; fi
}
configure_relay_only() {
    log "路径：强制 n0 中继（宣告一个到不了的直连地址）"
    dev_sh status --announce 10.0.2.99 >/dev/null
    timeout 60 ssh "$HOST-stack" restart "$STACK" core
    for _ in $(seq 1 60); do dev_sh status 2>/dev/null | grep -q '已连接\|connected' && break; sleep 2; done
    # 设备已是全新配对态（10:2x 手动配对+同步实证），pm clear 会摧毁好状态且 pair()
    # 的跳过判据会把 pm clear 后的引导页误判为已配对（三轮同断点），故跳过清与重配。
    # timeout 25 ssh "$HOST" "adb -s $DEV_A shell pm clear app.wonton.android; adb -s $DEV_B shell pm clear app.wonton.android"
    # pair a "$DEV_A"; pair b "$DEV_B"
    open_room "$DEV_A" "$room_name"; open_room "$DEV_B" "$room_name"
    sleep 3
}

# —— 准备 ——
[[ -f $pkg/docker-compose.yml ]] || die "插件包不在 $pkg"
log "预检"
# dev.sh status 会跑 docker compose ps，在共用机上常常一卡几分钟；这里只看 .dev-ports 和网关是否回 HTTP（根路径 401/404 都算在）。
read -r gw tunnel < <(timeout 25 ssh "$HOST" "cat ~/wonton-dev/stacks/dev-$STACK/.dev-ports")
[[ -n ${gw:-} && -n ${tunnel:-} ]] || die "栈 $STACK 的 .dev-ports 没有（栈没起？）"
code=$(timeout 25 ssh "$HOST" "curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$gw/" || true)
[[ $code != 000 ]] || die "网关 127.0.0.1:$gw 不在"
timeout 25 ssh "$HOST" "adb -s $DEV_A shell pm path app.wonton.android; adb -s $DEV_B shell pm path app.wonton.android" | grep -c package | grep -q 2 || die "App 没装上"
for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d reverse tcp:$gw tcp:$gw; adb -s $d reverse tcp:$tunnel tcp:$tunnel" >/dev/null; done
if [[ -n ${ROOM_ID:-} ]]; then
    room_name=$ROOM_NAME room_id=$ROOM_ID
    log "复用房间 $room_id"
else
    create_room
fi
pair a "$DEV_A"; pair b "$DEV_B"
# 屏幕一灭 uiautomator dump 与点按全部失效（轮 17-22 的爬行皆源于此）：保持常亮并唤醒。
for d in "$DEV_A" "$DEV_B"; do
    timeout 25 ssh "$HOST" "adb -s $d shell settings put system screen_off_timeout 1800000; adb -s $d shell svc power stayon true; adb -s $d shell input keyevent 224" >/dev/null 2>&1 || true
done
# 冷启动刷新同步：刚建的房间要等 App 拉到才出现在列表里；跳过配对的轮没有重启，列表是旧的。
for d in "$DEV_A" "$DEV_B"; do
    timeout 25 ssh "$HOST" "adb -s $d shell am force-stop app.wonton.android; adb -s $d shell am start -n app.wonton.android/.MainActivity" >/dev/null || true
done
sleep 6
to_list "$DEV_A"; to_list "$DEV_B"
timeout 25 ssh "$HOST" "adb -s $DEV_A shell pm grant app.wonton.android android.permission.CAMERA; adb -s $DEV_A shell pm grant app.wonton.android android.permission.RECORD_AUDIO; adb -s $DEV_B shell pm grant app.wonton.android android.permission.CAMERA; adb -s $DEV_B shell pm grant app.wonton.android android.permission.RECORD_AUDIO" >/dev/null 2>&1 || true
# 复用房间时设备可能停在断连态（Trying to reach your home server…）：
# 冷启动让同步引擎重连，不指望它自己恢复；否则 open_room 会对空列表空转。
for d in "$DEV_A" "$DEV_B"; do
    ssh "$HOST" "adb -s $d shell am force-stop app.wonton.android; adb -s $d shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
done
sleep 8
# 不再无条件 BACK：to_list 刚把 App 带回列表，BACK 会把它顶回桌面，open_room 就只能在桌面滑动。
clear_dialogs
open_room "$DEV_A" "$room_name"
open_room "$DEV_B" "$room_name"
sleep 3

inject_stats() { # 提前注入 premain，让页面自己抓 RTCPeerConnection；注入后必须验证，丢了的包裹器等于没注
    local port=9331 page t
    [[ $1 == b ]] && port=9334
    for _ in $(seq 1 30); do
        page=$(cdp "$1" "$port" pick 2>/dev/null || true)
        [[ -n $page ]] && break
        sleep 1
    done
    [[ -n $page ]] || { log "设备 $1 取不到 widget 页"; return 1; }
    for _ in $(seq 1 8); do
        cdp "$1" "$port" addscript "$page" premain.js >/dev/null 2>&1 || true
        cdp "$1" "$port" evalscript "$page" premain.js >/dev/null 2>&1 || true
        t=$(cdp "$1" "$port" eval "$page" 'typeof window.__pcs' 2>/dev/null | python3 -c 'import json,sys
try: print(json.loads(sys.stdin.read()).get("value",""))
except Exception: pass' || true)
        u=$(cdp "$1" "$port" eval "$page" 'location.href.slice(0,70)' 2>/dev/null | python3 -c 'import json,sys
try: print(json.loads(sys.stdin.read()).get("value",""))
except Exception: pass' || true)
        log "设备 $1 注入轮：page=$page typeof=${t:-err} url=$u"
        [[ $t == object ]] && { log "设备 $1 注入验证通过（page=$page）"; return 0; }
        sleep 2
    done
    log "设备 $1 注入验证失败（page=$page typeof=${t:-空}）；通话页可能在 join 时重建 target"
    return 1
}

hangup() { # 关掉 widget 页（页面卸载时 Element Call 发 leave）
    local d=$1 xml b c x1 y1 x2 y2
    xml=$(ui "$d" 2>/dev/null || true)
    c=$(grep -oE 'content-desc="Close page"[^>]*bounds="[^"]+"' <<<"$xml" | tail -1) || true
    b=${c##*bounds=\"}
    c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
    if [[ $c =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
        read -r x1 y1 x2 y2 <<<"$c"
        timeout 25 ssh "$HOST" "adb -s $d shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
    else
        log "设备 $d 没找到 Close page，页面可能已经关了"
    fi
}

run_group() { # run_group <a|b|c> <说明> <配置函数>
    log "== 组 $1：$2"
    "$3" || die "组 $1 配置失败（set -e 静默死的教训：配置失败必须带名字响亮报）"
    clear_dialogs
    for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true; done
    sleep 3
    # 强杀过的一端可能停在桌面或别的会话：先各自回到本房间，再开顶栏入口。
    open_room "$DEV_A" "$room_name"; open_room "$DEV_B" "$room_name"
    # 包裹器挂 getStats 原型，须在通话建立、页面稳定后注入（pre-join 注入会被 EC 重置，05:2x 实测）。
    open_call "$DEV_A"; join_call "$DEV_A"; inject_stats a || log "组 $1：a 注入失败"
    sleep 15
    open_call "$DEV_B"; join_call "$DEV_B"; inject_stats b || log "组 $1：b 注入失败"
    # 从聊天回到通话页会停在 Join call，join_call 会再点一次。
    join_call "$DEV_A" || true
    wait_members "$room_id" 2 60 || log "组 $1：call.member 读不到，改用核心时间线里的通话状态兜底"
    stack logs core 400 2>/dev/null | grep 'call.member' | tail -5 >> "$EVIDENCE/members.log" || true
    sleep 15
    if [[ $1 == a ]]; then
        # 两端同时录 20 秒：媒体路径的样子落成视频证据。
        rec "$DEV_A" "group-$1-a"; rec "$DEV_B" "group-$1-b"; sleep 22
        fetch_rec "$DEV_A" "group-$1-a"; fetch_rec "$DEV_B" "group-$1-b"
    fi
    shot "$DEV_A" "group-$1-a"; shot "$DEV_B" "group-$1-b"
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d shell dumpsys package app.wonton.android | grep -E 'android.permission.(CAMERA|RECORD_AUDIO): granted'"
    done | tee -a "$EVIDENCE/permissions.log" >/dev/null
    stats a 9331 "$1" || true; stats b 9334 "$1" || true
    local want=udp
    [[ $1 == b ]] && want=tcp
    [[ $1 == c ]] && want=relay
    if [[ $(tail -n 40 "$EVIDENCE/run.log" | grep -c "/$want->") -ge 2 ]]; then
        log "组 $1 判定：两端选中的都是 $want"
    else
        log "组 $1 判定：没看到两端选中 $want（看 webrtc-*.jsonl）"
    fi
    timeout 25 ssh "$HOST" "adb -s $DEV_A shell cmd media_session volume --stream 3 --set 0" >/dev/null 2>&1 || true
    hangup "$DEV_A"; hangup "$DEV_B"
    sleep 10
    shot "$DEV_A" "group-$1-after"
}

run_kill() { # 延迟 leave：B 正常挂断，再把 A 的 App 强杀，成员应在十几秒内被清掉
    log "== 组 kill：延迟 leave"
    clear_dialogs
    open_call "$DEV_A"; join_call "$DEV_A"; inject_stats a || log "组 kill：a 注入失败"
    sleep 15
    open_call "$DEV_B"; join_call "$DEV_B"; inject_stats b || log "组 kill：b 注入失败"
    wait_members "$room_id" 2 60 || log "组 kill：成员没到 2"
    hangup "$DEV_B"
    sleep 5
    log "强杀前成员=$(members "$room_id" | grep -c '@' || true)"
    timeout 25 ssh "$HOST" "adb -s $DEV_A shell am force-stop app.wonton.android"
    local t0 i n
    t0=$(date +%s)
    for i in $(seq 1 30); do
        sleep 2
        n=$(members "$room_id" | grep -c '@' || true)
        log "强杀后 $(( $(date +%s) - t0 ))s 成员=$n"
        if [[ $n -eq 0 ]]; then
            if [[ $(( $(date +%s) - t0 )) -le 25 ]]; then log "延迟 leave 正常：$(( $(date +%s) - t0 ))s"; else log "延迟 leave 偏慢：$(( $(date +%s) - t0 ))s"; fi
            return 0
        fi
    done
    log "延迟 leave 超时：60s 后仍有成员"
}

case "$stage" in
udp) run_group a "UDP 直连" configure_udp_only ;;
tcp) run_group b "只有 TCP" configure_tcp_only; restore_compose ;;
relay) run_group c "只有中继" configure_relay_only ;;
kill) run_kill ;;
all)
    run_group a "UDP 直连" configure_udp_only
    run_kill
    run_group b "只有 TCP" configure_tcp_only
    run_group c "只有中继" configure_relay_only
    restore_compose
    ;;
*) die "不认识的 STAGE=$stage" ;;
esac

log "完成"
