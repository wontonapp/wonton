#!/usr/bin/env bash
# smoke.sh 的步骤名、分段运行和公用的小函数。由 smoke.sh 在开头 source，配合它的 set -euo pipefail 运行；
# 函数里用到的 $browser、$core、$credential 等变量由 smoke.sh 定，调用时才取值。
# 步骤名的顺序就是 smoke.sh 里的执行顺序；SMOKE_UNTIL、SMOKE_ONLY 的用法见 smoke.sh 开头。

smoke_steps=(preflight core install isolation unauth sso models tunnel matrix echo hermes mcp files model-page revoke whoami
  restart uninstall)

ok() { printf '   ✓ %s\n' "$*"; }
fail() {
  printf '   ✗ %s\n' "$*" >&2
  exit 1
}
expect() {
  if [[ "$2" == "$3" ]]; then ok "$1"; else fail "$1：期望 $3，实际 $2"; fi
}
json() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(json.dumps(value) if isinstance(value, (bool, dict, list)) else value)' "$1"
}

known_step() { [[ " ${smoke_steps[*]} " == *" $1 "* ]]; }

# 参数先校验，写错的步骤名在起栈之前就报出来。
[[ -z "${SMOKE_UNTIL:-}" || -z "${SMOKE_ONLY:-}" ]] || fail "SMOKE_UNTIL 和 SMOKE_ONLY 只能设一个"
smoke_only="${SMOKE_ONLY:-}"
for smoke_name in ${SMOKE_UNTIL:-} ${smoke_only//,/ }; do
  known_step "$smoke_name" || fail "没有叫 $smoke_name 的步骤，可选：${smoke_steps[*]}"
done

# 这一步跑不跑：设了 SMOKE_ONLY 时只跑列出的。
want() { [[ -z "${SMOKE_ONLY:-}" || ",$SMOKE_ONLY," == *",$1,"* ]]; }
# 这次会不会跑到这一步（还要算上 SMOKE_UNTIL）：起栈前决定要不要先备好它需要的东西（如真实模型）。
step_index() { local i; for i in "${!smoke_steps[@]}"; do [[ "${smoke_steps[i]}" == "$1" ]] && echo "$i"; done; }
reaches() { want "$1" && { [[ -z "${SMOKE_UNTIL:-}" ]] || (($(step_index "$SMOKE_UNTIL") >= $(step_index "$1"))); }; }

# step <名字> <标题>：打印标题，这一步要跑时返回 0。SMOKE_UNTIL 那一步跑完、下一步开始时就正常退出（EXIT trap 照常清理或保留现场）。
last_step=
step() {
  local name="$1"
  shift
  known_step "$name" || fail "smoke.sh 里的步骤 $name 没有登记在 smoke-lib.sh 的 smoke_steps 里"
  if [[ -n "${SMOKE_UNTIL:-}" && "$last_step" == "$SMOKE_UNTIL" ]]; then
    printf '\n冒烟按 SMOKE_UNTIL=%s 跑到这里为止，前面的步骤都通过\n' "$SMOKE_UNTIL"
    exit 0
  fi
  last_step="$name"
  want "$name" || return 1
  step_now="$*"
  printf '\n== %s\n' "$*"
}

# 探针的 curl 统一走 smoke-curl.sh 里的 curl_in：都带超时，超时报出是哪一步（见该文件）。
# shellcheck source=deploy/dev/core/smoke-curl.sh
source "$(dirname "${BASH_SOURCE[0]}")/smoke-curl.sh"

# 浏览器：同一个 Cookie 罐，跨请求保持会话。它共享核心容器的网络命名空间，核心重启后要重建。
# 直连探针忽略 Docker CLI 注入的代理；隧道探针则清空 NO_PROXY，避免 *.localhost 绕过显式的本地代理。
start_browser() {
  docker rm -f "$browser" >/dev/null 2>&1 || true
  # 活 7200 秒：慢模型下一整套会跑一个多小时，1800 秒会在中途退出，后面的探针全部打到死容器上。
  docker run -d --name "$browser" --network "container:$core" --entrypoint sleep "$curl_image" 7200 >/dev/null
}
b() { curl_in "$browser" --noproxy '*' -sS -c /tmp/jar -b /tmp/jar "$@"; }
# 经手机的本地代理访问，和 App 一样带会话凭据（$credential）。t 像 Android WebView：http 地址直接发绝对形式的
# 请求行；tc 像 iOS WKWebView：每条连接先 CONNECT。两者各用一个 Cookie 罐，与上面的浏览器分开。
t() { curl_in "$browser" --noproxy '' -sS -c /tmp/phone-jar -b /tmp/phone-jar -x http://127.0.0.1:18008 -U "$credential" "$@"; }
tc() { curl_in "$browser" --noproxy '' -sS -c /tmp/ios-jar -b /tmp/ios-jar -p -x http://127.0.0.1:18008 -U "$credential" "$@"; }

labeled() {
  local id="$1"
  {
    docker ps -aq --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
    docker network ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
    docker volume ls -q --filter "label=wonton.plugin=$id" --filter "label=wonton.instance=$WONTON_DEV_INSTANCE"
  } | tr -d '[:space:]'
}
