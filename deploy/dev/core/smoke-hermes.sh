# Hermes 对话和用量断言的助手，只由 smoke.sh source：回复从探针输出里取，用量按本轮开始时间取增量
# （网关记 Hermes 的调用时 request_id 是空的），等不到时把最近几行打出来再失败。
# shellcheck shell=bash
# shellcheck disable=SC2154 # $magpie 由 smoke.sh 赋值

hermes_reply() {
  python3 -c 'import sys
print(sys.stdin.read().partition("第 1 轮：收到加密的回复：")[2].strip())' <"$1"
}
model_usage() { docker exec "$magpie" magpie usage --last 15m --csv; }
# <回复文件> <本轮开始的 epoch 秒> <第几轮>：用量按开始时间取增量（request_id 实测为空，靠不住）。
expect_model_reply() {
  local usage deadline=$((SECONDS + 30))
  # 用量行比回复晚一点落盘（实测回复先到、CSV 里还没有这一行）：最多等 30 秒，只重读用量，不重发对话。
  while :; do
    usage="$(model_usage)" || fail "读取 magpie 用量失败"
    python3 -c 'import csv, datetime, io, sys
since = float(sys.argv[1])
valid = [r for r in csv.DictReader(io.StringIO(sys.stdin.read()))
         if r["caller_key_name"] == "com.nousresearch.hermes/default"
         and datetime.datetime.fromisoformat(r["time"]).timestamp() >= since
         and 200 <= int(r["status"] or 0) < 300 and r["error"] == "false" and int(r["output_tokens"] or 0) > 0]
sys.exit(not valid)' "$2" <<<"$usage" && break
    ((SECONDS < deadline)) || {
      printf '   用量里这个凭据最近的行（time status error output_tokens）：\n' >&2
      python3 -c 'import csv, io, sys
rows = [r for r in csv.DictReader(io.StringIO(sys.stdin.read()))
        if r["caller_key_name"] == "com.nousresearch.hermes/default"][-3:]
for r in rows:
    print("  ", r["time"], r["status"], r["error"], r["output_tokens"])' <<<"$usage" >&2
      fail "模型没有正常回答：$(hermes_reply "$1")"
    }
    sleep 0.5
  done
  ok "第 $3 轮模型调用成功且有输出 token，用量记在 Hermes 的凭据名下"
}
expect_hermes_memory() {
  local reply
  reply="$(hermes_reply "$1")"
  # 英文提示下模型也可能答英文，两种都认。
  [[ "$reply" == *介绍* || "$reply" == *自己* || "$reply" == *我是* ||
    "$reply" == *introduce* || "$reply" == *yourself* ]] ||
    fail "第二轮没有记住第一轮的请求：$reply"
  ok "第二轮记得第一轮让它介绍自己"
}
