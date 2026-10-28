#!/usr/bin/env bash
# 本地开发用的核心（插件管理器 + 网关）和身份提供方（Ory Hydra）。
#
#   dev.sh up                          构建核心镜像，启动核心和 Hydra，等到健康
#   dev.sh down                        停止，保留数据
#   dev.sh reset                       停止并删除 Hydra 数据和核心数据目录
#   dev.sh status | logs               查看状态 / 跟随核心日志
#   dev.sh token                       输出主人令牌
#   dev.sh install <compose 文件> [--yes]  预览插件和权限，确认后安装（一条命令装好插件）
#   dev.sh uninstall <插件 ID> [--purge]   卸载；--purge 同时删除插件数据
#   dev.sh list                        列出插件
#
# 浏览器打开 http://wonton.localhost:18480 ，用 `dev.sh token` 的输出登录。
# 环境变量：WONTON_DEV_PORT（默认 18480）、WONTON_DEV_DOMAIN（默认 wonton.localhost）、
# WONTON_DEV_DATA（数据目录，默认本目录下的 .data；DOCKER_HOST 指向远端时默认 /tmp/wonton-dev）、
# WONTON_DEV_PROJECT（compose 项目名，也是核心的容器名，默认 wonton-dev-core）。
# 所有 HTTP 请求都在一个与核心共享网络的 curl 容器里发出，所以 DOCKER_HOST 指向远端主机时同样可用。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
secrets="$here/.secrets"
curl_image="curlimages/curl:8.23.0"

export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-dev-core}"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18480}"
export WONTON_DEV_DOMAIN="${WONTON_DEV_DOMAIN:-wonton.localhost}"
if [[ -z "${WONTON_DEV_DATA:-}" ]]; then
  case "${DOCKER_HOST:-}" in
    ssh://* | tcp://*) WONTON_DEV_DATA=/tmp/wonton-dev ;;
    *) WONTON_DEV_DATA="$here/.data" ;;
  esac
fi
export WONTON_DEV_DATA
core="$WONTON_DEV_PROJECT"
base_url="http://$WONTON_DEV_DOMAIN:$WONTON_DEV_PORT"

die() {
  echo "$*" >&2
  exit 1
}

ensure_secret() {
  local file="$secrets/hydra-system-secret"
  if [[ ! -s "$file" ]]; then
    mkdir -p "$secrets"
    chmod 700 "$secrets"
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n' >"$file"
    chmod 600 "$file"
  fi
  WONTON_DEV_HYDRA_SECRET="$(cat "$file")"
  export WONTON_DEV_HYDRA_SECRET
}

compose() {
  ensure_secret
  docker compose -f "$here/compose.yaml" "$@"
}

# 在与核心共享网络命名空间的容器里运行 curl：*.localhost 由 curl 解析到 127.0.0.1，即核心网关。
client() {
  docker run --rm -i --network "container:$core" "$curl_image" "$@"
}

token() {
  docker exec "$core" cat "$WONTON_DEV_DATA/core/owner-token"
}

api() {
  local method="$1" path="$2"
  shift 2
  client -sS --fail-with-body -X "$method" -H "Authorization: Bearer $(token)" "$@" "$base_url$path"
}

json_field() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(value if not isinstance(value, (dict, list)) else json.dumps(value, ensure_ascii=False))' "$1"
}

install() {
  local file="$1" yes="${2:-}" preview grants
  [[ -f "$file" ]] || die "找不到 compose 文件：$file"
  preview="$(api POST /_wonton/api/plugins/preview --data-binary @- <"$file")" ||
    die "预览失败：$preview"
  grants="$(json_field permissions <<<"$preview" | python3 -c 'import json,sys
print(",".join(p["permission"] for p in json.load(sys.stdin)))')"
  echo "插件：$(json_field id <<<"$preview")（$(json_field title.zh <<<"$preview")）" >&2
  echo "需要的权限：${grants:-无}" >&2
  if [[ "$yes" != "--yes" ]]; then
    read -r -p "确认安装？[y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]] || die "已取消"
  fi
  api POST "/_wonton/api/plugins?grant=$grants" --data-binary @- <"$file"
  echo
}

# 删除数据目录里核心创建的内容。文件可能属于 root 或插件用户，所以在容器里以 root 删除。
wipe_data() {
  docker run --rm --user 0 --entrypoint rm -v "$WONTON_DEV_DATA:/wipe" "$curl_image" \
    -rf /wipe/core /wipe/apps /wipe/shares
}

case "${1:-}" in
  up)
    case "${DOCKER_HOST:-}" in ssh://* | tcp://*) ;; *) mkdir -p "$WONTON_DEV_DATA" ;; esac
    compose up -d --build --wait
    echo "核心已就绪：$base_url（主人令牌：$0 token）"
    ;;
  down) compose down ;;
  reset)
    compose down --volumes
    wipe_data
    ;;
  status) compose ps ;;
  logs) compose logs -f core ;;
  token) token ;;
  install) install "${2:?用法：$0 install <compose 文件> [--yes]}" "${3:-}" ;;
  uninstall)
    id="${2:?用法：$0 uninstall <插件 ID> [--purge]}"
    purge=false
    [[ "${3:-}" == "--purge" ]] && purge=true
    api DELETE "/_wonton/api/plugins/$id?purge=$purge"
    echo
    ;;
  list)
    api GET /_wonton/api/plugins
    echo
    ;;
  *) die "用法：$0 up|down|reset|status|logs|token|install <compose 文件> [--yes]|uninstall <插件 ID> [--purge]|list" ;;
esac
