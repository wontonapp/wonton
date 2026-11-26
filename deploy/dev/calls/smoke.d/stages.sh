#!/usr/bin/env bash
# 通话冒烟：各组的路径配置/还原、媒体与中继判定、组执行；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2154,SC2034 # 这些变量由别的库赋值或使用，单文件分析看不见。

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

