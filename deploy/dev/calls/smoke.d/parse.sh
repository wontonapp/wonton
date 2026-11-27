#!/usr/bin/env bash
# 通话冒烟：可离线测的纯解析函数（房名匹配、引导状态、端口选择）；由 smoke.sh 在开头 source。
# 这里只做字符串判断，不直接碰 adb/ssh，test/offline.sh 可以喂假 dump 断言。
# shellcheck shell=bash
# shellcheck disable=SC2154 # STACK/log/die 由同一轮的别的库或调用方提供。

# 在 uiautomator dump 里按当前房间名找列表行，输出该行的 text 值（字面匹配：房名里的 - .
# 不当正则）。找到返回 0，找不到返回 1。
room_label() { # room_label <xml>
    local xml=$1 line text
    [[ -n ${room_name:-} ]] || return 1
    while IFS= read -r line; do
        text=${line#*text=\"}
        text=${text%%\"*}
        [[ $text == *"$room_name"* ]] && { printf '%s\n' "$text"; return 0; }
    done < <(grep -oE 'text="[^"]*"[^>]*bounds="[^"]+"' <<<"$xml")
    return 1
}

# 首次引导/默认机器人面板的状态：install（点主按钮装预置插件）| goto | next | close。
# 引导页的关闭是图标（content-desc）；这里 text="Close" 是引导后自动打开的默认机器人面板按钮。
onboarding_action() { # onboarding_action <xml>
    local xml=$1
    grep -q 'text="Install these apps"' <<<"$xml" && { echo install; return 0; }
    grep -q 'text="Go to chat"\|text="Done"\|text="去聊天"\|text="完成"' <<<"$xml" && { echo goto; return 0; }
    grep -qE 'text="Next"|text="下一步"' <<<"$xml" && { echo next; return 0; }
    grep -q 'text="Close"' <<<"$xml" && { echo close; return 0; }
    return 0
}

# 选端口：stack-slot 的端口优先（名额重排会换），拿不到时沿用 .dev-ports（未占名额的栈）并写明原因；
# 两处都拿不到才报错。输出 "slot <旧隧道> <新网关> <新隧道>" 或 "dev <旧隧道> <网关> <隧道>"。
select_ports() { # select_ports <'.dev-ports' 内容> <stack-slot ports 输出（含错因）>
    local dev=$1 slot=$2 gw tunnel ngw ntun why
    read -r gw tunnel <<<"$(tr '\n' ' ' <<<"${dev:-}")" || true
    why=$(head -1 <<<"${slot:-}")
    if [[ ! ${gw:-} =~ ^[0-9]+$ || ! ${tunnel:-} =~ ^[0-9]+$ ]]; then
        die "栈 $STACK 既没有 .dev-ports 也没有 stack-slot 端口（${why:-未占名额}）；先 remote-dev.sh $STACK up"
    fi
    if read -r ngw ntun <<<"${slot:-}" && [[ $ngw =~ ^[0-9]+$ && $ntun =~ ^[0-9]+$ ]]; then
        printf 'slot %s %s %s\n' "$tunnel" "$ngw" "$ntun"
        return 0
    fi
    log "未占名额，使用 .dev-ports：$gw $tunnel（stack-slot：${why:-无输出}）"
    printf 'dev %s %s %s\n' "$tunnel" "$gw" "$tunnel"
}
