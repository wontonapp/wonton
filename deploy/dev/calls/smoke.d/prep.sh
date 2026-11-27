#!/usr/bin/env bash
# 通话冒烟：准备段（前置检查、配对、建房、常亮），smoke.sh 在 source 完之后执行。
# shellcheck shell=bash
# shellcheck disable=SC2154 # 这些变量由别的库赋值，单文件分析看不见。

# —— 准备 ——
[[ -f $pkg/docker-compose.yml ]] || die "插件包不在 $pkg"
# 容器前缀是 wonton-<实例>（项目名去掉标点加哈希，不等于栈名），只认服务段。
timeout 25 ssh "$HOST" "docker ps --format '{{.Names}}'" | grep -q -- '--app-element-call--' ||
    die "栈里没装 app.element-call（先装默认 UDP 版）"
log "预检"
# dev.sh status 会跑 docker compose ps，在共用机上常常一卡几分钟；这里只看 .dev-ports 和网关是否回 HTTP（根路径 401/404 都算在）。
# 名额端口优先（重排会换）；未占名额时 select_ports 沿用 .dev-ports 并留下原因。
dev_ports=$(timeout 25 ssh "$HOST" "cat ~/wonton-dev/stacks/dev-$STACK/.dev-ports" 2>/dev/null || true)
slot_ports=$(timeout 25 ssh "$HOST" "\$HOME/wonton-dev/stack-slot.sh ports $STACK" 2>&1 || true)
ports=$(select_ports "$dev_ports" "$slot_ports")
read -r port_src old_tunnel gw tunnel <<<"$ports"
if [[ $port_src == slot ]]; then
    timeout 25 ssh "$HOST" "printf '%s %s\n' '$gw' '$tunnel' > ~/wonton-dev/stacks/dev-$STACK/.dev-ports" || true
    # 名额重排换了端口：App 记着旧隧道口，补一条反向映射，配对和通话才不会拨空。
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
if [[ -n ${ROOM_ID:-} || -n ${ROOM_NAME:-} ]]; then
    room_name=$(resolve_room_reuse)
    room_id=$ROOM_ID
    log "复用房间 $room_id（$room_name）"
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

