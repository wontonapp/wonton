#!/usr/bin/env bash
# 通话冒烟：必需环境变量、默认值和证据目录；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2034 # 这两个变量给同一轮 source 的别的库用，单文件分析看不见赋值。

: "${STACK:?}" "${HOST:?}" "${DEV_A:?}" "${DEV_B:?}" "${EVIDENCE:?}"
stage="${STAGE:-all}"
pkg="${PKG:-deploy/dev/calls/plugin}"
# HOST 是本机时（栈和模拟器都在本机）不走 ssh：照旧写 `ssh "$HOST" …`，执行换成本地 bash，
# 好让同一份脚本在本机和远端栈上都能跑。
local_host=0
case $HOST in localhost|127.0.0.1|::1|"$(hostname)"|"$(hostname -f 2>/dev/null)") local_host=1 ;; esac

mkdir -p "$EVIDENCE"
chmod 700 "$EVIDENCE"

mask_relays() { sed -E 's#(://)[^@/]+@#\1***@#g' <<<"$1"; }
relays=${RELAYS:-}
relay_host=${RELAY_HOST:-}
if [[ -z $relay_host && -n $relays ]]; then
    # 取第一条真正的中继地址：跳过 n0/no-n0 和 !删除标记，别把 "n0" 当主机。
    for item in ${relays//[,;]/ }; do
        case $item in n0|no-n0|'!'*) continue ;; esac
        relay_host=$(sed -E 's#^[A-Za-z]+://##; s#^[^@]*@##; s#[:/\#].*$##' <<<"$item")
        [[ -n $relay_host ]] && break
    done
fi
# 中继组的连续性判定：20 秒里每 2 秒一拍，每拍都要有已连接的候选对，收音频速率不低于这个值。
relay_min_kbps=${RELAY_MIN_KBPS:-4}
