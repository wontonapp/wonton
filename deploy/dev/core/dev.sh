#!/usr/bin/env bash
# 本地开发用的核心（插件管理器 + 网关）、身份提供方（Ory Hydra）和 Matrix 服务器（Tuwunel）。
#
#   dev.sh up                          构建核心镜像，启动核心、Hydra 和 Tuwunel，等到健康
#   dev.sh down                        停止，保留数据
#   dev.sh reset                       停止并删除 Hydra、Tuwunel 的数据和核心数据目录
#   dev.sh status | logs               查看状态 / 跟随核心日志
#   dev.sh token                       输出主人令牌
#   dev.sh install <compose 文件或插件包目录> [--yes]  预览插件和权限，确认后安装（一条命令装好插件）
#   dev.sh uninstall <插件 ID> [--purge]   卸载；--purge 同时删除插件数据
#   dev.sh list                        列出插件
#   dev.sh tunnel                      隧道入口的 endpoint ID、中继地址和直连地址
#   dev.sh devices                     已登记的设备
#   dev.sh revoke <endpoint ID>        吊销设备，立即断开它的隧道连接
#   dev.sh build <docker build 的参数>  构建镜像，带上 WONTON_DEV_BUILD_PROXY、WONTON_DEV_BUILD_JOBS 要的参数
#   dev.sh pair [直连地址]             发一个配对码：终端里出二维码，最后一行是码的文字；直连地址默认 127.0.0.1:隧道端口
#   dev.sh commit                      构建镜像用的提交号（工作区的 Cargo 源码有没提交的改动时带 -dirty）
#
# 浏览器打开 http://wonton.localhost:18480 ，用 `dev.sh token` 的输出登录。
# 环境变量：WONTON_DEV_PORT（默认 18480）、WONTON_DEV_DOMAIN（默认 wonton.localhost）、
# WONTON_DEV_TUNNEL_PORT（隧道 UDP 端口，默认 18482）、WONTON_DEV_TUNNEL_BIND（默认只给本机 127.0.0.1）、
# WONTON_DEV_DATA（数据目录，默认本目录下的 .data；DOCKER_HOST 指向远端时默认 /tmp/wonton-dev）、
# WONTON_DEV_PROJECT（compose 项目名，也是核心的容器名，默认 wonton-dev-core）、
# WONTON_DEV_INSTANCE（核心的实例名，插件的资源名和标签都带上它；默认按项目名取，默认项目没有实例名，设成空也没有）、
# WONTON_BUILD_COMMIT（编进核心、打在启动日志里的提交号，默认按当前检出算）、
# WONTON_DEV_BUILD_PROXY（构建镜像时经 Docker 主机上的这个代理下载依赖，如 npchy 本机的 http://127.0.0.1:7899）、
# WONTON_DEV_BUILD_JOBS（构建镜像时编译的并发数，和别的任务共用一台主机时用）、
# WONTON_DEV_DEFAULT_PLUGINS（设成空时核心不装默认 Agent Hermes，省得拉它 4 GB 的镜像）、
# WONTON_DEV_OWNER_PROFILE（设成 off 时不替你填资料，测还没填的情形用）、
# WONTON_DEV_LAN_ADDRESSES（插件的局域网端口只开在这些地址上，即核心的 WONTON_LAN_ADDRESSES；默认核心自动选）。
# 所有 HTTP 请求都在一个与核心共享网络的 curl 容器里发出，所以 DOCKER_HOST 指向远端主机时同样可用。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
secrets="$here/.secrets"
curl_image="curlimages/curl:8.23.0"

export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-dev-core}"
# 实例名只能有小写字母和数字，最长 16 个字符（WONTON_INSTANCE）。
default_instance() {
  [[ "$WONTON_DEV_PROJECT" == wonton-dev-core ]] && return
  local name="${WONTON_DEV_PROJECT#wonton-}"
  name="${name//[^a-z0-9]/}"
  printf '%s' "${name:0:16}"
}
export WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-$(default_instance)}"
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18480}"
export WONTON_DEV_DOMAIN="${WONTON_DEV_DOMAIN:-wonton.localhost}"
export WONTON_DEV_TUNNEL_PORT="${WONTON_DEV_TUNNEL_PORT:-18482}"
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

