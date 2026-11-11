#!/usr/bin/env bash
# 本地开发用的核心（插件管理器 + 网关）、身份提供方（Ory Hydra）和 Matrix 服务器（Tuwunel）。
#
#   dev.sh up                          构建核心镜像，启动核心、Hydra 和 Tuwunel，等到健康
#   dev.sh status --entry-cloud        跑 deploy/dev/entry-cloud 的双机入口验收（远端 stack-remote.sh 只转发固定子命令）
#   dev.sh down                        停止，删掉核心按插件起的容器和网络，保留数据和卷
#   dev.sh reset [--keep-tunnel-identity]
#                                      停止并删除 Hydra、Tuwunel 的数据、核心数据目录和插件卷；
#                                      给了开关时保留隧道私钥和设备登记（endpoint ID 不变，已配对的设备不用重配）
#   dev.sh clean-data                  把数据目录（WONTON_DEV_DATA，默认 .data）整个删掉：先确认栈已 down，再在一次性
#                                      容器里删（里面的文件常属 root，普通用户删不掉）。路径必须在 deploy/dev 下，
#                                      指到别处（远端 Docker 的 /tmp/wonton-dev、家目录）时拒绝；Docker 卷和密钥不动，reset 才管
#   dev.sh status | logs [<服务>]      查看状态 / 跟随日志：核心栈的服务（core、hydra 等）或插件的服务名
#   dev.sh diag                        只读诊断：插件登录用的主机名解析、出口代理设置和可达性、注入的代理变量
#   dev.sh token                       输出主人令牌
#   dev.sh install <compose 文件或插件包目录> [--yes]  预览插件和权限，确认后安装（一条命令装好插件）
#   dev.sh uninstall <插件 ID> [--purge]   卸载；--purge 同时删除插件数据
#   dev.sh list                        列出插件（一行一个：ID、状态、网页地址）
#   dev.sh open <插件 ID>              以主人身份走完核心登录和插件的会话交接、OIDC，打印网页地址和 Cookie 罐路径
#   dev.sh tunnel                      隧道入口的 endpoint ID、中继地址和直连地址
#   dev.sh devices                     已登记的设备
#   dev.sh revoke <endpoint ID>        吊销设备，立即断开它的隧道连接
#   dev.sh build <docker build 的参数>  构建镜像，带上 WONTON_DEV_BUILD_PROXY、WONTON_DEV_BUILD_JOBS 要的参数
#   dev.sh pair [直连地址]             发一个配对码：终端里出二维码，最后一行是码的文字；直连地址默认 127.0.0.1:隧道端口
#   dev.sh commit                      构建镜像用的提交号（工作区的 Cargo 源码有没提交的改动时带 -dirty）
#   dev.sh share-smoke                 编好探针和隔离冒烟用的核心镜像，跑 ADR-0051 的双云吞分享冒烟（自己占名额）
#
# 浏览器打开 http://wonton.localhost:18480 ，用 `dev.sh token` 的输出登录。
# 用 IP 当 Host 打核心 API 会一律 404 not_found：网关按 Host 分类，IP 只回 GET / 的局域网状态页（局域网防 DNS
# 重绑定，crates/wonton-core/src/gateway/lan.rs）。在核心容器外取证要带域名 Host；本机 *.localhost 本就解析到
# 127.0.0.1，直接用域名的 URL 也行：
#   curl -sS -H "Host: wonton.localhost:18480" http://127.0.0.1:18480/_wonton/api/me
# 实例名不是默认的（WONTON_DEV_PROJECT 不是 wonton-dev-core）时，域名是 <实例>.wonton.localhost，端口看 WONTON_DEV_PORT。
# 环境变量：WONTON_DEV_PORT（默认 18480）、WONTON_DEV_DOMAIN（默认 wonton.localhost）、
# WONTON_DEV_TUNNEL_PORT（隧道 UDP 端口，默认 18482）、WONTON_DEV_TUNNEL_BIND（默认只给本机 127.0.0.1）、
# WONTON_DEV_DATA（数据目录，默认本目录下的 .data；DOCKER_HOST 指向远端时默认 /tmp/wonton-dev）、
# WONTON_DEV_SECRETS（Hydra 和 Matrix 的密钥目录，默认 ~/.local/state/wonton-dev/<项目名>/secrets；按项目名放在
# 检出目录外，同项目名的各份检出共用一份；DOCKER_HOST 指向远端时也生成在本机，经环境变量传给容器）、
# WONTON_DEV_PROJECT（compose 项目名，也是核心的容器名，默认 wonton-dev-core）、
# WONTON_CORE_IMAGE_TAG（核心镜像标签，默认 dev；并行冒烟用它换成自己的名字，别互相覆盖 wonton-core:dev）、
# WONTON_DEV_INSTANCE（核心的实例名，插件的资源名和标签都带上它；默认按项目名取，去了标点时加项目名哈希
# 后缀免得和别的项目名撞，默认项目没有实例名，设成空也没有）、
# WONTON_BUILD_COMMIT（编进核心、打在启动日志里的提交号，默认按当前检出算）、
# WONTON_DEV_BUILD_PROXY（构建镜像时经 Docker 主机上的这个代理下载依赖，如那台主机上的 http://127.0.0.1:7899）、
# WONTON_DEV_BUILD_JOBS（构建镜像时编译的并发数，和别的任务共用一台主机时用）、
# WONTON_DEV_DEFAULT_PLUGINS（设成空时核心不装默认 Agent Hermes，省得拉它 4 GB 的镜像）、
# WONTON_DEV_OWNER_PROFILE（设成 off 时不替你填资料，测还没填的情形用）、
# WONTON_DEV_LAN_ADDRESSES（插件的局域网端口只开在这些地址上，即核心的 WONTON_LAN_ADDRESSES；默认核心自动选）、
# WONTON_DEV_ANNOUNCE_ADDRESSES（告诉手机去连的地址，即核心的 WONTON_ANNOUNCE_ADDRESSES，可以带前缀；设了就绑 0.0.0.0）、
# WONTON_DEV_CORE_SUBNET（核心网络用这个子网，如 10.214.1.0/24；不设由 Docker 从默认地址池分。默认池只有约 30 个网络，
# 几套开发栈共用一台 Docker 时会用光；设了就要在这套栈的整个生命里一直设着，换了值 compose 会要求重建网络）。
# 所有 HTTP 请求都在一个与核心共享网络的 curl 容器里发出，所以 DOCKER_HOST 指向远端主机时同样可用。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
curl_image="curlimages/curl:8.23.0"

