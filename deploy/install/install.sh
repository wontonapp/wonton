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
case "$data_dir" in
  /*) ;;
  *) die "WONTON_DATA_DIR 要是绝对路径：$data_dir" ;;
esac

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
docker run --rm -v "$install_dir:$install_dir" "$pinned" init "$install_dir" "$domain"

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
EOF

# docker compose 先看环境变量再看 .env：调用时带着的 WONTON_* 会盖过上面按 digest 写下的镜像。
unset WONTON_CORE_IMAGE WONTON_INSTALL_DIR WONTON_DATA_DIR WONTON_PORT WONTON_TUNNEL_PORT WONTON_BASE_DOMAIN
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
