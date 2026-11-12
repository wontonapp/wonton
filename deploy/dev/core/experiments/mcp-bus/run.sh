#!/usr/bin/env bash
# 跑一遍 rmcp 最小总线：白名单、敏感工具确认、审计日志。结论见 ADR-0006“验证结果”。
#   deploy/dev/core/experiments/mcp-bus/run.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
cargo build --quiet --manifest-path "$here/Cargo.toml" --bins
bin="$here/target/debug"

cleanup() {
  kill "${pids[@]}" 2>/dev/null || true
  rm -rf "$work"
}
pids=()
trap cleanup EXIT

"$bin/photos" 127.0.0.1:8101 2>"$work/photos.log" &
pids+=($!)
sleep 1
(cd "$work" && exec "$bin/bus" http://127.0.0.1:8101/mcp 2>"$work/bus.log") &
pids+=($!)
sleep 1

guest=bot-family-guest
owner=bot-owner-assistant
echo "访客 Bot 看到的工具：$("$bin/bot" $guest list)"
echo "访客 Bot 调敏感工具：$("$bin/bot" $guest call photos__delete_photo '{"id":"p1"}')"
echo "主人助手看到的工具：$("$bin/bot" $owner list)"
echo "主人助手搜照片：$("$bin/bot" $owner call photos__search_photos '{"query":"海边"}')"

"$bin/bot" $owner call photos__delete_photo '{"id":"p1"}' >"$work/delete.out" &
call=$!
for _ in $(seq 50); do
  approval="$(grep -o "approvals/[0-9a-f]*" "$work/bus.log" | head -1 | cut -d/ -f2 || true)"
  [[ -n "$approval" ]] && break
  sleep 0.1
done
sleep 1
kill -0 "$call" 2>/dev/null && echo "删除请求已挂起，等待主人确认（$approval）"
curl -sS --max-time 10 -X POST "http://127.0.0.1:8100/approvals/$approval?approve=true" >/dev/null
wait "$call"
echo "主人确认后删除结果：$(cat "$work/delete.out")"
echo "审计日志："
cat "$work/audit.jsonl"
