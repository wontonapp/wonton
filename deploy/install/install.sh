#!/bin/sh
# 云吞的一行安装：在装好 Docker 的 Linux 主机上用 root 执行，例如 `curl -fsSL <地址>/install.sh | sudo sh`。
#
# 再执行一次就是升级或修复：拉核心镜像的最新版本，密钥和数据都沿用，compose 文件按新的核心镜像重写。可调的环境变量：
#
#   WONTON_CORE_IMAGE   要装的核心镜像（默认 ghcr.io/geraldpeng6/wonton-core:beta，也可以写别的 tag 或 digest）
#   WONTON_INSTALL_DIR  compose 文件、.env 和密钥（默认 /opt/wonton）
#   WONTON_DATA_DIR     核心和插件的数据（默认 /var/lib/wonton）
#   WONTON_PORT         局域网网页和网关的 TCP 端口（默认 8480）
#   WONTON_TUNNEL_PORT  隧道的 UDP 端口（默认 8482）
#   WONTON_BASE_DOMAIN  基础域名，同时是 Matrix 的服务器名，装好后不能改（默认 wonton.localhost）
#   WONTON_OWNER_NAME   主人的用户名，同时是 Matrix 用户名和插件里的账号，只能用小写字母、数字、_ 和 -，以字母开头，
#                       最长 32 个字符，装好后不能改（默认 owner）
#   WONTON_OWNER_DISPLAY_NAME  主人的显示名（默认 Owner）。设成空就取消
#   WONTON_OWNER_EMAIL  主人的邮箱（默认没有；要邮箱的插件这时装不上）。设成空就取消
#   TZ                  时区，IANA 时区名，如 Asia/Shanghai（默认宿主的时区，读不到时是 UTC）
#   WONTON_LAN_ADDRESSES  插件的局域网端口只开在这些私有地址上，逗号分隔（默认不设：核心自动选，主人在 App 里改；
#                       ADR-0019）。设成空就取消
#   WONTON_EGRESS_PROXY  出网插件和核心经这个代理访问外网，http://、https:// 或 socks5://，可以带用户名和密码，特殊字符写成
#                       百分号编码。测不通时安装失败。默认不设：第一次安装时检测一次，主人在 App 里改（ADR-0023）。设成空就取消
#   WONTON_PULL=never   不拉核心镜像，用本机已有的（测试用）
#
# 第一次装好后，安装目录里的 .env 记下这些设置，以后再执行时不用再给。
set -eu

die() {
  echo "安装失败：$*" >&2
  exit 1
}

info() {
  echo "==> $*"
}

[ "$(id -u)" = 0 ] || die "要用 root 执行（sudo sh）：要装进 /opt 和 /var/lib，并管理 Docker"

case "$(uname -m)" in
  x86_64 | amd64 | aarch64 | arm64) ;;
  *) die "不支持的处理器架构 $(uname -m)，只支持 amd64 和 arm64" ;;
esac

if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}" in
    debian | ubuntu) ;;
    *) echo "提示：只在 Debian 和 Ubuntu 上验证过，当前是 ${PRETTY_NAME:-未知系统}" ;;
  esac
fi

command -v docker >/dev/null 2>&1 ||
  die "没有找到 Docker。先按 https://docs.docker.com/engine/install/ 装好 Docker Engine 28 或更新的版本"
docker_version=$(docker version --format '{{.Server.Version}}' 2>/dev/null) ||
  die "连不上 Docker 守护进程，它在运行吗？"
case "${docker_version%%.*}" in
  '' | *[!0-9]*) die "认不出 Docker 的版本号 $docker_version" ;;
esac
[ "${docker_version%%.*}" -ge 28 ] || die "Docker Engine 要 28 或更新的版本，当前是 $docker_version"
docker compose version >/dev/null 2>&1 || die "没有 docker compose 插件（Docker 官方仓库的 docker-compose-plugin）"

install_dir=${WONTON_INSTALL_DIR:-/opt/wonton}
env_file="$install_dir/.env"

# 上次安装记下的设置。环境变量里给了的以环境变量为准。
saved() {
  [ -f "$env_file" ] || return 0
  sed -n "s/^$1=//p" "$env_file" | tail -n 1
}

