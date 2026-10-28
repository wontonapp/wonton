#!/bin/sh
# 核心在 magpie 容器里执行本脚本，为声明了 models 的插件（或其中一个 Bot）签发、吊销网关凭据。
# 约定见 docs/plugin-spec.md 的“凭据的签发与吊销”：
#
#   credentials.sh issue <凭据名>          标准输入给出凭据的值。没有这个名字的凭据时新建（不限定模型，
#                                          主人可以在 magpie 网页里再限定）；已有时不改动。
#   credentials.sh revoke <凭据名>         删除这个名字的所有凭据；本来就没有也算成功。
#
# magpie 按 ID 操作凭据，名字可以重复，所以先从 gateway-key list 的表格里按名字找 ID。
set -eu

usage() {
  echo "用法：$0 issue <凭据名> | revoke <凭据名>" >&2
  exit 2
}

# 凭据名是插件 id 或 <插件 id>/<Bot 名>，不含空白，表格按空白分列才不会错位。
check_name() {
  case "$1" in
  '' | *[!a-z0-9./-]*)
    echo "凭据名不合法：$1" >&2
    exit 2
    ;;
  esac
}

ids() {
  magpie gateway-key list </dev/null | awk -v name="$1" 'NR > 1 && $2 == name { print $1 }'
}

issue() {
  name="$1"
  if [ -n "$(ids "$name")" ]; then
    return 0
  fi
  # 值从本脚本的标准输入直接交给 magpie，不出现在任何进程的参数里。
  magpie gateway-key add "$name" --key - >/dev/null
}

revoke() {
  for id in $(ids "$1"); do
    magpie gateway-key remove "$id" >/dev/null
  done
}

case "${1:-}" in
issue)
  [ "$#" -eq 2 ] || usage
  check_name "$2"
  issue "$2"
  ;;
revoke)
  [ "$#" -eq 2 ] || usage
  check_name "$2"
  revoke "$2"
  ;;
*) usage ;;
esac
