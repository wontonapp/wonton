#!/usr/bin/env bash
# 通话冒烟：往 HOST 推 cdp 工具、remote-dev/stack 包装和配对直连地址；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2154 # 这些变量由别的库赋值，单文件分析看不见。

# cdp.py / premain.js 在 <主机> 上跑：随栈副本推过去（remote-dev 维护 stacks/dev-<栈>）。
ssh() {
    if (( local_host )) && [[ ${1:-} == "$HOST" ]]; then
        shift
        if (( $# == 1 )); then bash -c "$1"; else bash -c "$*"; fi
    else
        command ssh "$@"
    fi
}

RCD="\$HOME/wonton-dev/stacks/dev-$STACK/deploy/dev/calls"
timeout 60 ssh "$HOST" "true; mkdir -p $RCD"
# cdp.py 的 websocket-client 随脚本推到 HOST 的栈副本，固定 PYTHONPATH；矩阵操作还要 host 上的 PyJWT。
for f in cdp.py premain.js; do timeout 60 ssh "$HOST" "cat > $RCD/$f" < "$here/$f"; done
timeout 60 ssh "$HOST" "tar -C $RCD -xzf -" < <(tar -C "$here" -czf - pylib)


dev_sh() { "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" "$@"; }
stack() { timeout 60 ssh "$HOST-stack" "$1" "$STACK" "${@:2}"; }

stack_ports() { # 打印名额现在发的「网关 隧道」端口。登记可能挂在栈名或项目名上，两个名字都试；
    # 只认两个纯数字，读不到返回 1（主机负载高时 ssh 要 25 秒，给三次机会）。
    local n out a b i
    for i in 1 2 3; do
        for n in "$STACK" "wonton-$STACK"; do
            out=$(timeout 60 ssh "$HOST" "~/wonton-dev/stack-slot.sh ports $n" 2>&1) || continue
            read -r a b <<<"$out"
            if [[ $a =~ ^[0-9]+$ && $b =~ ^[0-9]+$ ]]; then printf '%s %s\n' "$a" "$b"; return 0; fi
        done
        sleep 2
    done
    return 1
}

refresh_ports() { # up 后名额可能换端口（.dev-ports 也变）：以 stack-slot 现在发的端口为准，重写并重做两端的 reverse
    local old_gw=$gw old_tunnel=$tunnel dev_ports slot_ports ports
    dev_ports=$(timeout 60 ssh "$HOST" "cat ~/wonton-dev/stacks/dev-$STACK/.dev-ports" 2>/dev/null || true)
    slot_ports=$(stack_ports 2>&1 || true)
    # 有占名额端口就以它为准；未占名额 / 没自动登记时 select_ports 沿用 .dev-ports 并留下原因；
    # 两处都没有才 die（点名 remote-dev up），不让空端口一路走到 curl。
    ports=$(select_ports "$dev_ports" "$slot_ports")
    read -r _ _ gw tunnel <<<"$ports"
    timeout 60 ssh "$HOST" "printf '%s %s\n' '$gw' '$tunnel' > ~/wonton-dev/stacks/dev-$STACK/.dev-ports" || log "警告：写不回 .dev-ports"
    for d in "$DEV_A" "$DEV_B"; do
        [[ $old_tunnel != "$tunnel" ]] && timeout 60 ssh "$HOST" "adb -s $d reverse tcp:$old_tunnel tcp:$tunnel" >/dev/null 2>&1 || true
        [[ $old_gw != "$gw" ]] && timeout 60 ssh "$HOST" "adb -s $d reverse tcp:$old_gw tcp:$gw" >/dev/null 2>&1 || true
    done
}
# 配对码里的直连地址要从 HOST 算：本机模拟器用 10.0.2.2（加 adb reverse），远端栈取那台的局域网 IPv4。
pair_addr=$HOST
case $HOST in localhost|127.0.0.1|::1|"$(hostname)"|"$(hostname -f 2>/dev/null)") pair_addr=10.0.2.2 ;;
*) [[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || pair_addr=$(ssh -G "$HOST" 2>/dev/null | awk '$1 == "hostname" { print $2 }') ;;
esac
[[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || die "HOST=$HOST 解析不出 IPv4 直连地址"