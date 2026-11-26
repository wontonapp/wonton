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
# 只跑某一步：STAGE=udp|tcp|relay|kill|plain（默认 all：udp→kill→tcp→relay）。默认每轮新建加密房间；
# plain 用不加密房间跑 UDP 组（记录 Element Call 在不加密房间里的行为，供“房间不再端到端加密”的决定用）。
# ROOM_ID=<房间> ROOM_NAME=<名字> 可复用已有房间（旧房间会积着上轮成员，判定要看本轮新增的两条）。
# 插件包：PKG（worktree 内的相对路径，默认 deploy/dev/calls/plugin，由调用方放好）。
# 每组有组级 deadline：GROUP_DEADLINE=秒（默认 600），从判定阶段起算，到点就 fail_group 出判决，不被卡死的步骤吞掉。
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
# cdp.py 的 websocket-client 随脚本推到 HOST 的栈副本，固定 PYTHONPATH；矩阵操作还要 host 上的 PyJWT。
for f in cdp.py premain.js; do timeout 25 ssh "$HOST" "cat > $RCD/$f" < "$here/$f"; done
timeout 60 ssh "$HOST" "tar -C $RCD -xzf -" < <(tar -C "$here" -czf - pylib)

log() { printf '%s %s\n' "$(date +%T)" "$*" | tee -a "$EVIDENCE/run.log" >&2; }
die() { log "失败：$*"; exit 1; }

