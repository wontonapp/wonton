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
# 自建中继：RELAYS 是核心的 `WONTON_RELAYS` 值（含令牌，如 `https://<令牌>@relay.808cbd.com,no-n0`）。
# 设了就写进 worktree 的 deploy/dev/core/.env、重起核心（remote-dev 的 up 会把它推到栈上），配对前断言
# 中继接口给的表逐项等于 RELAYS；relay 组再要求两端的隧道路径真落在 RELAY_HOST（默认从 RELAYS 取）上。
# 只看到 `relay: true` 也可能是 n0，所以断言里必须带主机。
# 实现按职责拆在 smoke.d/ 下，由本文件按依赖顺序 source；函数和变量共用同一个 shell。
set -Eeuo pipefail
shopt -s inherit_errexit

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=deploy/dev/calls/smoke.d/env.sh
source "$here/smoke.d/env.sh"
# shellcheck source=deploy/dev/calls/smoke.d/evidence.sh
source "$here/smoke.d/evidence.sh"
# shellcheck source=deploy/dev/calls/smoke.d/remote.sh
source "$here/smoke.d/remote.sh"
# shellcheck source=deploy/dev/calls/smoke.d/matrix.sh
source "$here/smoke.d/matrix.sh"
# shellcheck source=deploy/dev/calls/smoke.d/ui.sh
source "$here/smoke.d/ui.sh"
# shellcheck source=deploy/dev/calls/smoke.d/stages.sh
source "$here/smoke.d/stages.sh"
# 库都 source 完、准备段之前挂 EXIT 还原；更早的失败和原来一样还没挂上。
trap restore_defaults EXIT
trap 'printf "%s 收到 SIGTERM\n" "$(date +%T)" >> "$EVIDENCE/run.log"; exit 143' TERM
trap 'printf "%s 收到 SIGINT\n" "$(date +%T)" >> "$EVIDENCE/run.log"; exit 130' INT
# set -e 静默退出时留下行号和命令，别只看到日志停住。
trap 'printf "%s 意外退出：line %s（%s）\n" "$(date +%T)" "$LINENO" "$BASH_COMMAND" >> "$EVIDENCE/run.log"' ERR
# shellcheck source=deploy/dev/calls/smoke.d/prep.sh
source "$here/smoke.d/prep.sh"

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