# 核心签发 Matrix 登录 JWT 的 Ed25519 密钥，和 Tuwunel 的注册共享密钥（ADR-0009）。私钥给核心，公钥给
# Tuwunel。生成需要 OpenSSL 3 及以上（macOS 可以 brew install openssl@3）。
ensure_matrix_secrets() {
  local key="$secrets/matrix-jwt.pem" pub="$secrets/matrix-jwt.pub.pem" shared="$secrets/matrix-shared-secret"
  mkdir -p "$secrets"
  chmod 700 "$secrets"
  if [[ ! -s "$key" || ! -s "$pub" ]]; then
    [[ "$(openssl version 2>/dev/null)" =~ ^OpenSSL\ [3-9] ]] ||
      die "生成 Matrix 签发密钥需要 OpenSSL 3 及以上（当前：$(openssl version 2>/dev/null || echo 未安装)）"
    (umask 077 && openssl genpkey -algorithm ed25519 -out "$key")
    openssl pkey -in "$key" -pubout -out "$pub"
  fi
  if [[ ! -s "$shared" ]]; then
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n' >"$shared"
    chmod 600 "$shared"
  fi
  WONTON_DEV_MATRIX_JWT_KEY="$(cat "$key")"
  WONTON_DEV_MATRIX_JWT_PUBLIC_KEY="$(cat "$pub")"
  WONTON_DEV_MATRIX_SECRET="$(cat "$shared")"
  export WONTON_DEV_MATRIX_JWT_KEY WONTON_DEV_MATRIX_JWT_PUBLIC_KEY WONTON_DEV_MATRIX_SECRET
}

compose() {
  ensure_secret
  ensure_matrix_secrets
  docker compose -f "$here/compose.yaml" "$@"
}

# docker build，加上 WONTON_DEV_BUILD_PROXY、WONTON_DEV_BUILD_JOBS 要的参数。代理只监听 Docker 主机的回环地址，
# 构建要用宿主网络；compose 把构建交给 buildx bake，传不了 --allow，所以经代理时核心镜像也由这里构建。
build_image() {
  local flags=()
  if [[ -n "${WONTON_DEV_BUILD_PROXY:-}" ]]; then
    flags+=(--network host --allow network.host)
    local var
    for var in HTTP_PROXY HTTPS_PROXY http_proxy https_proxy; do
      flags+=(--build-arg "$var=$WONTON_DEV_BUILD_PROXY")
    done
  fi
  if [[ -n "${WONTON_DEV_BUILD_JOBS:-}" ]]; then
    flags+=(--build-arg "CARGO_BUILD_JOBS=$WONTON_DEV_BUILD_JOBS")
  fi
  docker build ${flags[@]+"${flags[@]}"} "$@"
}

# 镜像里是哪份代码：当前检出的提交号；构建上下文（Cargo 工作区）里有没提交的改动时加 -dirty。
build_commit() {
  local commit
  commit="$(git -C "$repo" rev-parse --short HEAD)"
  if [[ -n "$(git -C "$repo" status --porcelain -- crates Cargo.toml Cargo.lock rust-toolchain.toml)" ]]; then
    commit="$commit-dirty"
  fi
  echo "$commit"
}

# 在与核心共享网络命名空间的容器里运行 curl：*.localhost 由 curl 解析到 127.0.0.1，即核心网关。
# Docker CLI 会注入客户端代理配置；这些内部请求必须直连，不能依赖客户端的 NO_PROXY 名单。
client() {
  docker run --rm -i --network "container:$core" "$curl_image" --noproxy '*' "$@"
}

token() {
  docker exec "$core" cat "$WONTON_DEV_DATA/core/owner-token"
}

api() {
  local method="$1" path="$2"
  shift 2
  client -sS --fail-with-body -X "$method" -H "Authorization: Bearer $(token)" "$@" "$base_url$path"
}

# 开发环境填一份资料，要邮箱的插件才用得了。
fill_dev_profile() {
  [[ "${WONTON_DEV_OWNER_PROFILE:-}" != off ]] || return 0
  local current
  current="$(api GET /_wonton/api/owner-profile)"
  [[ "$current" == *'"email":null'* ]] || return 0
  api PUT /_wonton/api/owner-profile -d "{\"display_name\":\"Owner\",\"email\":\"owner@$WONTON_DEV_DOMAIN\"}" >/dev/null
}