export WONTON_DEV_PROJECT="${WONTON_DEV_PROJECT:-wonton-dev-core}"
# 密钥按项目名放在检出目录外：换个检出（worktree）起同一个项目名的栈，用的还是同一份，Hydra 才读得出库里的
# 签名密钥，插件的 OIDC 客户端不用重发。检出目录里旧的 .secrets/ 不再读，也不迁移。
export WONTON_DEV_SECRETS="${WONTON_DEV_SECRETS:-${XDG_STATE_HOME:-$HOME/.local/state}/wonton-dev/$WONTON_DEV_PROJECT/secrets}"
secrets="$WONTON_DEV_SECRETS"
# FNV-1a 的 32 位哈希，十六进制输出。不用 cksum：GNU 和 BSD 的 CRC 不一样，Mac 上经 DOCKER_HOST
# 起的栈要和 Linux 上算出同一个标签。只用 bash 内建，本地设 LC_ALL=C 按字节算，不随语言环境变。
fnv1a32() {
  local LC_ALL=C
  local hash=2166136261 byte i
  for ((i = 0; i < ${#1}; i++)); do
    printf -v byte '%d' "'${1:i:1}"
    hash=$(((hash ^ byte) * 16777619 & 0xffffffff))
  done
  printf '%08x' "$hash"
}

# 实例名只能有小写字母和数字，最长 16 个字符（WONTON_INSTANCE）。去掉非字母数字会丢信息（#855：
# wonton-chat-image 和 wonton-chatimage 都变成 chatimage，down 会清到对方的资源）：名字变了就在截短后
# 接上完整项目名哈希的前 5 位十六进制，两个只差标点的项目名不会再得到同一个标签。没丢字符的保持原样，
# 已有的栈标签不变。
default_instance() {
  [[ "$WONTON_DEV_PROJECT" == wonton-dev-core ]] && return
  local name="${WONTON_DEV_PROJECT#wonton-}"
  local stripped="${name//[^a-z0-9]/}"
  if [[ "$stripped" == "$name" ]]; then
    printf '%s' "${name:0:16}"
    return
  fi
  local hash
  hash="$(fnv1a32 "$WONTON_DEV_PROJECT")"
  printf '%s%s' "${stripped:0:11}" "${hash:0:5}"
}
export WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-$(default_instance)}"
# 端口没给时沿用这套栈上次起的端口（.data/dev-port）。up 时远端选的是别的端口的场景靠它，
# 不然后续子命令连不上，还得手动带环境变量。
if [[ -z "${WONTON_DEV_PORT:-}" && -f "$here/.data/dev-port" ]]; then
  WONTON_DEV_PORT="$(cat "$here/.data/dev-port")"
fi
export WONTON_DEV_PORT="${WONTON_DEV_PORT:-18480}"
default_domain() {
  [[ "$WONTON_DEV_PROJECT" == wonton-dev-core ]] && printf 'wonton.localhost' && return
  local name="${WONTON_DEV_PROJECT#wonton-}"
  name="${name//[^a-z0-9.-]/-}"
  printf '%s.wonton.localhost' "$name"
}
export WONTON_DEV_DOMAIN="${WONTON_DEV_DOMAIN:-$(default_domain)}"
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
# 核心给插件容器、网络和卷打的标签（crates/wonton-core/src/plugin/names.rs）。实例标签只有设了实例名的核心才打，
# 它只属于这套栈；插件标签是跨实例、跨安装都一样的。
label_instance=wonton.instance
label_plugin=wonton.plugin
label_managed_by=wonton.managed-by

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

# 核心签发 Matrix 登录 JWT 的 Ed25519 密钥，和 Tuwunel 的注册共享密钥（ADR-0009）。核心还要给开发栈的
# 联邦端口准备一张只用于本地测试的证书；证书不出开发栈，也不代表生产证书。
ensure_matrix_secrets() {
  local key="$secrets/matrix-jwt.pem" pub="$secrets/matrix-jwt.pub.pem" shared="$secrets/matrix-shared-secret"
  local tls_cert="$secrets/tuwunel-tls.crt" tls_key="$secrets/tuwunel-tls.key"
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
  if [[ ! -s "$tls_cert" || ! -s "$tls_key" ]]; then
    (umask 077 && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
      -subj "/CN=$WONTON_DEV_DOMAIN" \
      -addext "subjectAltName=DNS:$WONTON_DEV_DOMAIN" \
      -keyout "$tls_key" -out "$tls_cert")
    chmod 600 "$tls_cert" "$tls_key"
  fi
  WONTON_DEV_MATRIX_JWT_KEY="$(cat "$key")"
  WONTON_DEV_MATRIX_JWT_PUBLIC_KEY="$(cat "$pub")"
  WONTON_DEV_MATRIX_SECRET="$(cat "$shared")"
  export WONTON_DEV_MATRIX_JWT_KEY WONTON_DEV_MATRIX_JWT_PUBLIC_KEY WONTON_DEV_MATRIX_SECRET
}

# Tuwunel 先于核心启动，名单文件必须已经在：缺失时先写一份放行全部兜底；核心起来后按配对名单原地重写，
# 所以不在每次 compose 调用时覆盖它。代理文件核心不改，每次写。
ensure_dev_federation() {
  local path="$WONTON_DEV_DATA/fed/config.toml"
  local proxy="$WONTON_DEV_DATA/fed/proxy.toml"
  mkdir -p "${path%/*}"
  if [[ ! -s "$path" ]]; then
    cat >"$path" <<'EOF'
# 开发栈联邦名单；正式安装由核心按配对记录重写，不使用这份放行全部的配置。
allowed_remote_server_names_experimental = ['^.*$']
forbidden_remote_server_names = []
EOF
  fi
  cat >"$proxy" <<'EOF'
# 开发栈联邦出站代理；单独成文件，避免名单文件被核心原地重写时丢掉代理规则。
[global.proxy]
[[global.proxy.by_domain]]
url = "http://federation-proxy.wonton.internal:8448"
include = []
exclude = ["push.wonton.internal"]
EOF
  chmod 644 "$path" "$proxy"
}

compose() {
  ensure_secret
  ensure_matrix_secrets
  ensure_dev_federation
  local files=(-f "$here/compose.yaml")
  if [[ -n ${WONTON_DEV_CORE_SUBNET:-} ]]; then
    local override="$secrets/$WONTON_DEV_PROJECT.subnet.yaml"
    printf 'networks:\n  core:\n    ipam:\n      config:\n        - subnet: %s\n' "$WONTON_DEV_CORE_SUBNET" >"$override"
    files+=(-f "$override")
  fi
  docker compose "${files[@]}" "$@"
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

# 容器的名字前缀：有实例名时 wonton-<实例>--，默认项目没有实例名时只有 wonton-。
container_prefix() {
  if [[ -n "$WONTON_DEV_INSTANCE" ]]; then
    printf 'wonton-%s--' "$WONTON_DEV_INSTANCE"
  else
    printf 'wonton-'
  fi
}

# 容器名 <前缀><插件>--<服务>[--<Bot>] 里的服务名。插件和服务名都不含 `--`；核心容器没有分隔符。
service_of() {
  local rest="${1#"$(container_prefix)"}"
  rest="${rest#*--}"
  printf '%s\n' "${rest%%--*}"
}

# 这套栈的插件容器。核心栈自己的容器（hydra 之类）不带 `--`，默认项目里的核心容器按名字排掉。
plugin_containers() {
  local name
  docker ps -a --filter "name=^$(container_prefix)" --format '{{.Names}}' |
    while read -r name; do
      [[ "$name" != "$core" && "$name" == *--* ]] && printf '%s\n' "$name"
    done
}

# 跟着看日志：先认核心栈的 compose 服务，再认插件的服务；都没有时列出可选的。
logs() {
  local service="${1:-core}" name candidates services
  local -a names=()
  services="$(compose config --services 2>/dev/null)" || true
  if [[ -z "${1:-}" ]] || grep -qxF -- "$service" <<<"$services"; then
    compose logs -f --tail 200 "$service"
    return
  fi
  while read -r name; do
    [[ "$(service_of "$name")" == "$service" ]] && names+=("$name")
  done < <(plugin_containers) || true
  if [[ ${#names[@]} -eq 1 ]]; then
    docker logs -f --tail 200 "${names[@]}"
    return
  fi
  if [[ ${#names[@]} -gt 1 ]]; then
    # docker logs 一次只能跟一个容器：各起一个后台跟着，Ctrl-C 一起收到，等它们都结束。
    local -a pids=()
    for name in "${names[@]}"; do
      docker logs -f --tail 200 "$name" &
      pids+=("$!")
    done
    wait "${pids[@]}" || true
    return
  fi
  candidates="$( {
    printf '%s\n' "$services"
    plugin_containers | while read -r name; do service_of "$name"; done
  } | sed '/^$/d' | sort -u | paste -sd ' ' -)"
  die "没有叫 $service 的服务。可选：${candidates:-没有取到}"
}

# 在与核心共享网络命名空间的容器里运行 curl：*.localhost 由 curl 解析到 127.0.0.1，即核心网关。
# Docker CLI 会注入客户端代理配置；这些内部请求必须直连，不能依赖客户端的 NO_PROXY 名单。
client() {
  docker run --rm -i --network "container:$core" "$curl_image" --noproxy '*' "$@"
}

# open 用的 Cookie 罐：数据目录在 Docker 主机上一定存在（compose 挂着它），curl 容器挂进去写，宿主上直接
# curl -b 复用；DOCKER_HOST 指向远端时罐在远端的数据目录里。
cookie_jar() { printf '%s/open/cookies.txt' "$WONTON_DEV_DATA"; }

# -v 会自动建目录，但属主是 root：先建好并交给跑 curl 容器的用户（和 compose 的一致）。
ensure_jar_dir() {
  docker run --rm --user 0 --entrypoint sh -v "$WONTON_DEV_DATA/open:/open" "$curl_image" \
    -c "mkdir -p /open && chown ${WONTON_DEV_PUID:-1000}:${WONTON_DEV_PGID:-1000} /open && chmod 700 /open"
}

# open 的浏览器替身：与核心共享网络的 curl 容器，同一个 Cookie 罐跨请求保持会话。
browser_curl() {
  docker run --rm --network "container:$core" -v "$WONTON_DEV_DATA/open:/jar" \
    --user "${WONTON_DEV_PUID:-1000}:${WONTON_DEV_PGID:-1000}" \
    "$curl_image" --noproxy '*' -sS -c /jar/cookies.txt -b /jar/cookies.txt "$@"
}

token() {
  docker exec "$core" cat "$WONTON_DEV_DATA/core/owner-token"
}

api() {
  local method="$1" path="$2"
  shift 2
  client -sS --fail-with-body -X "$method" -H "Authorization: Bearer $(token)" "$@" "$base_url$path"
}

fill_dev_profile() {
  [[ "${WONTON_DEV_OWNER_PROFILE:-}" != off ]] || return 0
  local current
  current="$(api GET /_wonton/api/me)"
  [[ "$current" == *'"email":null'* ]] || return 0
  api PUT /_wonton/api/me -d "{\"display_name\":\"Owner\",\"email\":\"owner@$WONTON_DEV_DOMAIN\"}" >/dev/null
}

json_field() {
  python3 -c 'import json,sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(value if not isinstance(value, (dict, list)) else json.dumps(value, ensure_ascii=False))' "$1"
}

# 插件一行一个：ID、状态、网页地址（核心给的 web_url，不按 id 拼；没有网页的不显示）。degraded 的行尾带核心写的原因。
plugins_table() {
  local info
  info="$(api GET /_wonton/api/plugins)" || return 1
  python3 -c 'import json, sys
plugins = json.load(sys.stdin)["plugins"]
width = max((len(p["id"]) for p in plugins), default=0)
for p in plugins:
    line = "%-*s  %s" % (width, p["id"], p["status"]["state"])
    web = p.get("web_url") or ""
    if web:
        line += "  " + web
    reason = p["status"].get("reason") or ""
    if reason:
        line += "  " + reason
    print(line)' <<<"$info"
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
  local file="$1" yes="${2:-}" secrets="${3:-}" preview grants type blocked headers
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
  headers=(-H "Content-Type: $type")
  if [[ -n "$secrets" ]]; then
    # 值是 base64url(JSON)：查询串会进日志，密钥放头里。
    headers+=(-H "Wonton-Plugin-Secrets: $(python3 -c 'import base64,json,sys;print(base64.urlsafe_b64encode(json.dumps(json.loads(sys.argv[1])).encode()).decode().rstrip("="))' "$secrets")")
  fi
  package "$file" | api POST "/_wonton/api/plugins?grant=$grants" "${headers[@]}" --data-binary @-
  echo
}

# 填一项密钥或清空可选项（ADR-0064）。值留空是清空。
secret() {
  local id="${1:?用法：$0 secret <插件 ID> <名字> [值]}"
  local name="${2:?用法：$0 secret <插件 ID> <名字> [值]}"
  local value="${3:-}"
  api PUT "/_wonton/api/plugins/$id/secrets/$name" -d "{\"value\": \"$value\"}"
  echo
}

# 插件容器的环境变量，只列 WONTON_SECRET_*。
secret_env() {
  local id="${1:?用法：$0 secret-env <插件 ID>}"
  docker ps -q --filter "name=${id//./-}" | while read -r c; do
    docker inspect "$c" --format '{{json .Config.Env}}'
  done | tr ',' '\n' | grep -o 'WONTON_SECRET_[A-Z0-9_]*=[^"]*' | sort -u
}

# 以主人身份打开插件网页：核心登录 → 会话交接 → 插件的 OIDC 登录，全程用上面的浏览器替身走完，走到插件页面
# 返回 200 才算成。罐里的 Cookie 给 curl 和冒烟复用；本机浏览器没有这份罐，第一次打开插件会先跳到核心登录页。
open_plugin() {
  local id="${1:?用法：$0 open <插件 ID>}"
  docker inspect -f '{{.State.Running}}' "$core" >/dev/null 2>&1 || die "核心没在运行：先 $0 up"
  local list found state web_url landed
  list="$(api GET /_wonton/api/plugins)" || die "取不到插件列表"
  found="$(python3 -c 'import json, sys
p = next((q for q in json.load(sys.stdin)["plugins"] if q["id"] == sys.argv[1]), None)
if p is None:
    sys.exit(3)
print(p["status"]["state"])
print(p.get("web_url") or "")' "$id" <<<"$list")" || die "插件 $id 不存在（$0 list 看装了哪些）"
  state="${found%%$'\n'*}"
  web_url="${found#*$'\n'}"
  [[ "$state" == running ]] || die "插件 $id 没在运行（状态：$state，$0 list 看原因）"
  [[ -n "$web_url" ]] || die "插件 $id 没有网页"
  ensure_jar_dir
  local code
  code="$(browser_curl -o /dev/null -w '%{http_code}' -X POST --data-urlencode "token=$(token)" "$base_url/_wonton/login")"
  [[ "$code" == 303 ]] || die "核心登录失败（HTTP ${code:-见上面的错误}）"
  landed="$(browser_curl -L -o /dev/null -w '%{http_code} %{url_effective}' -H 'Accept: text/html' "$web_url")"
  [[ "${landed%% *}" == 200 ]] || die "打开 $web_url 失败（HTTP ${landed%% *}，落在 ${landed#* }）"
  echo "浏览器打开 $web_url；浏览器还没登录过核心的话，先在跳出的登录页输一次 $0 token 的令牌"
  echo "Cookie 罐：$(cookie_jar)（curl -b 复用）"
}

# 删除数据目录里核心和 dev.sh 创建的内容（open 的 Cookie 罐也是会话凭据）。文件可能属于 root 或插件用户，
# 所以在容器里以 root 删除。给了 keep_tunnel_identity 时先把隧道私钥和设备登记挪出 core/，删完放回：endpoint ID 由
# 私钥决定（crates/wonton-core/src/tunnel.rs），设备登记是白名单（crates/wonton-core/src/device.rs），两份都留着
# 才是原来那台服务器、原样认已配对的设备；只留私钥的话空白名单照样连不上。
wipe_data() {
  local before="" after=""
  if [[ "${1:-}" == keep_tunnel_identity ]]; then
    # $f 由容器里的 sh 展开。
    # shellcheck disable=SC2016
    before='for f in tunnel-key devices.json; do mv "/wipe/core/$f" "/wipe/$f.keep" 2>/dev/null || true; done; '
    # shellcheck disable=SC2016
    after='; for f in tunnel-key devices.json; do if [ -e "/wipe/$f.keep" ]; then mkdir -p /wipe/core && mv "/wipe/$f.keep" "/wipe/core/$f"; fi; done'
  fi
  docker run --rm --user 0 --entrypoint sh -v "$WONTON_DEV_DATA:/wipe" "$curl_image" -c "${before}rm -rf /wipe/core /wipe/apps /wipe/shares /wipe/open${after}"
}

# down 之后删掉本实例的数据目录：里面的文件常属 root（Docker 建的目录、容器写的），宿主用户删不掉，所以用一次性容器删
# （smoke-cleanup 的删法）。只在路径落在 deploy/dev 里时动手：数据目录指到检出外（远端 Docker 的 /tmp/wonton-dev、
# 手滑指到家目录）时拒绝，绝不 rm 到别处。
clean_data() {
  if [[ -n "$(docker ps -q --filter "label=com.docker.compose.project=$core")" ]] ||
    [[ -n "$WONTON_DEV_INSTANCE" && -n "$(docker ps -q --filter "label=$label_instance=$WONTON_DEV_INSTANCE")" ]]; then
    die "这套栈还有在跑的容器：先 $0 down 再 clean-data"
  fi
  local data deploy
  data="$(python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$WONTON_DEV_DATA")"
  deploy="$(python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$here/..")"
  [[ "$data" == "$deploy/"* && "$data" != "$deploy" ]] ||
    die "数据目录 $WONTON_DEV_DATA 不在 $deploy 下，clean-data 不删（远端 Docker 的数据目录在那台主机上清）"
  [[ -e "$data" ]] || { echo "数据目录 $data 不存在，无需清理"; return 0; }
  docker run --rm --user 0 --entrypoint rm -v "$(dirname "$data"):/host-data" "$curl_image" \
    -rf "/host-data/$(basename "$data")"
  [[ ! -e "$data" ]] || die "数据目录 $data 没删掉：里面可能还有容器挂着的文件"
  echo "已删掉数据目录 $data"
}

# 密钥属于这个项目，reset 连它一起删。手滑把家目录、检出目录设成 WONTON_DEV_SECRETS 时拒绝删除。
reset_secrets() {
  local dir="${secrets%/}"
  case "$dir" in
    / | "$HOME" | "$here" | "$repo") die "secrets 目录是 $dir，不敢删；用 WONTON_DEV_SECRETS 指到专门的目录" ;;
  esac
  rm -rf -- "$dir"
}

# 这套实例的插件容器、网络和卷。按核心给它们打的实例标签找：实例名只属于这套栈，名字前缀不参与判断。
instance_containers() { docker ps -aq --filter "label=$label_instance=$WONTON_DEV_INSTANCE"; }
instance_networks() { docker network ls -q --filter "label=$label_instance=$WONTON_DEV_INSTANCE"; }
instance_volumes() { docker volume ls -q --filter "label=$label_instance=$WONTON_DEV_INSTANCE"; }

# 停掉并删除这套实例的插件容器和网络（数据卷留着，reset 另删）。出口容器和一次性探测容器也带实例标签，一起清。
remove_instance_resources() {
  local id
  local -a containers=() networks=()
  while read -r id; do containers+=("$id"); done < <(instance_containers)
  if [[ ${#containers[@]} -gt 0 ]]; then
    # 先 SIGTERM 让插件自己收尾（数据库之类），停了再删；stop 超时的由 rm -f 兜住。
    docker stop "${containers[@]}" >/dev/null 2>&1 || true
    docker rm -f "${containers[@]}" >/dev/null
    echo "已停掉并删除 ${#containers[@]} 个插件容器"
  fi
  # 容器删完网络才删得掉。
  while read -r id; do networks+=("$id"); done < <(instance_networks)
  if [[ ${#networks[@]} -gt 0 ]]; then
    docker network rm "${networks[@]}" >/dev/null
    echo "已删除 ${#networks[@]} 个插件网络"
  fi
}

remove_instance_volumes() {
  local id
  local -a volumes=()
  while read -r id; do volumes+=("$id"); done < <(instance_volumes)
  if [[ ${#volumes[@]} -gt 0 ]]; then
    docker volume rm "${volumes[@]}" >/dev/null
    echo "已删除 ${#volumes[@]} 个插件卷"
  fi
}

# 默认项目没有实例名，插件资源只带插件标签、没有实例标签。正式安装的 Wonton 也是这样（一套 dev 栈和一套正式安装放不下
# 同名的插件），只凭标签分不出是谁的，所以不自动删：列出留下的资源，给出要手删的命令。
# 出口容器和它的网络没有插件标签，带的是 wonton.managed-by=core，一起列。
# 别的实例的资源带实例标签，不列：docker 的 label 过滤不能取反，逐个查标签。
leftovers() {
  local kind="$1" name wanted
  shift
  case "$kind" in
    ps)
      docker ps -a --filter "label=$label_plugin" --format '{{.Names}}'
      docker ps -a --filter "label=$label_managed_by=core" --format '{{.Names}}'
      ;;
    network)
      docker network ls --filter "label=$label_plugin" --format '{{.Name}}'
      docker network ls --filter "label=$label_managed_by=core" --format '{{.Name}}'
      ;;
    volume) docker volume ls --filter "label=$label_plugin" --format '{{.Name}}' ;;
  esac | sort -u | while read -r name; do
    if [[ "$kind" == ps ]]; then
      wanted="$(docker inspect --format "{{index .Config.Labels \"$label_instance\"}}" "$name")"
    else
      wanted="$(docker "$kind" inspect --format "{{index .Labels \"$label_instance\"}}" "$name")"
    fi
    [[ -n "$wanted" ]] || echo "$name"
  done
}

note_plugin_leftovers() {
  local name
  local -a containers=() networks=() volumes=()
  while read -r name; do containers+=("$name"); done < <(leftovers ps)
  while read -r name; do networks+=("$name"); done < <(leftovers network)
  if [[ "${1:-}" == purge ]]; then
    while read -r name; do volumes+=("$name"); done < <(leftovers volume)
  fi
  (( ${#containers[@]} + ${#networks[@]} + ${#volumes[@]} )) || return 0
  echo
  echo "默认项目没有实例名，下面这些插件资源和出口容器分不出是这套栈的还是同一台 Docker 上正式安装的，没动："
  if [[ ${#containers[@]} -gt 0 ]]; then printf '  容器 %s\n' "${containers[@]}"; fi
  if [[ ${#networks[@]} -gt 0 ]]; then printf '  网络 %s\n' "${networks[@]}"; fi
  if [[ ${#volumes[@]} -gt 0 ]]; then printf '  卷 %s\n' "${volumes[@]}"; fi
  echo "要一起删就："
  if [[ ${#containers[@]} -gt 0 ]]; then echo "  docker rm -f ${containers[*]}"; fi
  if [[ ${#networks[@]} -gt 0 ]]; then echo "  docker network rm ${networks[*]}"; fi
  if [[ ${#volumes[@]} -gt 0 ]]; then echo "  docker volume rm ${volumes[*]}"; fi
}

# shellcheck source=deploy/dev/core/diag.sh
. "$here/diag.sh"

if [[ " $* " == *" --entry-cloud "* ]]; then
  # 远端 stack-remote.sh 只转发固定子命令、且各自校验参数：带上 --entry-cloud 的子命令都跑家名/入口云栈验收。
  # --entry-cloud-bg 脱离终端跑（远端 ssh 一断就收不到输出，日志写进副本根目录的 .entry-cloud-smoke.log）。
  rest=() detach=0
  for arg in "$@"; do
    case $arg in
      --entry-cloud) ;;
      --entry-cloud-bg) detach=1 ;;
      *) rest+=("$arg") ;;
    esac
  done
  # 前面带的那个子命令只是给 stack-remote.sh 做载体，真参数从 --entry-cloud 之后算。
  [[ ${rest[0]:-} == status || ${rest[0]:-} == up || ${rest[0]:-} == logs ]] && rest=("${rest[@]:1}")
  export WONTON_CORE_BINARY="${WONTON_CORE_BINARY:-$repo/target/debug/wonton-core}"
  export WONTON_HOME_BINARY="${WONTON_HOME_BINARY:-$repo/target/debug/wonton-home}"
  if [[ ${rest[0]:-} == destroy ]]; then
    pkill -f 'deploy/dev/entry-cloud/smoke\.py' 2>/dev/null || true
    exec bash "$repo/deploy/dev/entry-cloud/dev.sh" destroy
  fi
  if [[ ${rest[0]:-} == down || ${rest[0]:-} == logs ]]; then
    exec bash "$repo/deploy/dev/entry-cloud/dev.sh" "${rest[@]}"
  fi
  if [[ ${rest[0]:-} == net ]]; then
    for h in a b; do
      echo "== $h"
      docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{println $k $v.IPAddress}}{{end}}{{range .Config.Env}}{{println .}}{{end}}' "wonton-fr-entry-$h-core" | grep -E 'wonton|WONTON_PORT|WONTON_PUBLIC_PORT|WONTON_TUNNEL_PORT|WONTON_ANNOUNCE'
    done
    net=$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{if eq $k "wonton-fr-entry-entry"}}{{$k}}{{end}}{{end}}' wonton-fr-entry-b-core)
    img='public.ecr.aws/docker/library/debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4'
    docker exec wonton-fr-entry-a-core sh -c 'ip route 2>/dev/null || cat /proc/net/route' || true
    docker run --rm --network "$net" "$img" bash -c 'exec 3<>/dev/tcp/10.214.5.31/47403; printf "GET /_wonton/api/tunnel HTTP/1.0\r\nHost: wonton.localhost\r\n\r\n" >&3; timeout 3 head -c 60 <&3; echo tcp_rc=$?' || true
    exit 0
  fi
  if [[ ${rest[0]:-} == tlsprobe ]]; then
    echo "probe roots: $(ls -d /tmp/wonton-fr-entry-*/a/install 2>/dev/null | tr '\n' ' ')"
    install=$(ls -d /tmp/wonton-fr-entry-*/a/install 2>/dev/null | head -1); root=$(dirname "$(dirname "$install")")
    name=$(cat "$root/a/install/secrets/matrix-server-name")
    relay=$(grep WONTON_ENTRY_RELAY_IP "$root/a/install/entry-input/init.env" | cut -d= -f2)
    img='public.ecr.aws/docker/library/debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4'
    echo "name=$name relay=$relay root=$root"
    docker run --rm --network "${root:+$(basename "$root")_entry}" "$img" bash -c "echo | openssl s_client -connect $relay:443 -servername $name 2>&1 | head -20" || true
    exit 0
  fi
  if [[ ${rest[0]:-} == wipe-homes ]]; then
    docker ps -aq --filter name=wonton-fr-entry-a- | xargs -r docker rm -f >/dev/null
    docker ps -aq --filter name=wonton-fr-entry-b- | xargs -r docker rm -f >/dev/null
    docker network ls -q --filter name=wonton-fr-entry-a | xargs -r docker network rm >/dev/null 2>&1 || true
    docker volume ls -q --filter name=wonton-fr-entry-a | xargs -r docker volume rm >/dev/null 2>&1 || true
    docker network ls -q --filter name=wonton-fr-entry-b | xargs -r docker network rm >/dev/null 2>&1 || true
    docker volume ls -q --filter name=wonton-fr-entry-b | xargs -r docker volume rm >/dev/null 2>&1 || true
    echo wiped
    exit 0
  fi
  if [[ ${rest[0]:-} == release-mine ]]; then
    for d in "$HOME/wonton-dev/stack-slots"/*/; do
      owner=$(cat "$d/owner" 2>/dev/null || true)
      case $owner in
        fr-entry* | wonton-fr-entry*) echo "释放 $(basename "$d")：$owner"; rm -rf "$d" ;;
      esac
    done
    exit 0
  fi
  if [[ ${rest[0]:-} == stop ]]; then
    pkill -f 'deploy/dev/entry-cloud/smoke\.py' 2>/dev/null || true
    exit 0
  fi
  if [[ ${rest[0]:-} == bg-prio ]]; then
    extra="${rest[*]:1}"; export extra
    setsid nohup bash -c 'cd "$0" && WONTON_CORE_BINARY="$1/target/debug/wonton-core" WONTON_HOME_BINARY="$1/target/debug/wonton-home" exec "$HOME/wonton-dev/stack-slot.sh" acquire --priority --project wonton-fr-entry fr-entry -- python3 -u deploy/dev/entry-cloud/smoke.py $extra' "$repo" "$repo" >"$repo/.entry-cloud-smoke.log" 2>&1 </dev/null &
    echo "已按优先排队（参数：${extra:-}），日志：$repo/.entry-cloud-smoke.log"
    exit 0
  fi
  if [[ ${rest[0]:-} == release ]]; then
    for n in fr-entry wonton-fr-entry-a wonton-fr-entry-b; do bash "$HOME/wonton-dev/stack-slot.sh" release "$n" 2>&1 || true; done
    exit 0
  fi
  if [[ ${rest[0]:-} == hlogs ]]; then
    for h in a b; do echo "===== $h core ====="; docker logs --tail 100 "wonton-fr-entry-$h-core" 2>&1; for svc in entry connector; do echo "===== $h $svc ====="; docker logs --tail 60 "wonton-fr-entry-$h-$svc-1" 2>&1; done; done; echo "===== relay ====="; docker logs --tail 40 wonton-fr-entry-iroh-1 2>&1; docker inspect -f "{{json .State.Health}}" wonton-fr-entry-iroh-1 2>&1
    exit 0
  fi
  if [[ ${rest[0]:-} == cloud-up ]]; then
    export WONTON_DEV_CORE_SUBNET="${WONTON_DEV_CORE_SUBNET:-$(bash "$HOME/wonton-dev/stack-slot.sh" subnet fr-entry)}"
    exec bash "$repo/deploy/dev/entry-cloud/dev.sh" up
  fi
  if [[ ${rest[0]:-} == debug ]]; then
    exec bash -c 'cd "$0"; set -a; . .runtime/stack.env; set +a; echo port=$WONTON_ENTRY_PEBBLE_MANAGEMENT_PORT; docker compose --env-file .runtime/stack.env -f compose.yaml -f compose.binary.yaml ps; docker inspect wonton-fr-entry-pebble-1 -f "bindings={{json .HostConfig.PortBindings}} net={{.HostConfig.NetworkMode}}"; grep -nE "_PORT=" .runtime/stack.env; ss -ltn | grep -E ":(52185|46757|36865|43367)\b" || echo no-host-listen; docker info -f "rootless={{json .SecurityOptions}} driver={{.Driver}}"; curl -s -o /dev/null -w "api=%{http_code}\n" http://127.0.0.1:$(grep WONTON_ENTRY_API_PORT .runtime/stack.env | cut -d= -f2)/ || true; ss -ltn | grep -F ":$WONTON_ENTRY_PEBBLE_MANAGEMENT_PORT" || true; curl -sv --max-time 5 --noproxy "*" --cacert .runtime/ca/control-ca.pem --resolve "pebble.$WONTON_HOME_ZONE:$WONTON_ENTRY_PEBBLE_MANAGEMENT_PORT:127.0.0.1" "https://pebble.$WONTON_HOME_ZONE:$WONTON_ENTRY_PEBBLE_MANAGEMENT_PORT/roots/0" 2>&1 | tail -20' "$repo/deploy/dev/entry-cloud"
  fi
  if [[ ${rest[0]:-} == roots ]]; then
    exec bash "$repo/deploy/dev/entry-cloud/dev.sh" roots
  fi
  if (( detach )); then
    setsid nohup python3 -u "$repo/deploy/dev/entry-cloud/smoke.py" "${rest[@]}" \
      >"$repo/.entry-cloud-smoke.log" 2>&1 </dev/null &
    echo "已脱离运行（参数：${rest[*]}），日志：$repo/.entry-cloud-smoke.log"
    exit 0
  fi
  exec python3 "$repo/deploy/dev/entry-cloud/smoke.py" "${rest[@]}"
fi

# 被 source（instance-label_test.py 测实例标签）时只拿函数定义和上面的环境，不执行子命令。
# return 0 要写全：不带参数的 return 会把左边失败的 [[ ]] 的状态带出去，source 它的脚本开着 set -e 就会退出。
[[ "${BASH_SOURCE[0]}" == "$0" ]] || return 0
case "${1:-}" in
  up)
    # 分流机的入口只放行固定的几个子命令；工作区里有标记时把 up 让给分享冒烟。
    if [[ -f "$repo/.share-smoke" ]]; then exec bash "$0" share-smoke; fi
    # 端口冲突在构建之前查出来：compose 要到绑定时才报，那时已经构建完了。
    "$here/port-preflight.sh" "$core" "$WONTON_DEV_PORT" "$WONTON_DEV_TUNNEL_PORT" || exit 1
    case "${DOCKER_HOST:-}" in ssh://* | tcp://*) ;; *) mkdir -p "$WONTON_DEV_DATA" ;; esac
    export WONTON_BUILD_COMMIT="${WONTON_BUILD_COMMIT:-$(build_commit)}"
    mkdir -p "$here/.data" && printf '%s' "$WONTON_DEV_PORT" > "$here/.data/dev-port"
    if [[ -n "${WONTON_DEV_BUILD_PROXY:-}" ]]; then
      build_image -q --target dev --build-arg WONTON_BUILD_COMMIT -t "wonton-core:${WONTON_CORE_IMAGE_TAG:-dev}" -f "$here/Dockerfile" "$repo" >/dev/null
      compose up -d --wait
    else
      compose up -d --build --wait
    fi
    fill_dev_profile
    echo "核心已就绪：$base_url（提交 $WONTON_BUILD_COMMIT，主人令牌：$0 token）"
    ;;
  commit) build_commit ;;
  build) build_image "${@:2}" ;;
  down)
    compose down
    # 核心退出时不停插件容器，compose down 也管不到它们，这里按实例标签清；默认项目分不出来，只提示。
    if [[ -n "$WONTON_DEV_INSTANCE" ]]; then
      remove_instance_resources
    else
      note_plugin_leftovers
    fi
    ;;
  reset)
    compose down --volumes
    if [[ -n "$WONTON_DEV_INSTANCE" ]]; then
      remove_instance_resources
      remove_instance_volumes
    else
      note_plugin_leftovers purge
    fi
    if [[ "${2:-}" == --keep-tunnel-identity ]]; then
      wipe_data keep_tunnel_identity
    else
      wipe_data
    fi
    reset_secrets
    echo
    if [[ "${2:-}" == --keep-tunnel-identity ]]; then
      echo "隧道身份和设备登记保留（core/tunnel-key、core/devices.json）：endpoint ID 不变，已配对的设备不用重新配对（家人设备的用户名记录随核心数据删了）。"
    else
      echo "隧道身份已换，已配对的 App 要用新码添加服务器；旧服务器的数据保留。"
    fi
    ;;
  clean-data) clean_data ;;
  status)
    compose ps
    if docker inspect -f '{{.State.Running}}' "$core" >/dev/null 2>&1; then
      plugins_table || die "取不到插件列表"
    fi
    ;;
  logs) logs "${@:2}" ;;
  diag) diag ;;
  api)
    shift
    api "$@"
    echo
    ;;
  token) token ;;
  install)
    install "${2:?用法：$0 install <compose 文件或插件包目录> [--yes] [密钥 JSON]}" "${3:-}" "${4:-}"
    ;;
  secret) secret "${@:2}" ;;
  secret-env) secret_env "${2:?用法：$0 secret-env <插件 ID>}" ;;
  uninstall)
    id="${2:?用法：$0 uninstall <插件 ID> [--purge]}"
    purge=false
    [[ "${3:-}" == "--purge" ]] && purge=true
    api DELETE "/_wonton/api/plugins/$id?purge=$purge"
    echo
    ;;
  list) plugins_table || die "取不到插件列表"; echo ;;
  open) open_plugin "${2:?用法：$0 open <插件 ID>}" ;;
  tunnel) api GET /_wonton/api/tunnel; echo ;;
  devices) api GET /_wonton/api/devices; echo ;;
  revoke)
    api DELETE "/_wonton/api/devices/${2:?用法：$0 revoke <endpoint ID>}"
    echo
    ;;
  pair)
    # 主人发的配对码，已经有设备时也能用。局域网状态页（http://<本机 IP>:端口/）在还没有设备时另有首次配对码。
    # 直连地址要写 App 能到达的地址：同机的 iOS 模拟器用 127.0.0.1，Android 模拟器用 10.0.2.2，局域网里的手机
    # 用本机的局域网 IP（并设 WONTON_DEV_TUNNEL_BIND=0.0.0.0）。
    addr="${2:-127.0.0.1:$WONTON_DEV_TUNNEL_PORT}"
    host="${addr%:*}"
    host="${host#[}"
    host="${host%]}"
    case "${DOCKER_HOST:-}" in
      ssh://* | tcp://*)
        case "$host" in
          127.0.0.1 | localhost | ::1)
            echo "注意：DOCKER_HOST 指向远端 Docker 主机，$host 是那台机器的本机地址，手机和模拟器到不了。局域网里的设备用那台主机的局域网 IP 发码，并把 WONTON_DEV_TUNNEL_BIND 设成 0.0.0.0；同机的模拟器用那台机器上的，不在这里。" >&2
            ;;
        esac
        ;;
    esac
    docker exec "$core" wonton-core pair "$addr"
    ;;
  share-smoke)
    export PATH="$HOME/.cargo/bin:$PATH"
    bin="${WONTON_SHARE_SMOKE_BIN:-$HOME/wonton-dev/share-smoke-bin}"
    [[ -n "${WONTON_SHARE_SMOKE_BIN:-}" || -x "$bin/fr_share_smoke" ]] ||
      bin="$(dirname "$repo")/share-smoke-bin"
    if [[ ! -x "$bin/fr_share_smoke" || ! -x "$bin/share-smoke-relay" ]]; then
      mkdir -p "$bin"
      ( cd "$repo" && cargo build -p wonton-core --example fr_share_smoke -p wonton-tunnel --example share-smoke-relay &&
        cp target/debug/examples/fr_share_smoke target/debug/examples/share-smoke-relay "$bin/" ) || exit 1
    fi
    image="${WONTON_SHARE_SMOKE_CORE_IMAGE:-wonton-core:share-smoke}"
    if [[ -z "$(docker image ls -q "$image")" ]]; then
      build_image --target dev -t "$image" -f "$here/Dockerfile" "$repo" >&2 || exit 1
    fi
    exec env WONTON_SHARE_SMOKE_CORE_IMAGE="$image" WONTON_SHARE_SMOKE_BIN_DIR="$bin" bash "$here/smoke-sharing.sh"
    ;;
  *) die "用法：$0 up|down|reset [--keep-tunnel-identity]|clean-data|status|logs [<服务>]|diag|token|build <docker build 的参数>|install <compose 文件或插件包目录> [--yes]|uninstall <插件 ID> [--purge]|list|open <插件 ID>|tunnel|devices|revoke <endpoint ID>|pair [直连地址]|share-smoke" ;;
esac
