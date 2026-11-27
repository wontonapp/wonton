#!/usr/bin/env bash
# 通话冒烟：经 adb/CDP 驱动 App 页面（找房、入口、Join、媒体、挂断、证据截图）；由 smoke.sh 在开头 source。
# shellcheck shell=bash

# dump 常在冷启动/转场后返回空壳：循环到拿到超过 3000 字节的层级再回，拿到就够。
ui() { # <dev>：当前界面 XML。系统 ANR 弹窗（"isn't responding"）会盖住 App、吞掉后面的点按：
        # 从弹窗自己的 bounds 点 Wait 再重取，最多 3 次。
    local dev=$1 xml b c x1 y1 x2 y2 i
    for i in 1 2 3; do
        xml=$(timeout 90 ssh "$HOST" "for i in 1 2 3 4 5 6 7 8; do adb -s $dev shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && sleep 0.3 && n=\$(adb -s $dev shell \"wc -c < /sdcard/ui.xml\" | tr -d \"\\r\") && [ \"\$n\" -gt 3000 ] 2>/dev/null && adb -s $dev shell cat /sdcard/ui.xml && exit 0; sleep 5; done; exit 1")
        grep -qE "isn't responding|无响应|Keeps stopping" <<<"$xml" || { printf '%s\n' "$xml"; return 0; }
        b=$(grep -oE 'text="(Wait|等待)"[^>]*bounds="\[[0-9]+,[0-9]+\]\[[0-9]+,[0-9]+\]"' <<<"$xml" | tail -1)
        c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b##*bounds=\"}")
        if [[ $c =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
            read -r x1 y1 x2 y2 <<<"$c"
            timeout 60 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))" >/dev/null 2>&1 || true
            log "设备 $dev：系统 ANR 弹窗，点了 Wait"
        else
            timeout 60 ssh "$HOST" "adb -s $dev shell input keyevent 66" >/dev/null 2>&1 || true
        fi
        sleep 1
    done
    printf '%s\n' "$xml"
}
# App 是否在前台。只看窗口焦点，不信 dump 文本：部分系统界面（配对页的输入框焦点、
# 桌面）在 dump 里没有可认的字，靠文本判断会把它们当成 App 页面继续空转。
app_fg() { timeout 60 ssh "$HOST" "adb -s $1 shell dumpsys window 2>/dev/null | grep -q 'mCurrentFocus=.*app.wonton.android'" >/dev/null 2>&1; }
# 把页面带回聊天列表：焦点不在 App 就冷启动，在就 BACK 退出设置/备份这类页面。
to_list() { # to_list <dev>：一路清掉验证/备份页直到聊天列表出现
    local dev=$1 xml action
    for _ in $(seq 1 12); do
        xml=$(ui "$dev" 2>/dev/null || true)
        # 首次引导（Install these apps → Go to chat）和引导后自动打开的默认机器人面板（Close）
        # 先处理：只认 Sidebar/房名的 to_list 会把这些页面当成“还没到列表”，一路 BACK 退出 App。
        action=$(onboarding_action "$xml")
        if [[ -n $action ]]; then onboarding_do "$dev" "$action"; continue; fi
        if grep -qE 'content-desc="Sidebar"|text="Waiting for you"' <<<"$xml" || room_label "$xml" >/dev/null 2>&1; then return 0; fi
        if grep -qE 'text="(Skip|Skip for now|跳过|先跳过|先跳過)"' <<<"$xml"; then
            tap_text "$dev" "Skip for now" >/dev/null 2>&1 || tap_text "$dev" "Skip" >/dev/null 2>&1 ||
                tap_text "$dev" "先跳过" >/dev/null 2>&1 || tap_text "$dev" "跳过" >/dev/null 2>&1 || true
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
            timeout 60 ssh "$HOST" "adb -s $dev shell input keyevent 66" >/dev/null 2>&1 || true
        fi
        sleep 1
    done
    printf '%s\n' "$xml"
}
# 处理识别到的引导/面板状态（onboarding_action 的 install|goto|next|close）。
# install 后要等预置插件装完（按钮变成 Go to chat），失败出 Retry 就重试；最多 90 秒，不无限等。
onboarding_do() { # onboarding_do <dev> <action>
    local dev=$1 action=$2 xml i
    case $action in
    install)
        log "设备 $dev 首次引导：装预置插件"
        tap_text "$dev" "Install these apps" >/dev/null 2>&1 || true
        for i in $(seq 1 45); do
            sleep 2
            xml=$(ui "$dev" 2>/dev/null || true)
            if grep -q 'text="Go to chat"\|text="去聊天"\|text="Done"\|text="完成"' <<<"$xml"; then
                tap_text "$dev" "Go to chat" >/dev/null 2>&1 || tap_text "$dev" "Done" >/dev/null 2>&1 || true
                sleep 2; return 0
            fi
            if grep -qE 'text="(Try again|重试)"' <<<"$xml"; then
                tap_text "$dev" "Try again" >/dev/null 2>&1 || tap_text "$dev" "重试" >/dev/null 2>&1 || true
            fi
        done
        log "设备 $dev 首次引导：预置插件 90 秒没装完（还停在 Install these apps）"
        ;;
    goto) tap_text "$dev" "Go to chat" >/dev/null 2>&1 || tap_text "$dev" "去聊天" >/dev/null 2>&1 ||
        tap_text "$dev" "Done" >/dev/null 2>&1 || tap_text "$dev" "完成" >/dev/null 2>&1 || true; sleep 2 ;;
    next) tap_text "$dev" "Next" >/dev/null 2>&1 || tap_text "$dev" "下一步" >/dev/null 2>&1 || true; sleep 2 ;;
    close) tap_desc "$dev" "Close page" >/dev/null 2>&1 || tap_desc "$dev" "Close panel" >/dev/null 2>&1 ||
        tap_desc "$dev" "Close" >/dev/null 2>&1 || tap_text "$dev" "Close" >/dev/null 2>&1 || true; sleep 2 ;;
    esac
}
# 把要放进 grep -E 的字面串转义：房名里的 + ( . 等不再当正则。
ere_escape() { printf '%s\n' "$1" | sed 's/[][\\.^$*+?(){}|]/\\&/g'; }
# 点一个无障碍可见的名字：先等它出现（最多 90 秒），再按它的 bounds 中心点。
tap_name() {
    local dev=$1 text=$2 xml coords x y esc
    esc=$(ere_escape "$text")
    for _ in $(seq 1 10); do
        xml=$(ui "$dev" 2>/dev/null || true)
        # 取最后一个匹配：弹窗节点排在底页同文案之后。
        line=$(printf '%s' "$xml" | grep -oE "text=\"$esc\"[^>]*bounds=\"[^\"]+\"" | tail -1) || true
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
    timeout 60 ssh "$HOST" "adb -s $dev shell input tap $x $y"
    # 第二下只在目标还在时补：双击会把刚弹出的确认框点到背景上关掉。
    sleep 1
    ui "$dev" 2>/dev/null | grep -q "text=\"$esc\"" && timeout 60 ssh "$HOST" "adb -s $dev shell input tap $x $y"
    true
}
shot() { timeout 60 ssh "$HOST" "adb -s $1 exec-out screencap -p" > "$EVIDENCE/$2.png"; }
rec() { timeout 60 ssh "$HOST" "adb -s $1 shell screenrecord --bit-rate 4000000 --time-limit 300 /sdcard/$2.mp4" & }
fetch_rec() { # 视频经 exec-out 回本地（adb pull 会把文件写到 HOST 上，不是本机）
    timeout 60 ssh "$HOST" "adb -s $1 shell pkill -INT screenrecord" >/dev/null 2>&1 || true
    sleep 2
    timeout 60 ssh "$HOST" "adb -s $1 exec-out cat /sdcard/$2.mp4" > "$EVIDENCE/$2.mp4"
    timeout 60 ssh "$HOST" "adb -s $1 shell rm -f /sdcard/$2.mp4" >/dev/null 2>&1 || true
    [[ -s $EVIDENCE/$2.mp4 ]] || log "录屏为空：$2"
}

