#!/usr/bin/env bash
# 照常运行参数里的命令；它被外部 SIGTERM 杀掉时（退出码 143），打出时间、runner 和进程组，好和宿主机的 auditd 记录对上。
# 2026-11-06 npchy 上按名字杀过全机的 `cargo test`，日志里看不出是谁发的。
"$@"
rc=$?
if [ "$rc" -eq 143 ]; then
  printf '::error::%s 被 SIGTERM 杀掉：%s，runner %s，进程组 %s\n' \
    "$1" "$(date '+%F %T %Z')" "${RUNNER_NAME:-?}" "$(ps -o pgid= $$ | tr -d ' ')"
fi
exit "$rc"