json_field() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(value if not isinstance(value, (dict, list)) else json.dumps(value, ensure_ascii=False))' "$1"
}

# 插件包：compose 文件原样发送；目录打成 tar 归档发送（compose 引用了包里的文件时，如 Hermes 的 config.yaml）。
package() {
  if [[ -d "$1" ]]; then
    COPYFILE_DISABLE=1 tar -C "$1" -cf - .
  else
    cat "$1"
  fi
}

package_type() {
  if [[ -d "$1" ]]; then echo application/x-tar; else echo application/yaml; fi
}

install() {
  local file="$1" yes="${2:-}" preview grants type blocked
  [[ -e "$file" ]] || die "找不到插件包：$file"
  type="$(package_type "$file")"
  preview="$(package "$file" | api POST /_wonton/api/plugins/preview -H "Content-Type: $type" --data-binary @-)" ||
    die "预览失败：$preview"
  grants="$(json_field permissions <<<"$preview" | python3 -c 'import json,sys
print(",".join(p["permission"] for p in json.load(sys.stdin)))')"
  echo "插件：$(json_field id <<<"$preview")（$(json_field title <<<"$preview")）" >&2
  echo "需要的权限：${grants:-无}" >&2
  python3 -c 'import json,sys
for notice in json.load(sys.stdin)["notices"]:
    print("提示：" + notice["text"])' <<<"$preview" >&2
  blocked="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["blocked"] or "")' <<<"$preview")"
  [[ -z "$blocked" ]] || die "这台主机上装不了：$blocked"
  if [[ "$yes" != "--yes" ]]; then
    read -r -p "确认安装？[y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]] || die "已取消"
  fi
  package "$file" | api POST "/_wonton/api/plugins?grant=$grants" -H "Content-Type: $type" --data-binary @-
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
    export WONTON_BUILD_COMMIT="${WONTON_BUILD_COMMIT:-$(build_commit)}"
    if [[ -n "${WONTON_DEV_BUILD_PROXY:-}" ]]; then
      build_image -q --target dev --build-arg WONTON_BUILD_COMMIT -t wonton-core:dev -f "$here/Dockerfile" "$repo" >/dev/null
      compose up -d --wait
    else
      compose up -d --build --wait
    fi
    fill_dev_profile
    echo "核心已就绪：$base_url（提交 $WONTON_BUILD_COMMIT，主人令牌：$0 token）"
    ;;
  commit) build_commit ;;
  build) build_image "${@:2}" ;;
  down) compose down ;;
  reset)
    compose down --volumes
    wipe_data
    ;;
  status) compose ps ;;
  logs) compose logs -f core ;;
  token) token ;;
  install) install "${2:?用法：$0 install <compose 文件或插件包目录> [--yes]}" "${3:-}" ;;
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
  tunnel)
    api GET /_wonton/api/tunnel
    echo
    ;;
  devices)
    api GET /_wonton/api/devices
    echo
    ;;
  revoke)
    api DELETE "/_wonton/api/devices/${2:?用法：$0 revoke <endpoint ID>}"
    echo
    ;;
  pair)
    # 主人发的配对码，已经有设备时也能用。局域网状态页（http://<本机 IP>:端口/）在还没有设备时另有首次配对码。
    # 直连地址要写 App 能到达的地址：同机的 iOS 模拟器用 127.0.0.1，Android 模拟器用 10.0.2.2，局域网里的手机
    # 用本机的局域网 IP（并设 WONTON_DEV_TUNNEL_BIND=0.0.0.0）。
    docker exec "$core" wonton-core pair "${2:-127.0.0.1:$WONTON_DEV_TUNNEL_PORT}"
    ;;
  *) die "用法：$0 up|down|reset|status|logs|token|build <docker build 的参数>|install <compose 文件或插件包目录> [--yes]|uninstall <插件 ID> [--purge]|list|tunnel|devices|revoke <endpoint ID>|pair [直连地址]" ;;
esac
