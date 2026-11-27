# Hermes 对话和用量断言的助手，只由 smoke.sh source：回复从探针输出里取，用量按本轮开始时间取增量
# （宿主 magpie 记 Hermes 的调用时 request_id 是空的，网关凭据的 caller_key_name 也是空的），按 agent 收口；
# 等不到时把最近几行打出来再失败。
# shellcheck shell=bash
# shellcheck disable=SC2154 # host_magpie 由 smoke.sh 赋值

hermes_reply() {
  python3 -c 'import sys
print(sys.stdin.read().partition("第 1 轮：收到明文的回复：")[2].strip())' <"$1"
}
# 用量日志：宿主 CLI 在单机布局下和服务是同一份；这里 serve 跑在自己的容器里（宿主网络），日志在容器内，
# 宿主那份停在旧文件上。宿主这份没有行时，用带这次凭据名的容器日志（先到先得），都不行再用宿主这份。
model_usage() {
  local csv candidate host_csv fallback=""
  host_csv="$(host_magpie usage --last 15m --csv 2>/dev/null)"
  [[ "$(wc -l <<<"$host_csv")" -gt 1 ]] && { printf '%s\n' "$host_csv"; return; }
  for candidate in ${WONTON_SMOKE_HOST_MAGPIE:-} $(docker ps --format '{{.Names}}' | grep -E '(^|-)magpie$'); do
    csv="$(docker exec "$candidate" magpie usage --last 15m --csv 2>/dev/null)" || continue
    [[ "$(wc -l <<<"$csv")" -gt 1 ]] || continue
    [[ -n "$fallback" ]] || fallback="$csv"
    if [[ "$csv" == *"${host_model_key_name:-}"* ]]; then printf '%s\n' "$csv"; return; fi
  done
  printf '%s\n' "${fallback:-$host_csv}"
}
# <回复文件> <本轮开始的 epoch 秒> <第几轮>：用量按开始时间取增量（request_id 实测为空，靠不住）。
expect_model_reply() {
  local usage deadline=$((SECONDS + 30))
  # 用量行比回复晚一点落盘（实测回复先到、CSV 里还没有这一行）：最多等 30 秒，只重读用量，不重发对话。
  while :; do
    usage="$(model_usage)" || fail "读取 magpie 用量失败"
    python3 -c 'import csv, datetime, io, sys
since = float(sys.argv[1])
# 凭据名留空也算：这个 magpie 版本给用网关凭据的调用不记 caller_key_name（实测只在老行里见过），
# 用 agent 再收一道——Hermes 报的是 OpenAI 的 UA，宿主上别的调用方不是这个名字。
valid = [r for r in csv.DictReader(io.StringIO(sys.stdin.read()))
         if r["caller_key_name"] in ("", "com.nousresearch.hermes/default")
         and r["agent"] == "OpenAI"
         and datetime.datetime.fromisoformat(r["time"]).timestamp() >= since
         and 200 <= int(r["status"] or 0) < 300 and r["error"] == "false" and int(r["output_tokens"] or 0) > 0]
sys.exit(not valid)' "$2" <<<"$usage" && break
    ((SECONDS < deadline)) || {
      printf '   用量里最近的 OpenAI 行（time status error output_tokens）：\n' >&2
      python3 -c 'import csv, io, sys
rows = [r for r in csv.DictReader(io.StringIO(sys.stdin.read())) if r["agent"] == "OpenAI"][-3:]
for r in rows:
    print("  ", r["time"], r["status"], r["error"], r["output_tokens"])' <<<"$usage" >&2
      fail "模型没有正常回答：$(hermes_reply "$1")"
    }
    sleep 0.5
  done
  grep -qE "$no_model_hint" <<<"$(hermes_reply "$1")" &&
    fail "配上模型后 Hermes 还在提示去设置模型：$(hermes_reply "$1")"
  ok "第 $3 轮模型调用成功且有输出 token，用量里能查到这次调用"
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