image=${WONTON_CORE_IMAGE:-$(saved WONTON_CORE_SOURCE)}
# 1.0 之前默认装 beta 渠道：内测期只发预发布版本，stable 渠道在 1.0 才有第一个版本。
image=${image:-ghcr.io/geraldpeng6/wonton-core:beta}
data_dir=${WONTON_DATA_DIR:-$(saved WONTON_DATA_DIR)}
data_dir=${data_dir:-/var/lib/wonton}
port=${WONTON_PORT:-$(saved WONTON_PORT)}
port=${port:-8480}
tunnel_port=${WONTON_TUNNEL_PORT:-$(saved WONTON_TUNNEL_PORT)}
tunnel_port=${tunnel_port:-8482}
domain=${WONTON_BASE_DOMAIN:-$(saved WONTON_BASE_DOMAIN)}
domain=${domain:-wonton.localhost}
# 没给也没记下时留空：核心用数据目录里记下的，第一次启动时是 owner。
owner_name=${WONTON_OWNER_NAME:-$(saved WONTON_OWNER_NAME)}
# 不用 `:-`：环境变量设成空就是取消上次的设置。
lan_addresses=${WONTON_LAN_ADDRESSES-$(saved WONTON_LAN_ADDRESSES)}
owner_display_name=${WONTON_OWNER_DISPLAY_NAME-$(saved WONTON_OWNER_DISPLAY_NAME)}
owner_email=${WONTON_OWNER_EMAIL-$(saved WONTON_OWNER_EMAIL)}
egress_proxy=${WONTON_EGRESS_PROXY-$(saved WONTON_EGRESS_PROXY)}
egress_detected=$(saved WONTON_EGRESS_PROXY_DETECTED)
tz=${TZ:-$(saved TZ)}
# `curl | sudo sh` 的环境里通常没有 TZ。都读不到时留空，由 compose 给 UTC。
if [ -z "$tz" ]; then
  tz=$(timedatectl show --property=Timezone --value 2>/dev/null) || tz=
  [ "$tz" != n/a ] || tz=
  [ -n "$tz" ] || [ ! -f /etc/timezone ] || tz=$(sed -n 1p /etc/timezone)
  [ -z "$tz" ] || info "用宿主的时区 $tz（不对的话用 TZ=Asia/Shanghai 这样再执行一次）"
fi
case "$data_dir" in
  /*) ;;
  *) die "WONTON_DATA_DIR 要是绝对路径：$data_dir" ;;
esac
# 字符规则和数据目录里的记录由下面的 `init` 查。这一条查的是核心还没记下用户名的老安装：旧 compose 没把这个变量传进
# 核心，主机一直是 owner，.env 里手写的这一行从没生效过。下面的 `init` 会覆盖 compose，所以在这里看。
if [ -f "$install_dir/compose.yaml" ] && ! grep -q 'WONTON_OWNER_NAME:' "$install_dir/compose.yaml" &&
  [ -n "$owner_name" ] && [ "$owner_name" != owner ]; then
  die "这台主机上主人的用户名一直是 owner，这次给的是 $owner_name：旧版本没用过这个设置，用户名装好后不能改。去掉命令里的 WONTON_OWNER_NAME 和 $env_file 里的这一行；要换用户名只能用新的数据目录重新安装"
fi

mkdir -p "$install_dir" "$data_dir"
chmod 700 "$install_dir"

if [ "${WONTON_PULL:-}" = never ]; then
  docker image inspect "$image" >/dev/null 2>&1 || die "本机没有镜像 $image"
else
  info "拉取核心镜像 $image"
  docker pull "$image" >/dev/null || die "拉不到镜像 $image"
fi
# 能写成 digest 就写 digest：升级、回滚都按它（ADR-0012）。本机构建的镜像没有 digest，照原样写。
pinned=$(docker image inspect --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' "$image")
pinned=${pinned:-$image}

info "生成密钥和配置（已有的沿用）"
# 带上主人的设置：核心启动时也查，但那时容器会一直重启，要等满 10 分钟才报错，这里当场说清楚。
docker run --rm -v "$install_dir:$install_dir" -v "$data_dir:$data_dir:ro" \
  -e WONTON_DATA_DIR="$data_dir" -e WONTON_OWNER_NAME="$owner_name" -e WONTON_OWNER_EMAIL="$owner_email" -e TZ="$tz" \
  "$pinned" init "$install_dir" "$domain" || die "设置不对：按上面的错误改了再执行一次"

# 下面几个函数处理出网代理（ADR-0023 的“代理地址从哪里来”）。

# 打印时不带用户名和密码。
masked_proxy() {
  printf '%s' "$1" | sed -E 's#^([a-z0-9]+://)[^/@]*@#\1***@#'
}

# 容器里的回环地址是容器自己，要换成宿主。没写协议的按 http，和 curl 一样。
proxy_for_containers() {
  case "$1" in
    *://*) proxy=$1 ;;
    *) proxy="http://$1" ;;
  esac
  printf '%s' "$proxy" |
    sed -E 's#^([a-z0-9]+://([^/@]*@)?)(127\.0\.0\.1|localhost|\[::1\])([:/]|$)#\1host.docker.internal\4#'
}

# .env 不加引号，这些字符会被 docker compose 或 sh 另作解释。
proxy_fits_env() {
  case "$1" in
    *[[:space:]\$\'\"\`\\\#]*) return 1 ;;
  esac
}

# 测试容器接在核心以后用的网络上：防火墙常按网段放行，别的网桥上测通不算数。这个网络核心沿用，标签要和核心建的一样。
ensure_egress_network() {
  docker network inspect wonton-egress >/dev/null 2>&1 ||
    docker network create --driver bridge --label wonton.managed-by=core \
      --label wonton.core.component=wonton-egress wonton-egress >/dev/null ||
    die "建不了 Docker 网络 wonton-egress"
}

# 打印一行 JSON（测通时是 null），退出码 0 是代理能用。地址经环境变量传入容器，不出现在 docker 的命令行上。
check_proxy() {
  WONTON_EGRESS_CHECK_PROXY=$1 docker run --rm --network wonton-egress \
    --add-host host.docker.internal:host-gateway --user 65534:65534 --cap-drop ALL \
    --security-opt no-new-privileges --read-only -e WONTON_EGRESS_CHECK_PROXY \
    "$pinned" egress-proxy-check </dev/null
}

check_message() {
  printf '%s' "$1" | sed -n 's/.*"message":"\(.*\)"}.*/\1/p'
}

