# 冒烟脚本共用的 curl 探针，由使用方 source，不单独执行：每个请求都带超时，网页或接口不回时，冒烟和它占的
# 验证栈名额不跟着挂死。网页 120 秒、连接 10 秒；更慢的调用在自己的参数里给 --max-time（写在调用处），
# 报出来的也是那个数。
# shellcheck shell=bash
: "${SMOKE_WEB_MAX_TIME:=120}"
: "${SMOKE_CONNECT_TIMEOUT:=10}"
step_now=""

# 超时退出（curl 退出码 28）时报出是哪一步、哪个地址、多少秒：curl 自己只会给“curl: (28)”。
timed_out() {
  (( $1 == 28 )) || return 0
  printf '   ✗ 第「%s」步超时（上限 %s 秒）：%s\n' "${step_now:-?}" "$2" "$3" >&2
}

# 在容器 $1 里跑 curl，地址从参数里的 http(s) 地址取。--max-time 由调用方在后面给时以后者为准（curl 取最后一个）。
curl_in() {
  local where="$1" rc=0 secs="$SMOKE_WEB_MAX_TIME" i url=""
  shift
  local -a args=("$@")
  for ((i = 0; i < ${#args[@]}; i++)); do
    case "${args[$i]}" in
      --max-time) secs="${args[$((i + 1))]}" ;;
      --max-time=*) secs="${args[$i]#--max-time=}" ;;
      http://* | https://*) url="${args[$i]}" ;;
    esac
  done
  docker exec "$where" curl --max-time "$secs" --connect-timeout "$SMOKE_CONNECT_TIMEOUT" "${args[@]}" || rc=$?
  timed_out "$rc" "$secs" "$url"
  return "$rc"
}