tap_text() { # 按精确文本点中心，不做祖先回退
    local dev=$1 text=$2 xml c b x1 y1 x2 y2 esc
    esc=$(ere_escape "$text")
    xml=$(ui "$dev" 2>/dev/null || true)
    c=$(grep -oE "text=\"$esc\"[^>]*bounds=\"[^\"]+\"" <<<"$xml" | tail -1) || true
    [[ -n $c ]] || return 1
    b=${c##*bounds=\"}
    c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
    read -r x1 y1 x2 y2 <<<"$c"
    timeout 25 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
}

tap_desc() { # 按 content-desc 点中心（关闭这类只有无障碍描述的按钮）
    local dev=$1 text=$2 xml c b x1 y1 x2 y2 esc
    esc=$(ere_escape "$text")
    xml=$(ui "$dev" 2>/dev/null || true)
    c=$(grep -oE "content-desc=\"$esc\"[^>]*bounds=\"[^\"]+\"" <<<"$xml" | tail -1) || true
    [[ -n $c ]] || return 1
    b=${c##*bounds=\"}
    c=$(sed -E 's/.*\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\].*/\1 \2 \3 \4/' <<<"${b%\"}")
    read -r x1 y1 x2 y2 <<<"$c"
    timeout 60 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
}

clear_dialogs() { # 设备验证、备份提示这类会话级弹层
    local d xml last
    for d in "$DEV_A" "$DEV_B"; do
        last=""
        for _ in 1 2 3 4 5 6 7 8; do
            xml=$(ui "$d" 2>/dev/null || true)
            # 一轮下来界面没变就收手：备份横幅这类按钮点不掉的提示会一直挂着。
            [[ $xml == "$last" ]] && break
            last=$xml
            if grep -q 'text="Reset chat backup?"' <<<"$xml"; then tap_text "$d" "Cancel" || true; sleep 1; timeout 60 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true; sleep 2; continue; fi
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
                timeout 60 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true
            elif grep -q "I don't have my other device" <<<"$xml"; then
                tap_text "$d" "I don't have my other device" >/dev/null 2>&1 || true
            elif grep -q 'Enter your recovery key' <<<"$xml"; then
                timeout 60 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true
            elif grep -q 'text="Later"' <<<"$xml"; then
                tap_text "$d" "Later" >/dev/null 2>&1 || true
            elif grep -qE 'Reset backup|can.t see past messages' <<<"$xml"; then
                # 备份页本体（含 Verify/Enter recovery key/Reset backup）与"看不到历史"页：
                # BACK 退回列表；点 Reset backup 只会弹确认框把流程拖死。
                timeout 60 ssh "$HOST" "adb -s $d shell input keyevent BACK" || true
            else
                break
            fi
            sleep 3
        done
    done
}

open_room() { # open_room <dev> <显示名片段>
    # 列表行按房间名找；房名含 - . 等字符，按字面匹配不当正则。
    local dev=$1 name=$2 xml row action b cs x1 y1 x2 y2
    # App 冷启动会回到上次的界面（可能在别的房间里，没有列表）：先退回聊天列表再找行。
    to_list "$dev" >/dev/null 2>&1 || true
    for i in $(seq 1 400); do
        deadline_hit && { log "open_room $dev 组级超时"; return 1; }
        # 列表 UI 不实时刷新：新建房间的行要等 App 重启才渲染出来（17:19 实测）。
        # 周期性重启一次，重渲染列表；房间行出现就往下走。
        if (( i % 60 == 0 )); then
            timeout 60 ssh "$HOST" "adb -s $dev shell am force-stop app.wonton.android; adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
            sleep 12
        fi
        xml=$(ui "$dev" 2>/dev/null || true)
        # 首次引导/默认机器人面板会盖住列表：先按状态推进，别在 open_room 里只滑动空转。
        action=$(onboarding_action "$xml")
        if [[ -n $action ]]; then onboarding_do "$dev" "$action"; continue; fi
        # 冷启动或房间打不开时核心/页面会盖一层弹窗，先点掉再找房名。
        if grep -q 'text="OK"' <<<"$xml"; then tap_text "$dev" "OK" >/dev/null 2>&1 || true; sleep 2; continue; fi
        if grep -q 'text="Got it"' <<<"$xml"; then tap_text "$dev" "Got it" >/dev/null 2>&1 || true; sleep 1; continue; fi
        if grep -q 'text="Allow"' <<<"$xml" && grep -qiE 'camera|microphone' <<<"$xml"; then
            tap_text "$dev" "Allow" >/dev/null 2>&1 || true; sleep 1; continue
        fi
        # 焦点掉到桌面/系统 UI 时 dump 里常常认不出字，不能只靠页面文本判断。
        if ! app_fg "$dev" && ! grep -qE 'content-desc="(Sidebar|Collapse sidebar)"' <<<"$xml"; then
            timeout 60 ssh "$HOST" "adb -s $dev shell am start -n app.wonton.android/.MainActivity" >/dev/null 2>&1 || true
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
                timeout 60 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))" >/dev/null 2>&1 || true
            else
                timeout 60 ssh "$HOST" "adb -s $dev shell input tap 983 1200" >/dev/null 2>&1 || true
            fi
            sleep 2
            continue
        fi
        # 房名在就先匹配：横幅与房名可同页共存（04:46 实测），recovery 分支先走会让
        # 匹配器永远轮不到；横幅留给进房后处理。
        row=$(room_label "$xml") || true
        [[ -n $row ]] && break
        if grep -qE 'Verify with another device|Enter (your )?recovery key|This device.c?an.t see past messages yet|text="Chat backup"' <<<"$xml"; then
            to_list "$dev"; continue
        fi
        if (( i % 15 == 0 )); then log "open_room i=$i len=${#xml} fg=$(app_fg "$dev" && echo fg || echo bg)"; fi
        if grep -q 'text="End call"' <<<"$xml" || grep -q 'content-desc="Close page"' <<<"$xml"; then
            timeout 60 ssh "$HOST" "adb -s $dev shell input keyevent BACK" || true
            sleep 2
            continue
        fi
        if grep -q 'text="Later"' <<<"$xml"; then tap_text "$dev" "Later" >/dev/null 2>&1 || true; sleep 2; continue; fi
        if grep -q "I don't have my other device" <<<"$xml"; then tap_text "$dev" "I don't have my other device" >/dev/null 2>&1 || true; sleep 2; continue; fi
        # 验证向导只退不进：开始验证会引入需要人守的多步流程，冒烟一律 BACK。
        if grep -q 'text="Start verification"' <<<"$xml"; then to_list "$dev"; continue; fi
        if grep -q 'text="Element Call |' <<<"$xml"; then return 0; fi
        row=$(room_label "$xml") || true
        [[ -n $row ]] && break
        # 列表不滚太狠：每 3 轮才向下试一屏，每 12 轮回顶部，免得把已经露出来的新房间刷过去。
        if (( i % 12 == 0 )); then
            for _ in 1 2 3; do timeout 60 ssh "$HOST" "adb -s $dev shell input swipe 250 700 250 1700 200" >/dev/null 2>&1 || true; done
        elif (( i % 3 == 0 )); then
            timeout 60 ssh "$HOST" "adb -s $dev shell input swipe 250 1400 250 700 300" >/dev/null 2>&1 || true
        fi
        sleep 2
    done
    if [[ -z ${row:-} ]]; then
        # 死前留档：最后一次 dump 原文与前台状态，别再盲猜循环里看到的是什么。
        xml=$(ui "$dev" 2>/dev/null || true)
        { echo "== $(date +%T) $dev fg=$(ssh "$HOST" "adb -s $dev shell dumpsys window 2>/dev/null | grep -o 'mCurrentFocus=[^ ]*'" | tr -d '\r')"; printf '%s\n' "$xml" | head -40; } >> "$EVIDENCE/openroom-debug.log" 2>&1 || true
        die "房间 $name 不在设备 $dev 的列表里"
    fi
    tap_name "$dev" "$row" || true
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
    local dev=$1 port=9331 label
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
            if label=$(room_label "$xml"); then
                tap_name "$dev" "$label" >/dev/null 2>&1 || true
                sleep 3
                continue
            fi
            coords="1000 207 1000 207" # 顶栏座标兜底：列表也没有时的最后手段
        fi
        if [[ $coords =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+$ ]]; then
            read -r x1 y1 x2 y2 <<<"$coords"
            timeout 60 ssh "$HOST" "adb -s $dev shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
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
            # EC 的介绍层是原生层：touch 在上面经常不生效，DPAD 中间键才稳；tap 只兜底。
            timeout 60 ssh "$HOST" "adb -s $dev shell input keyevent 23" >/dev/null 2>&1 || true
            sleep 1
            xml=$(ui "$dev" 2>/dev/null || true)
            grep -q 'text="Got it"' <<<"$xml" && tap_text "$dev" "Got it" >/dev/null 2>&1 || true
            sleep 1
            continue
        fi
        if grep -q 'text="Allow"' <<<"$xml" && grep -qiE 'camera|microphone' <<<"$xml"; then
            # 原生的媒体权限弹窗同样 touch 不可靠：DPAD 右移到 Allow 再中键确定；tap 只兜底。
            timeout 60 ssh "$HOST" "adb -s $dev shell input keyevent 22; adb -s $dev shell input keyevent 23" >/dev/null 2>&1 || true
            sleep 1
            xml=$(ui "$dev" 2>/dev/null || true)
            grep -q 'text="Allow"' <<<"$xml" && tap_text "$dev" "Allow" || true
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
                { read -r x; read -r y; } < <(rect_xy "$rect") || true
                if [[ -z $x || -z $y ]]; then
                    # premain 的 __joinRect 没给到时，按 DOM 里含 “Join call” 的按钮现算中点。
                    rect=$(cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" eval "$page" '(async()=>{const b=[...document.querySelectorAll("button")].find(b=>(b.textContent||"").includes("Join call"));if(!b)return null;const r=b.getBoundingClientRect();return JSON.stringify({x:r.x+r.width/2,y:r.y+r.height/2})})()') || true
                    { read -r x; read -r y; } < <(rect_xy "$rect") || true
                fi
                if [[ -n $x && -n $y ]]; then
                    cdp "$( [[ $dev == "$DEV_B" ]] && echo b || echo a )" "$port" click "$page" "$x" "$y" >/dev/null 2>&1 || true
                else
                    timeout 60 ssh "$HOST" "adb -s $dev shell input tap 540 1906" >/dev/null 2>&1 || true
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
    pids=$(timeout 60 ssh "$HOST" "adb -s $dev shell pidof app.wonton.android" | tr -d '\r' | tr '\n' ' ')
    for cand in $(timeout 60 ssh "$HOST" "adb -s $dev shell cat /proc/net/unix" | grep -o 'webview_devtools_remote_[0-9]*' | sort -u); do
        if [[ " $pids " == *" ${cand##*_} "* ]]; then sock=$cand; fi
    done
    [[ -n $sock ]] || sock="webview_devtools_remote_${pids%% *}"
    timeout 60 ssh "$HOST" "adb -s $dev forward --remove tcp:$port; adb -s $dev forward tcp:$port localabstract:$sock" >/dev/null 2>&1 || true
    printf -v q ' %q' "$@"
    # 系统 python 可能没有 websocket-client（有的机器重启后 pip 模块丢）；纯 Python 包随仓库 pylib 带上。
    timeout 90 ssh "$HOST" "cd $RCD && no_proxy=* PYTHONPATH=$RCD/pylib python3 cdp.py --port $port$q"
}
# widget 页上的可见文本（CDP 一次 eval，不碰 uiautomator）。
ptext() { cdp "$1" "$2" eval "$3" 'document.body.innerText' 2>/dev/null | tail -1; }

# 点掉页面里的 “Got it” 介绍层（在 WebView 里，走 CDP 的鼠标事件）。
rect_xy() { # <cdp eval 输出>：cdp.py 打的是 {"exception":…,"value":"<JS 返回的字符串>"}，先剥一层再取 x/y
    python3 -c 'import json,sys
try:
    v = json.loads(sys.argv[1])
    if isinstance(v, dict) and "value" in v:
        v = json.loads(v["value"])
    print(int(v["x"])); print(int(v["y"]))
except Exception:
    pass' "$1" 2>/dev/null || true
}

page_dismiss() { # page_dismiss <a|b> <端口> <页面>
    local pos x y
    pos=$(cdp "$1" "$2" eval "$3" 'JSON.stringify(window.__gotItRect ? window.__gotItRect() : null)' 2>/dev/null) || return 0
    { read -r x; read -r y; } < <(rect_xy "$pos") || true
    [[ -n $x && -n $y ]] || return 0
    cdp "$1" "$2" click "$3" "$x" "$y" >/dev/null 2>&1 || true
    sleep 2
}

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
    timeout 60 ssh "$HOST" "adb -s $d shell input tap $(( (x1 + x2) / 2 )) $(( (y1 + y2) / 2 ))"
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

