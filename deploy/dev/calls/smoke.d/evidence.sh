#!/usr/bin/env bash
# 通话冒烟：log/die、证据目录锁、组级 deadline 与失败计数；由 smoke.sh 在开头 source。
# shellcheck shell=bash

exec 9>"$EVIDENCE/.lock"
flock -n 9 || { echo "另一份冒烟在跑（$EVIDENCE）" >&2; exit 1; }

log() { printf '%s %s\n' "$(date +%T)" "$*" | tee -a "$EVIDENCE/run.log" >&2; }
die() { log "失败：$*"; exit 1; }

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

capture_failure_evidence() { # <tag>：组失败时把两端 App 核心日志尾和 logcat 关键行落盘，别让结论只活在终端里
    local tag=$1 d side
    for d in "$DEV_A" "$DEV_B"; do
        if [[ $d == "$DEV_B" ]]; then side=b; else side=a; fi
        timeout 60 ssh "$HOST" "adb -s $d shell run-as app.wonton.android tail -n 200 files/core.log" \
            > "$EVIDENCE/failure-$tag-app-$side.log" 2>&1 || true
        timeout 60 ssh "$HOST" "adb -s $d shell logcat -d" 2>/dev/null |
            grep -iE 'chromium|ERR_CONTENT_LENGTH|Failed to connect|transport-unavailable|relay is rate-limiting|Element Call' \
            > "$EVIDENCE/failure-$tag-console-$side.log" 2>&1 || true
    done
    timeout 60 ssh "$HOST" "docker logs --tail 200 wonton-$STACK" 2>/dev/null |
        grep -iE 'CheckedPath|relay|rate-limit|transport-unavailable|tunnel_stream_aborted' \
        > "$EVIDENCE/failure-$tag-core.log" 2>&1 || true
}