# 调用者的 Docker 客户端配置会给容器注入什么。只建容器不启动：看环境变量就够了，不用在 sh 里解析 JSON。
client_proxies() {
  config=${DOCKER_CONFIG:-}
  if [ -z "$config" ] && [ -n "${SUDO_USER:-}" ]; then
    config=$(getent passwd "$SUDO_USER" | cut -d: -f6)/.docker
  fi
  config=${config:-${HOME:-/root}/.docker}
  [ -f "$config/config.json" ] || return 0
  container=$(docker --config "$config" create "$pinned" 2>/dev/null) || return 0
  docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$container" |
    sed -n -E 's/^(HTTPS_PROXY|https_proxy|HTTP_PROXY|http_proxy)=//p'
  docker rm "$container" >/dev/null
}

proxy_candidates() {
  # `sudo -E` 时才有。
  printf '%s\n' "${HTTPS_PROXY:-}" "${https_proxy:-}" "${HTTP_PROXY:-}" "${http_proxy:-}"
  client_proxies
  docker info --format '{{.HTTPSProxy}}
{{.HTTPProxy}}' 2>/dev/null
}

# 打印第一个测得通的地址。都测不通时什么也不打印：这台主机可能本来就不需要代理。
detect_proxy() {
  tried=
  proxy_candidates | while IFS= read -r candidate; do
    [ -n "$candidate" ] || continue
    proxy=$(proxy_for_containers "$candidate")
    case " $tried " in *" $proxy "*) continue ;; esac
    tried="$tried $proxy"
    shown=$(masked_proxy "$proxy")
    if ! proxy_fits_env "$proxy"; then
      echo "提示：没用代理 $shown：地址里的特殊字符要写成百分号编码" >&2
    elif result=$(check_proxy "$proxy"); then
      message=$(check_message "$result")
      [ -z "$message" ] || echo "提示：$message" >&2
      printf '%s\n' "$proxy"
      break
    else
      echo "提示：没用代理 $shown：$(check_message "$result")" >&2
    fi
  done
}