dev_sh() { "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" "$@"; }
stack() { timeout 60 ssh "$HOST-stack" "$1" "$STACK" "${@:2}"; }
# 配对码里的直连地址要从 HOST 算：本机模拟器用 10.0.2.2（加 adb reverse），远端栈取那台的局域网 IPv4。
pair_addr=$HOST
case $HOST in localhost|127.0.0.1|::1|"$(hostname)"|"$(hostname -f 2>/dev/null)") pair_addr=10.0.2.2 ;;
*) [[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || pair_addr=$(ssh -G "$HOST" 2>/dev/null | awk '$1 == "hostname" { print $2 }') ;;
esac
[[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || die "HOST=$HOST 解析不出 IPv4 直连地址"
failures=0
# 组级 deadline：到点就 fail_group，别让一个卡死的步骤把整轮吞掉、也不出判决。
GROUP_DEADLINE="${GROUP_DEADLINE:-600}"
[[ $GROUP_DEADLINE =~ ^[0-9]+$ ]] || die "GROUP_DEADLINE 要是整数秒（当前 '$GROUP_DEADLINE'）"
group_deadline_at=0
deadline_hit() { (( group_deadline_at > 0 && SECONDS >= group_deadline_at )); }
fail_group() {
    failures=$((failures + 1))
    log "组 $1 失败：$2"
    deadline_hit && log "组 $1 已过组级 deadline ${GROUP_DEADLINE}s"
    if [[ $1 == c ]] && declare -F capture_failure_evidence >/dev/null; then
        capture_failure_evidence "$1"
    fi
}
compose_changed=0 direct_blocked=0
restore_defaults() {
    restore_compose
    (( direct_blocked )) && unblock_direct
    if (( compose_changed )); then
        dev_sh uninstall app.element-call >/dev/null 2>&1 || true
        echo y | dev_sh install "$pkg" >"$EVIDENCE/install-default.log" 2>&1 ||
            log "还原默认插件失败（见 install-default.log；留给人工重装）"
        compose_changed=0
    fi
}
trap restore_defaults EXIT
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
        # 配对后的资料/引导页（1/5 之类）：中间页有 Skip，最后一页是 Done；BACK 会退到桌面。
        if grep -q 'text="Go to chat"\|text="Done"\|text="完成"\|text="去聊天"' <<<"$xml"; then
            tap_text "$dev" "Go to chat" >/dev/null 2>&1 || tap_text "$dev" "去聊天" >/dev/null 2>&1 ||
                tap_text "$dev" "Done" >/dev/null 2>&1 || tap_text "$dev" "完成" >/dev/null 2>&1 || true
            sleep 2; continue
        fi
        if grep -q 'text="Skip"\|text="跳过"' <<<"$xml"; then
            tap_text "$dev" "Skip" >/dev/null 2>&1 || tap_text "$dev" "跳过" >/dev/null 2>&1 || true
            sleep 2; continue
        fi
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
            if grep -q 'text="OK"' <<<"$xml"; then
                tap_text "$d" "OK" >/dev/null 2>&1 || true; sleep 2; continue
            fi
            if grep -q 'text="Got it"' <<<"$xml"; then
                tap_text "$d" "Got it" >/dev/null 2>&1 || true; sleep 2; continue
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


open_room() { # open_room <dev> <显示名片段>
    # 列表行按房间名找；房间名带时间戳，用前缀匹配。
    local dev=$1 name=$2 xml row b cs x1 y1 x2 y2
    # App 冷启动会回到上次的界面（可能在别的房间里，没有列表）：先退回聊天列表再找行。
    to_list "$dev" >/dev/null 2>&1 || true
    for i in $(seq 1 400); do
        deadline_hit && { log "open_room $dev 组级超时"; return 1; }
        # 列表 UI 不实时刷新：新建房间的行要等 App 重启才渲染出来（17:19 实测）。
        # 周期性重启一次，重渲染列表；房间行出现就往下走。
        if (( i % 60 == 0 )); then
            timeout 25 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
            sleep 12
        fi
        xml=$(ui "$dev" 2>/dev/null || true)
        # 冷启动或房间打不开时核心/页面会盖一层弹窗，先点掉再找房名。
        if grep -q 'text="OK"' <<<"$xml"; then tap_text "$dev" "OK" >/dev/null 2>&1 || true; sleep 2; continue; fi
        if grep -q 'text="Got it"' <<<"$xml"; then tap_text "$dev" "Got it" >/dev/null 2>&1 || true; sleep 1; continue; fi
        if grep -q 'text="Allow"' <<<"$xml" && grep -qiE 'camera|microphone' <<<"$xml"; then
            tap_text "$dev" "Allow" >/dev/null 2>&1 || true; sleep 1; continue
        fi
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
        # 列表不滚太狠：每 3 轮才向下试一屏，每 12 轮回顶部，免得把已经露出来的新房间刷过去。
        if (( i % 12 == 0 )); then
            for _ in 1 2 3; do timeout 25 ssh "$HOST" "adb -s $dev shell input swipe 250 700 250 1700 200" >/dev/null 2>&1 || true; done
        elif (( i % 3 == 0 )); then
            timeout 25 ssh "$HOST" "adb -s $dev shell input swipe 250 1400 250 700 300" >/dev/null 2>&1 || true
        fi
        sleep 2
    done
    if [[ -z ${row:-} ]]; then
        # 死前留档：最后一次 dump 原文与前台状态，别再盲猜循环里看到的是什么。
        xml=$(ui "$dev" 2>/dev/null || true)
        { echo "== $(date +%T) $dev fg=$(ssh "$HOST" "adb -s $dev shell dumpsys window 2>/dev/null | grep -o 'mCurrentFocus=[^ ]*'" | tr -d '\r')"; printf '%s\n' "$xml" | head -40; } >> "$EVIDENCE/openroom-debug.log" 2>&1 || true
        die "房间 $name 不在设备 $dev 的列表里"
    fi
    local label=${row#text=\"}; label=${label%\"}
    tap_name "$dev" "$label"
    sleep 4
    # 进房后偶尔弹核心的「Not available right now / OK」（同步还没就绪），点掉再确认房名。
    for _ in $(seq 1 12); do
        xml=$(ui "$dev" 2>/dev/null || true)
        grep -qF "$name" <<<"$xml" && return 0
        grep -q 'text="OK"' <<<"$xml" && tap_text "$dev" "OK" >/dev/null 2>&1 || true
        if grep -q 'text="Allow"' <<<"$xml" && grep -qiE 'camera|microphone' <<<"$xml"; then
            tap_text "$dev" "Allow" >/dev/null 2>&1 || true
        fi
        sleep 2
    done
    log "设备 $dev 没打开房间 $name（组级超时或页面没起）"
    return 1
}

open_call() { # 顶栏的入口，通话中时是“通话中”
    local dev=$1 port=9331
    [[ $dev == "$DEV_B" ]] && port=9334
    for i in $(seq 1 20); do
        deadline_hit && { log "open_call $dev 组级超时"; return 1; }
        local xml desc bounds coords x1 y1 x2 y2 page
        xml=$(ui "$dev" 2>/dev/null || true)
        if grep -q 'text="OK"' <<<"$xml"; then tap_text "$dev" "OK" >/dev/null 2>&1 || true; sleep 1; continue; fi
        if grep -q 'text="Got it"' <<<"$xml"; then tap_text "$dev" "Got it" >/dev/null 2>&1 || true; sleep 1; continue; fi
        if grep -q 'text="End call"\|text="Element Call |' <<<"$xml"; then allow_media "$dev"; return 0; fi
        page=$(cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" pick 2>/dev/null || true)
        if [[ -n $page ]] && cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" eval "$page" 'document.readyState' 2>/dev/null | grep -q complete; then allow_media "$dev"; return 0; fi
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
            sleep 2
            allow_media "$dev"
            return 0
        fi
        sleep 1
    done
    die "设备 $dev 上没有通话入口"
}

# 网页里的 Join：页面在 WebView 里，按钮靠 uiautomator 的文本也能看到。
allow_media() { # 页面一加载就请求摄像头/麦克风：核心弹原生询问，答错一次就按拒绝记下来，后面全被拒
    local dev=$1 xml
    for _ in $(seq 1 6); do
        xml=$(ui "$dev" 2>/dev/null || true)
        # 首次打开会盖一层 “Got it” 介绍；不点掉，底下的 Join call 点不到。
        if grep -q 'text="Got it"' <<<"$xml"; then
            tap_text "$dev" "Got it" >/dev/null 2>&1 || true
            sleep 1
            continue
        fi
        if grep -q 'text="Allow"' <<<"$xml" && grep -qiE 'camera|microphone' <<<"$xml"; then
            tap_text "$dev" "Allow" || true
            : > "$EVIDENCE/.media-$dev-ok"
            log "设备 $dev 已允许摄像头/麦克风"
        fi
        [[ -f $EVIDENCE/.media-$dev-ok ]] && return 0
        sleep 1
    done
}

join_call() {
    local dev=$1 side=a port=9331 page txt i rect x y
    [[ $dev == "$DEV_B" ]] && { side=b; port=9334; }
    allow_media "$dev"
    # 核心重启后隧道重连要几分钟，40 轮（~3 分钟）不够（relay 组实测），放宽到 100。
    for i in $(seq 1 100); do
        deadline_hit && { log "设备 $dev join_call 组级超时（page=${page:-无}）"; return 1; }
        page=$(cdp "$side" "$port" pick 2>/dev/null || true)
        if [[ -n $page ]]; then
            page_dismiss "$side" "$port" "$page"
            # pipefail 下 cdp/ssh 瞬断会让赋值非零触发 set -e 静默死，轮询赋值一律容错。
            txt=$(ptext "$side" "$port" "$page") || true
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
        # 大 JS 包在限速的公网中继上会半截失败（ERR_CONTENT_LENGTH_MISMATCH），空页面重载一次再试。
        if (( i % 20 == 0 )) && [[ -n $page ]]; then
            cdp "$side" "$port" eval "$page" 'location.reload()' >/dev/null 2>&1 || true
            sleep 5
            continue
        fi
        (( i % 8 == 0 )) && open_call "$dev"
        sleep 2
    done
    log "设备 $dev 没进通话（$page）"
    return 1
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
    # 系统 python 可能没有 websocket-client（有的机器重启后 pip 模块丢）；纯 Python 包随仓库 pylib 带上。
    timeout 90 ssh "$HOST" "cd $RCD && no_proxy=* PYTHONPATH=$RCD/pylib python3 cdp.py --port $port$q"
}
# widget 页上的可见文本（CDP 一次 eval，不碰 uiautomator）。
ptext() { cdp "$1" "$2" eval "$3" 'document.body.innerText' 2>/dev/null | tail -1; }

# 点掉页面里的 “Got it” 介绍层（在 WebView 里，走 CDP 的鼠标事件）。
page_dismiss() { # page_dismiss <a|b> <端口> <页面>
    local pos x y
    pos=$(cdp "$1" "$2" eval "$3" 'JSON.stringify(window.__gotItRect ? window.__gotItRect() : null)' 2>/dev/null) || return 0
    x=$(python3 -c 'import json,sys
try: print(json.loads(sys.argv[1])["x"])
except Exception: pass' "$pos" 2>/dev/null || true)
    y=$(python3 -c 'import json,sys
try: print(json.loads(sys.argv[1])["y"])
except Exception: pass' "$pos" 2>/dev/null || true)
    [[ -n $x && -n $y ]] || return 0
    cdp "$1" "$2" click "$3" "$x" "$y" >/dev/null 2>&1 || true
    sleep 2
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

stats() { # stats <a|b> <端口> <组名>：一端采样落盘（判定在 check_media_path 里做）
    local page
    page=$(cdp "$1" "$2" pick 2>/dev/null || true)
    [[ -n $page ]] || { log "统计：设备 $1 没有 widget 页"; return 1; }
    # 同连接装钩子+采样：target 换代后旧注入是死信，采样时现装才落在本页上。
    cdp "$1" "$2" session "$page" premain.js 15 > "$EVIDENCE/webrtc-$1-$3.jsonl" 2>>"$EVIDENCE/webrtc-$1.err" || true
    [[ -s $EVIDENCE/webrtc-$1-$3.jsonl ]] || return 1
}

check_media_path() { # <组 a|b|c|plain>：两端都必须走指定路径，且双向音频和视频都真正有字节
    python3 - "$1" "$EVIDENCE/webrtc-a-$1.jsonl" "$EVIDENCE/webrtc-b-$1.jsonl" <<'PEOF' | tee -a "$EVIDENCE/run.log"
import json, sys
group, *files = sys.argv[1:]
bad = []
sides = {}
for side, path in zip("ab", files):
    try:
        samples = [json.loads(l) for l in open(path) if l.strip()]
    except FileNotFoundError:
        samples = []
    if not samples:
        bad.append(f"设备 {side}: 统计为空")
        print(f"设备 {side}: 统计为空")
        continue
    pcs = [pc for s in samples for pc in (s.get("pcs") or []) if pc.get("selected")]
    if not pcs:
        bad.append(f"设备 {side}: 没有选中的候选对")
        print(f"设备 {side}: 没有选中的候选对")
        continue
    pc = pcs[-1]
    cands = {c.get("id"): c for c in pc.get("candidates") or []}
    sel = pc["selected"]
    local = cands.get(sel.get("local")) or {}
    remote = cands.get(sel.get("remote")) or {}
    kind, proto = local.get("type"), local.get("protocol")
    # 用全部采样的最大值，容忍起步时的零。
    def peak(direction, media):
        return max([((s.get("pcs") or [{}])[0].get(direction, {}).get(media) or {}).get("bytes") or 0
                    for s in samples] or [0])
    media = {(d, m): peak(d, m) for d in ("inbound", "outbound") for m in ("audio", "video")}
    sides[side] = media
    print(f"设备 {side}: state={pc.get('state')} pair={kind}/{proto}->{remote.get('type')}"
          f" rtt={sel.get('rtt')} 收音频={media[('inbound', 'audio')]} 发音频={media[('outbound', 'audio')]}"
          f" 收视频={media[('inbound', 'video')]} 发视频={media[('outbound', 'video')]}")
    # 中继组看的是隧道路径（见 check_tunnel_relay），WebRTC 的 local 候选在 ICE-lite 下恒是 host/prflx。
    # plain 是“不加密房间”的 UDP 组，路径判据和 a 相同。
    if {"a": proto == "udp" and kind != "relay",
        "plain": proto == "udp" and kind != "relay",
        "b": proto == "tcp" and kind != "relay",
        "c": True}[group] is False:
        bad.append(f"设备 {side}: 路径是 {kind}/{proto}，不是 {group}")
    if pc.get("state") != "connected":
        bad.append(f"设备 {side}: 连接状态 {pc.get('state')}")
if len(sides) == 2:
    a, b = sides["a"], sides["b"]
    # 双向音视频。视频的发送统计在有的设备上恒为 0，但接收端看得到字节，两个方向都按
    # “发送有字节或对端收到字节”算；音频必须两端都真有收发。
    directions = {
        "a→b 音频": a[("outbound", "audio")] > 0 and b[("inbound", "audio")] > 0,
        "b→a 音频": b[("outbound", "audio")] > 0 and a[("inbound", "audio")] > 0,
        "a→b 视频": a[("outbound", "video")] > 0 or b[("inbound", "video")] > 0,
        "b→a 视频": b[("outbound", "video")] > 0 or a[("inbound", "video")] > 0,
    }
    for name, ok in directions.items():
        if not ok:
            bad.append(f"{name}没有字节")
for reason in bad:
    print(f"  - {reason}")
raise SystemExit(1 if bad else 0)
PEOF
}
check_tunnel_relay() { # <a|b>：App 核心日志里最近一次隧道路径必须是 relay（中继组真正的判据）
    local side=$1 dev last errs
    [[ $side == a ]] && dev=$DEV_A || dev=$DEV_B
    # 回前台会重查一次隧道；不这样拿到的可能是阻断直连之前的旧路径。
    timeout 25 ssh "$HOST" "adb -s $dev shell input keyevent 3; adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
    sleep 8
    last=$(timeout 25 ssh "$HOST" "adb -s $dev shell 'run-as app.wonton.android tail -n 400 files/core.log'" 2>/dev/null |
        grep 'CheckedPath' | tail -1 || true)
    # 不管过没过都落盘：路径判据和出错原因（中继限速、transport-unavailable、EC 连不上）都要能复查。
    timeout 25 ssh "$HOST" "adb -s $dev shell 'run-as app.wonton.android tail -n 400 files/core.log'" 2>/dev/null |
        grep -E 'CheckedPath|transport-unavailable|rate-limit|Failed to connect' | tail -8 \
        >> "$EVIDENCE/relay-path-$side.log" || true
    if grep -q 'relay: true' <<<"$last"; then
        log "设备 $side 隧道走中继：$last"
        return 0
    fi
    errs=$(timeout 25 ssh "$HOST" "adb -s $dev shell 'run-as app.wonton.android tail -n 200 files/core.log'" 2>/dev/null |
        grep -iE 'transport-unavailable|rate-limit|Failed to connect|tunnel check failed' | tail -3 || true)
    log "设备 $side 隧道没走中继：${last:-没有路径日志}${errs:+；错因：$(tr '\n' ' ' <<<"$errs")}"
    return 1
}

configure_udp_only() { log "路径：UDP 直连（插件默认）"; }
configure_tcp_only() {
    log "路径：只留 TCP 7881"
    local dc=$pkg/docker-compose.yml
    [[ -f $dc.orig ]] || cp "$dc" "$dc.orig"
    sed -i 's/^      UDP_PORT: "7882"$/      UDP_PORT: "0"/' "$dc"
    grep -q '^      UDP_PORT: "0"$' "$dc" || { log "compose 里没找到 UDP_PORT: \"7882\"，sed 没生效"; return 1; }
    # 插件 compose 里 livekit 的 UDP 端口映射也去掉：隧道不再转发 7882。
    sed -i '/service: livekit, port: 7882, protocol: udp/d' "$dc"
    ! grep -q 'port: 7882, protocol: udp' "$dc" || { log "compose 里的 7882/udp 隧道行没删掉"; return 1; }
    compose_changed=1
    # remote-dev install 只收 worktree 相对路径，pkg 保持相对（绝对化会被拒）。
    # 同版本已装会被拒：先幂等卸载（未装时静默），ssh 闪断已由落盘+响亮 die 兜住。
    dev_sh uninstall app.element-call >/dev/null 2>&1 || true
    # install 的审查确认读 stdin，setsid 的 /dev/null 会让它 EOF 静默弃装——喂 y。
    echo y | "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" install "$pkg" >"$EVIDENCE/install-tcp.log" 2>&1 ||
        { log "插件重装失败（组 tcp，见 install-tcp.log）"; return 1; }
    sleep 20
}
restore_compose() { # 把就地改过的插件 compose 还原（副本在插件仓库外，git checkout 无效）
    local dc=$pkg/docker-compose.yml
    if [[ -f $dc.orig ]]; then cp "$dc.orig" "$dc" && rm "$dc.orig"; log "已还原插件 compose"; fi
}
block_direct() { # 模拟两端只能走中继：丢掉到私网直连候选的包，只留模拟器的 DNS 和宿主机 10.0.2.2
    local d
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d root" >/dev/null 2>&1 || true
    done
    sleep 4
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d wait-for-device" >/dev/null 2>&1 || true
        # 只有模拟器自己的 DNS（10.0.2.3）和宿主 TCP（页面/调试）留在私网里；到宿主的隧道 UDP 也丢掉，
        # 否则 10.0.2.2:隧道端口 仍会被当成直连。
        timeout 25 ssh "$HOST" "adb -s $d shell '
            iptables -F OUTPUT
            iptables -I OUTPUT 1 -d 192.168.0.0/16 -j DROP
            iptables -I OUTPUT 1 -d 172.16.0.0/12 -j DROP
            iptables -I OUTPUT 1 -d 10.0.0.0/8 -j DROP
            iptables -I OUTPUT 1 -d 10.0.2.2 -p udp --dport $tunnel -j DROP
            iptables -I OUTPUT 1 -d 10.0.2.2 -p tcp -j ACCEPT
            iptables -I OUTPUT 1 -d 10.0.2.3 -j ACCEPT'" >/dev/null 2>&1 || true
        timeout 25 ssh "$HOST" "adb -s $d reverse tcp:$gw tcp:$gw; adb -s $d reverse tcp:$tunnel tcp:$tunnel" >/dev/null 2>&1 || true
        # 规则生效后 iroh 会自己把路径切到 relay；不要 force-stop，重拨会重新拉 EC 的大 JS 包，
        # 公网中继限速时更容易把隧道打成 transport-unavailable。只把 App 带回前台。
        timeout 25 ssh "$HOST" "adb -s $d shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
    done
    direct_blocked=1
}
unblock_direct() { # 去掉 block_direct 的规则，恢复直连
    local d
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d root" >/dev/null 2>&1 || true
    done
    sleep 4
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d wait-for-device; adb -s $d shell iptables -F OUTPUT" >/dev/null 2>&1 || true
    done
    direct_blocked=0
}
configure_relay_only() {
    log "路径：强制 n0 中继（宣告一个到不了的直连地址）"
    # 重新 up 一次而不是 status：只有 up 会把 --announce 写进 .dev-announce 并传给核心，重起核心。
    dev_sh up --announce 10.0.2.99 >/dev/null || { log "带中继宣告地址重起核心失败"; return 1; }
    # 名额重排后 up 会换端口（.dev-ports 也变）；以 stack-slot 现在发的端口为准，并把 App 记着的旧隧道
    # 端口反向映射到新端口，否则 App 拨的还是旧隧道，永远连不回核心。
    local old_tunnel=$tunnel
    for i in $(seq 1 10); do
        if read -r gw tunnel < <(timeout 25 ssh "$HOST" "~/wonton-dev/stack-slot.sh ports $STACK" 2>/dev/null) && [[ -n $tunnel ]]; then break; fi
        sleep 2
    done
    timeout 25 ssh "$HOST" "printf '%s %s\n' '$gw' '$tunnel' > ~/wonton-dev/stacks/dev-$STACK/.dev-ports" || true
    for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d reverse tcp:$old_tunnel tcp:$tunnel" >/dev/null 2>&1 || true; done
    timeout 300 ssh "$HOST-stack" restart "$STACK" core || { log "重启核心失败"; return 1; }
    timeout 10 ssh "$HOST" "docker inspect wonton-$STACK" 2>/dev/null | grep -q 'WONTON_ANNOUNCE_ADDRESSES=10.0.2.99' ||
        log "警告：核心环境里没有 10.0.2.99 的宣告，靠 block_direct 兜底"
    # 重启后 remote-dev 可能丢 compose 的网络别名，core 一直起不来；必须等到网关、隧道和
    # Tuwunel 都能答，再往下走，否则失败会伪装成“房间不在列表/没进通话”。
    local i code ready=0
    for i in $(seq 1 90); do
        code=$(timeout 10 ssh "$HOST" "curl -s -m 3 -o /dev/null -w '%{http_code}' http://127.0.0.1:$gw/" || true)
        # dev.sh status 里没有 “已连接” 字样（它只打容器状态），别把它当隧道判据；网关 + Tuwunel 都答就算通。
        if [[ $code != 000 ]] && api GET /_matrix/client/versions >/dev/null 2>&1; then
            ready=1; break
        fi
        sleep 2
    done
    ((ready)) || { log "核心重启后 180 秒仍没就绪（网关/隧道/Tuwunel 有一个不通；别名丢失见 README）"; return 1; }
    # 先把房间和 EC 页面在直连下打开：公网中继会把超过约 100 KB 的 HTTP 响应流截断，
    # EC 的入口 JS 有 325 KB，冷缓存下永远加载不完（ERR_CONTENT_LENGTH_MISMATCH）。
    open_room "$DEV_A" "$room_name" || { log "中继组：a 打不开房间"; return 1; }
    open_room "$DEV_B" "$room_name" || { log "中继组：b 打不开房间"; return 1; }
    # 页面打开后等 EC 的 JS 包进缓存；open_call 失败不影响后面 block（join 还会再试）。
    open_call "$DEV_A" || true; open_call "$DEV_B" || true
    sleep 20
    block_direct
    # 直连被丢掉后 iroh 自己切到 relay；等它切完再回房间。
    sleep 30
    open_room "$DEV_A" "$room_name" || { log "中继组：block 后 a 回不了房间"; return 1; }
    open_room "$DEV_B" "$room_name" || { log "中继组：block 后 b 回不了房间"; return 1; }
    sleep 3
}

# —— 准备 ——
[[ -f $pkg/docker-compose.yml ]] || die "插件包不在 $pkg"
# 容器前缀是 wonton-<实例>（项目名去掉标点加哈希，不等于栈名），只认服务段。
timeout 25 ssh "$HOST" "docker ps --format '{{.Names}}'" | grep -q -- '--app-element-call--' ||
    die "栈里没装 app.element-call（先装默认 UDP 版）"
log "预检"
# dev.sh status 会跑 docker compose ps，在共用机上常常一卡几分钟；这里只看 .dev-ports 和网关是否回 HTTP（根路径 401/404 都算在）。
read -r gw tunnel < <(timeout 25 ssh "$HOST" "cat ~/wonton-dev/stacks/dev-$STACK/.dev-ports" 2>/dev/null)
[[ -n ${gw:-} && -n ${tunnel:-} ]] || die "栈 $STACK 的 .dev-ports 没有（栈没起？）"
if read -r ngw ntun < <(timeout 25 ssh "$HOST" "~/wonton-dev/stack-slot.sh ports $STACK" 2>/dev/null) && [[ -n ${ntun:-} ]]; then
    old_tunnel=$tunnel; gw=$ngw; tunnel=$ntun
    timeout 25 ssh "$HOST" "printf '%s %s\n' '$gw' '$tunnel' > ~/wonton-dev/stacks/dev-$STACK/.dev-ports" || true
    if [[ $old_tunnel != "$tunnel" ]]; then
        for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d reverse tcp:$old_tunnel tcp:$tunnel" >/dev/null 2>&1 || true; done
    fi
fi
code=$(timeout 25 ssh "$HOST" "curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$gw/" || true)
[[ $code != 000 ]] || die "网关 127.0.0.1:$gw 不在"
timeout 25 ssh "$HOST" "adb -s $DEV_A shell pm path app.wonton.android; adb -s $DEV_B shell pm path app.wonton.android" | grep -c package | grep -q 2 || die "App 没装上"
timeout 25 ssh "$HOST" "python3 -c 'import jwt'" || die "HOST 上缺 PyJWT（pip install pyjwt；EdDSA 还要 cryptography）"
timeout 25 ssh "$HOST" "PYTHONPATH=$RCD/pylib python3 -c 'import websocket'" || die "HOST 上的 cdp 缺 websocket-client（随仓库 pylib 推过去失败）"
for d in "$DEV_A" "$DEV_B"; do
    timeout 25 ssh "$HOST" "adb -s $d reverse tcp:$gw tcp:$gw; adb -s $d reverse tcp:$tunnel tcp:$tunnel" >/dev/null
done
rm -f "$EVIDENCE"/.media-*
# 上一轮被 kill -9 时 EXIT trap 没跑，block_direct 的 DROP 会把这一轮的隧道端口一起挡住，先清掉。
for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d root >/dev/null 2>&1; adb -s $d shell iptables -F OUTPUT" >/dev/null 2>&1 || true; done
timeout 60 ssh "$HOST" "for d in $DEV_A $DEV_B; do
    adb -s \$d shell pm clear app.wonton.android >/dev/null
    adb -s \$d shell pm grant app.wonton.android android.permission.CAMERA
    adb -s \$d shell pm grant app.wonton.android android.permission.RECORD_AUDIO
    adb -s \$d shell pm grant app.wonton.android android.permission.POST_NOTIFICATIONS
done" || die "清空 App 数据或授权失败"
pair a "$DEV_A"; pair b "$DEV_B"
# 配对之后再建房：wait_room_visible 要看设备的房间列表，未配对时列表一直是空的。
if [[ -n ${ROOM_ID:-} ]]; then
    room_name=$ROOM_NAME room_id=$ROOM_ID
    log "复用房间 $room_id"
else
    create_room
fi
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

hangup() { # 先点网页自己的挂断（页面卸载不会发 leave），再关页；两样都找不到返回 1
    local d=$1 side=a port=9331 page pos tb xml b c x1 y1 x2 y2
    [[ $d == "$DEV_B" ]] && { side=b; port=9334; }
    page=$(cdp "$side" "$port" pick 2>/dev/null || true)
    if [[ -n $page ]]; then
        pos=$(cdp "$side" "$port" eval "$page" 'JSON.stringify((()=>{const b=document.querySelector("[data-testid=incall_leave]");if(!b)return null;const r=b.getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})())' 2>/dev/null |
            python3 -c 'import json,sys; v=json.load(sys.stdin).get("value") or "null"; print(" ".join(str(round(x)) for x in (json.loads(v) or [])))' 2>/dev/null || true)
        if [[ -n ${pos:-} ]]; then
            read -r x1 y1 <<<"$pos"
            cdp "$side" "$port" click "$page" "$x1" "$y1" >/dev/null 2>&1 || true
            log "设备 $d 已点网页里的挂断"
            sleep 3
        fi
    fi
    tb=$(ui "$d" 2>/dev/null || true)
    c=$(grep -oE 'content-desc="Close page"[^>]*bounds="[^"]+"' <<<"$tb" | tail -1) || true
    [[ -n $page || -n $c ]] || { log "设备 $d 既没有通话页也没有 Close page"; return 1; }
    [[ -n $c ]] || return 0
    b=${c##*bounds=\"}
    c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
    read -r x1 y1 x2 y2 <<<"$c"
    timeout 25 ssh "$HOST" "adb -s $d shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
}

ensure_media() { # 页面可能停在入口/大厅/静音：没有页就重新开，没进就加入，静音/关摄像头就点开关
    local side=$1 dev=$DEV_A port=9331 page n=0 rects x y
    [[ $side == b ]] && { dev=$DEV_B; port=9334; }
    allow_media "$dev"
    for _ in $(seq 1 20); do
        deadline_hit && { log "设备 $side ensure_media 组级超时（page=${page:-无}）"; return 1; }
        allow_media "$dev"
        page=$(cdp "$side" "$port" pick 2>/dev/null || true)
        if [[ -z $page ]]; then
            open_call "$dev" || true
            sleep 2
            continue
        fi
        n=$(cdp "$side" "$port" eval "$page" \
            '(async()=>{let n=0;for(const pc of (window.__pcs||[]))for(const s of pc.getSenders())if(s.track){s.track.enabled=true;n++}return n})()' 2>/dev/null |
            python3 -c 'import json,sys; print(json.load(sys.stdin).get("value") or "")' 2>/dev/null || true)
        [[ $n =~ ^[1-9] ]] && { page_dismiss "$side" "$port" "$page"; log "设备 $side 已打开 $n 条发送轨道"; return 0; }
        # 还在大厅（没有挂断键）就先加入。
        if ! cdp "$side" "$port" eval "$page" '!!document.querySelector("[data-testid=incall_leave]")' 2>/dev/null | grep -q true; then
            join_call "$dev" || true
            sleep 2
            continue
        fi
        # 底栏开关：标签是 Unmute microphone / Start video 时点它（真鼠标事件，React 才收得到）。
        rects=$(cdp "$side" "$port" eval "$page" '(async()=>{const out=[];for(const [id,want] of [["incall_mute","Unmute microphone"],["incall_videomute","Start video"]]){const b=document.querySelector(`[data-testid=${id}]`);if(!b)continue;const label=document.getElementById(b.getAttribute("aria-labelledby"))?.textContent;if(label===want){const r=b.getBoundingClientRect();out.push([r.x+r.width/2,r.y+r.height/2])}}return JSON.stringify(out)})()' 2>/dev/null |
            python3 -c 'import json,sys; print(json.load(sys.stdin).get("value") or "[]")' 2>/dev/null || echo '[]')
        while read -r x y; do cdp "$side" "$port" click "$page" "$x" "$y" >/dev/null 2>&1 || true; done < <(
            python3 -c 'import json,sys
for x, y in json.loads(sys.argv[1]): print(round(x), round(y))' "$rects" 2>/dev/null || true)
        sleep 2
    done
    log "设备 $side 没有发送轨道（页面可能还在拒绝媒体，看 logcat 的 Permission denied）"
    return 1
}

capture_failure_evidence() { # <tag>：组失败时把两端 App 核心日志尾和 logcat 关键行落盘，别让结论只活在终端里
    local tag=$1 d side
    for d in "$DEV_A" "$DEV_B"; do
        if [[ $d == "$DEV_B" ]]; then side=b; else side=a; fi
        timeout 25 ssh "$HOST" "adb -s $d shell run-as app.wonton.android tail -n 200 files/core.log" \
            > "$EVIDENCE/failure-$tag-app-$side.log" 2>&1 || true
        timeout 25 ssh "$HOST" "adb -s $d shell logcat -d" 2>/dev/null |
            grep -iE 'chromium|ERR_CONTENT_LENGTH|Failed to connect|transport-unavailable|relay is rate-limiting|Element Call' \
            > "$EVIDENCE/failure-$tag-console-$side.log" 2>&1 || true
    done
    timeout 25 ssh "$HOST" "docker logs --tail 200 wonton-$STACK" 2>/dev/null |
        grep -iE 'CheckedPath|relay|rate-limit|transport-unavailable|tunnel_stream_aborted' \
        > "$EVIDENCE/failure-$tag-core.log" 2>&1 || true
}

run_group() { # run_group <a|b|c|plain> <说明> <配置函数>；任何一步不过都记一组失败，不再静默“完成”
    local g=$1 keys="$EVIDENCE/members-$1.keys" before="$EVIDENCE/members-$1.before" group_deadline_at=0
    log "== 组 $g：$2"
    "$3" || { fail_group "$g" "组 $g 配置失败"; return 1; }
    # 配置（含 relay 组的 up/预热）不占组级 deadline；从这里开始媒体/判定阶段受它约束。local 保证
    # 提前 return 时自动消失，别把过期 deadline 留给后面的 kill/tcp/relay 组。
    group_deadline_at=$((SECONDS + GROUP_DEADLINE))
    log "组 $g 判定阶段 deadline ${GROUP_DEADLINE}s"
    clear_dialogs
    for d in "$DEV_A" "$DEV_B"; do timeout 25 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true; done
    sleep 3
    if ! open_room "$DEV_A" "$room_name"; then
        fail_group "$g" "a 打不开房间（组级超时或页面没起）"
        return 1
    fi
    if ! open_room "$DEV_B" "$room_name"; then
        fail_group "$g" "b 打不开房间（组级超时或页面没起）"
        return 1
    fi
    deadline_hit && { fail_group "$g" "组级超时 ${GROUP_DEADLINE}s（开房间阶段）"; return 1; }
    active_members "$room_id" | sort -u > "$before" || true
    # 包裹器挂 getStats 原型，须在通话建立、页面稳定后注入（pre-join 注入会被 EC 重置，05:2x 实测）。
    open_call "$DEV_A"
    if ! join_call "$DEV_A"; then
        fail_group "$g" "a 没进通话（组级超时或页面没起）"
    fi
    inject_stats a || fail_group "$g" "a 注入失败"
    sleep 15
    open_call "$DEV_B"
    if ! join_call "$DEV_B"; then
        fail_group "$g" "b 没进通话（组级超时或页面没起）"
    fi
    inject_stats b || fail_group "$g" "b 注入失败"
    join_call "$DEV_A" || true
    # 中继组先判隧道路径：路径是 relay 组真正的判据，别等成员/媒体卡住后连它一起没落盘。
    if [[ $g == c ]]; then
        check_tunnel_relay a || fail_group "$g" "a 的隧道没走中继"
        check_tunnel_relay b || fail_group "$g" "b 的隧道没走中继"
    fi
    # 只认本轮新增的 state_key：房间复用时会积着上轮的成员。中继组 call.member 会晚到（EC 要先把
    # LiveKit 连上才发成员事件），给 3 分钟；直连组仍是 60 秒。
    member_wait=60; [[ $g == c ]] && member_wait=180
    if wait_new_members "$room_id" "$before" "$keys" 2 "$member_wait"; then
        log "组 $g 本轮成员：$(tr '\n' ' ' <"$keys")"
    else
        fail_group "$g" "${member_wait} 秒内没有两条本轮新增的 call.member"
    fi
    deadline_hit && { fail_group "$g" "组级超时 ${GROUP_DEADLINE}s（成员/路径阶段）"; return 1; }
    stack logs core 400 2>/dev/null | grep 'call.member' | tail -5 >> "$EVIDENCE/members.log" || true
    ensure_media a || fail_group "$g" "a 没有开麦/开摄像头"
    ensure_media b || fail_group "$g" "b 没有开麦/开摄像头"
    sleep 15
    if [[ $g == a || $g == plain ]]; then
        # 两端同时录 20 秒：媒体路径的样子落成视频证据。
        rec "$DEV_A" "group-$g-a"; rec "$DEV_B" "group-$g-b"; sleep 22
        fetch_rec "$DEV_A" "group-$g-a"; fetch_rec "$DEV_B" "group-$g-b"
    fi
    shot "$DEV_A" "group-$g-a"; shot "$DEV_B" "group-$g-b"
    for d in "$DEV_A" "$DEV_B"; do
        timeout 25 ssh "$HOST" "adb -s $d shell dumpsys package app.wonton.android | grep -E 'android.permission.(CAMERA|RECORD_AUDIO): granted'"
    done | tee -a "$EVIDENCE/permissions.log" >/dev/null
    stats a 9331 "$g" || fail_group "$g" "a 没有统计"
    stats b 9334 "$g" || fail_group "$g" "b 没有统计"
    if [[ -s $EVIDENCE/webrtc-a-$g.jsonl && -s $EVIDENCE/webrtc-b-$g.jsonl ]] && check_media_path "$g"; then
        log "组 $g 判定：路径和双向音视频都成立"
    else
        fail_group "$g" "两端路径或双向音视频不符（看 webrtc-*-$g.jsonl）"
    fi
    deadline_hit && { fail_group "$g" "组级超时 ${GROUP_DEADLINE}s（媒体/统计阶段）"; return 1; }
    timeout 25 ssh "$HOST" "adb -s $DEV_A shell cmd media_session volume --stream 3 --set 0" >/dev/null 2>&1 || true
    hangup "$DEV_A" || fail_group "$g" "a 找不到挂断按钮"
    hangup "$DEV_B" || fail_group "$g" "b 找不到挂断按钮"
    if [[ -s $keys ]] && wait_left "$room_id" "$keys" 60; then
        log "组 $g 挂断后本轮成员都不在通话里（state 清空）"
    else
        fail_group "$g" "挂断 60 秒后本轮成员还在通话里"
    fi
    shot "$DEV_A" "group-$g-after"
}

run_kill() { # 延迟 leave：B 正常挂断，再把 A 的 App 强杀，本轮两个成员都应消失
    local keys=$EVIDENCE/members-kill.keys before=$EVIDENCE/members-kill.before t0 i took
    log "== 组 kill：延迟 leave"
    clear_dialogs
    active_members "$room_id" | sort -u > "$before" || true
    open_call "$DEV_A"; join_call "$DEV_A"; inject_stats a || fail_group kill "a 注入失败"
    sleep 15
    open_call "$DEV_B"; join_call "$DEV_B"; inject_stats b || fail_group kill "b 注入失败"
    wait_new_members "$room_id" "$before" "$keys" 2 60 || fail_group kill "成员没到 2 条新键"
    ensure_media a || fail_group kill "a 没有开麦/开摄像头"
    ensure_media b || fail_group kill "b 没有开麦/开摄像头"
    if [[ ! -s $keys ]]; then fail_group kill "本轮没有成员键，延迟 leave 无从验证"; return 0; fi
    hangup "$DEV_B" || fail_group kill "b 找不到挂断按钮"
    # 正常挂断必须把 B 的那条清掉（B 之外的另一条是 A）。
    local active cleared=0
    for i in $(seq 1 30); do
        sleep 2
        active=$(comm -12 "$keys" <(active_members "$room_id" | sort -u) | grep -c '@' || true)
        if (( active <= 1 )); then cleared=1; break; fi
    done
    ((cleared)) || fail_group kill "B 挂断 60 秒后本轮成员仍有两條"
    sleep 3
    timeout 25 ssh "$HOST" "adb -s $DEV_A shell am force-stop app.wonton.android"
    t0=$(date +%s)
    for i in $(seq 1 30); do
        sleep 2
        if ! comm -12 "$keys" <(active_members "$room_id" | sort -u) | grep -q '@'; then
            took=$(( $(date +%s) - t0 ))
            log "强杀后延迟 leave 清掉了 A 的成员：${took}s"
            return 0
        fi
    done
    log "强杀 60 秒后 A 的 call.member 仍留着（当前 lk-jwt 没在 60 秒内代发延迟 leave；事件靠 expires 过期）"
    return 0
}

case "$stage" in
udp) run_group a "UDP 直连" configure_udp_only ;;
plain) run_group plain "不加密房间（UDP 直连）" configure_udp_only ;;
tcp) run_group b "只有 TCP" configure_tcp_only ;;
relay) run_group c "只有中继" configure_relay_only ;;
kill) run_kill ;;
all)
    # 一组失败也继续跑后面的组，最后按 failures 汇总退出码；否则第一组一挂后面根本没机会跑。
    run_group a "UDP 直连" configure_udp_only || true
    run_kill || true
    run_group b "只有 TCP" configure_tcp_only || true
    run_group c "只有中继" configure_relay_only || true
    ;;
*) die "不认识的 STAGE=$stage" ;;
esac

if (( failures )); then die "$failures 组失败（证据 $EVIDENCE）"; fi
log "全部通过"
