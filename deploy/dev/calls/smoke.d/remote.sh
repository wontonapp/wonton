#!/usr/bin/env bash
# 通话冒烟：往 HOST 推 cdp 工具、remote-dev/stack 包装和配对直连地址；由 smoke.sh 在开头 source。
# shellcheck shell=bash
# shellcheck disable=SC2154 # 这些变量由别的库赋值，单文件分析看不见。

# cdp.py / premain.js 在 <主机> 上跑：随栈副本推过去（remote-dev 维护 stacks/dev-<栈>）。
RCD="\$HOME/wonton-dev/stacks/dev-$STACK/deploy/dev/calls"
timeout 25 ssh "$HOST" "true; mkdir -p $RCD"
# cdp.py 的 websocket-client 随脚本推到 HOST 的栈副本，固定 PYTHONPATH；矩阵操作还要 host 上的 PyJWT。
for f in cdp.py premain.js; do timeout 25 ssh "$HOST" "cat > $RCD/$f" < "$here/$f"; done
timeout 60 ssh "$HOST" "tar -C $RCD -xzf -" < <(tar -C "$here" -czf - pylib)

dev_sh() { "$HOME/wonton-dev/remote-dev.sh" --host "$HOST" "$STACK" "$@"; }
stack() { timeout 60 ssh "$HOST-stack" "$1" "$STACK" "${@:2}"; }
# 配对码里的直连地址要从 HOST 算：本机模拟器用 10.0.2.2（加 adb reverse），远端栈取那台的局域网 IPv4。
pair_addr=$HOST
case $HOST in localhost|127.0.0.1|::1|"$(hostname)"|"$(hostname -f 2>/dev/null)") pair_addr=10.0.2.2 ;;
*) [[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || pair_addr=$(ssh -G "$HOST" 2>/dev/null | awk '$1 == "hostname" { print $2 }') ;;
esac
[[ $pair_addr =~ ^[0-9]+(\.[0-9]+){3}$ ]] || die "HOST=$HOST 解析不出 IPv4 直连地址"