record_detected=
[ ! -f "$env_file" ] || ! grep -q '^WONTON_EGRESS_PROXY_DETECTED=' "$env_file" || record_detected=yes
if [ -n "$egress_proxy" ]; then
  egress_proxy=$(proxy_for_containers "$egress_proxy")
  shown=$(masked_proxy "$egress_proxy")
  proxy_fits_env "$egress_proxy" || die "WONTON_EGRESS_PROXY 里的特殊字符要写成百分号编码：$shown"
  ensure_egress_network
  info "测试代理 $shown"
  result=$(check_proxy "$egress_proxy") ||
    die "WONTON_EGRESS_PROXY 用不了：$(check_message "$result")。改好了再执行一次，设成空就不用代理"
  message=$(check_message "$result")
  [ -z "$message" ] || echo "提示：$message"
elif [ -z "$record_detected" ]; then
  # 只检测一次：之后主人可能在 App 里改过，也可能就是不要代理。
  info "检测这台主机的代理"
  ensure_egress_network
  egress_detected=$(detect_proxy)
  record_detected=yes
  if [ -n "$egress_detected" ]; then
    info "出网插件和核心经代理 $(masked_proxy "$egress_detected") 访问外网（不要代理的话在 App 的设置里清空）"
  fi
fi

umask 077
cat >"$env_file" <<EOF
# 由 install.sh 写出。改端口、换镜像后再执行一次 install.sh；基础域名装好后不能改。
# 升级时拉 WONTON_CORE_SOURCE 的最新版本，compose 用按 digest 写下的 WONTON_CORE_IMAGE。
WONTON_CORE_SOURCE=$image
WONTON_CORE_IMAGE=$pinned
WONTON_INSTALL_DIR=$install_dir
WONTON_DATA_DIR=$data_dir
WONTON_PORT=$port
WONTON_TUNNEL_PORT=$tunnel_port
WONTON_BASE_DOMAIN=$domain
WONTON_OWNER_NAME=$owner_name
WONTON_OWNER_DISPLAY_NAME=$owner_display_name
WONTON_OWNER_EMAIL=$owner_email
TZ=$tz
WONTON_LAN_ADDRESSES=$lan_addresses
WONTON_EGRESS_PROXY=$egress_proxy
EOF
# 主人指定了地址时没有检测过，以后取消了还要检测，所以不留这一行。
if [ -n "$record_detected" ]; then
  echo "WONTON_EGRESS_PROXY_DETECTED=$egress_detected" >>"$env_file"
fi

# docker compose 先看环境变量再看 .env：调用时带着的 WONTON_* 会盖过上面按 digest 写下的镜像。
unset WONTON_CORE_IMAGE WONTON_INSTALL_DIR WONTON_DATA_DIR WONTON_PORT WONTON_TUNNEL_PORT WONTON_BASE_DOMAIN \
  WONTON_OWNER_NAME WONTON_OWNER_DISPLAY_NAME WONTON_OWNER_EMAIL TZ WONTON_LAN_ADDRESSES \
  WONTON_EGRESS_PROXY WONTON_EGRESS_PROXY_DETECTED
info "启动核心、身份提供方和 Matrix 服务器（第一次要拉镜像，几分钟）"
docker compose -f "$install_dir/compose.yaml" up -d --wait --wait-timeout 600 ||
  die "服务没有在 10 分钟内就绪，看看日志：docker compose -f $install_dir/compose.yaml logs"

# 浏览器和手机在局域网里用来找到这台主机的地址：默认路由出去的那张网卡。
lan_ip=$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1)
[ -n "$lan_ip" ] || lan_ip=$(hostname -I 2>/dev/null | awk '{print $1}')

mkdir -p /usr/local/bin
ln -sf "$install_dir/wonton" /usr/local/bin/wonton

echo
info "装好了。浏览器打开 http://${lan_ip:-<这台主机的 IP>}:$port/ 看运行状态。"
# shellcheck disable=SC1111 # 中文的引号是要显示的文字
cat <<EOF
防火墙要放行 TCP $port（局域网网页）和 UDP $tunnel_port（手机连回家）。
查看日志：docker compose -f $install_dir/compose.yaml logs -f core
升级：再执行一次安装命令。
以后加设备：在已配对的 App 里点“添加设备”；设备都丢了：sudo wonton pair

EOF
# 第一台设备配上前一直显示首次配对码（过期前换新的）；已经有设备时只说明怎么加设备。
"$install_dir/wonton" pair --first ||
  echo "提示：终端里出码失败，用局域网网页上的二维码配对" >&2
