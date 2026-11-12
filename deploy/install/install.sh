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
#   WONTON_BASE_DOMAIN  基础域名：App 经隧道访问核心和插件网页用的主机名，装好后不能改（默认 wonton.localhost）
#   WONTON_MATRIX_SERVER_NAME  Matrix 的服务器名，只在第一次安装时有用，装好后不能改。默认随机生成一个每台不同的
#                       名字；以后要和公网的 Matrix 联邦才写成自己的域名（ADR-0009）
#   WONTON_OWNER_NAME   用户名，同时是 Matrix 用户名和插件里的账号，只能用小写字母、数字、_ 和 -，以字母开头，
#                       最长 32 个字符，装好后不能改（默认 owner）
#   TZ                  时区，IANA 时区名，如 Asia/Shanghai（默认宿主的时区，读不到时是 UTC）
#   WONTON_LAN_ADDRESSES  插件的局域网端口只开在这些私有地址上，逗号分隔（默认不设：核心自动选，主人在 App 里改；
#                       ADR-0019）。设成空就取消
#   WONTON_EGRESS_PROXY  出网插件和核心经这个代理访问外网，http://、https:// 或 socks5://，可以带用户名和密码，特殊字符写成
#                       百分号编码。测不通时安装失败。默认不设：第一次安装时检测一次，主人在 App 里改（ADR-0023）。设成空就取消
#   WONTON_PULL=never   不拉核心镜像，用本机已有的（测试用）
#   WONTON_CORE_PIN     手动退回到这个 digest（sha256:…），之后照样跟渠道（ADR-0025）。有当时升级留下的备份就恢复核心
#                       数据和安装目录
#   WONTON_RESTORE_VOLUMES=1  退回不可逆的版本时，同时恢复备份里的卷
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

# Docker 报错里有拨号、超时、重置、DNS 或代理拨号一类的词，才算连不上镜像仓库；认证、清单不存在、磁盘满这些另说，
# 不要全都猜成代理问题。
looks_like_registry_outage() {
  grep -qiE 'dial tcp|connection refused|network is (unreachable|down)|i/o timeout|connection reset|no such host|server misbehaving|temporary failure|context deadline exceeded|TLS handshake timeout|proxyconnect|connect through proxy' "$1"
}

# 连不上镜像仓库时给的做法，地址用文档网段占位。
registry_outage_help() {
  cat >&2 <<'EOF'
安装失败：连不上 Docker 镜像仓库（Docker 报的是拨号、超时一类的网络错误，完整输出在上面）。
常见在国内网络：直连 Docker Hub 和 ghcr.io 不通。给 Docker daemon 配代理（地址换成你的代理）：
  sudo mkdir -p /etc/systemd/system/docker.service.d
  printf '[Service]\nEnvironment="HTTP_PROXY=http://192.0.2.10:7899"\nEnvironment="HTTPS_PROXY=http://192.0.2.10:7899"\nEnvironment="NO_PROXY=localhost,127.0.0.1"\n' | sudo tee /etc/systemd/system/docker.service.d/proxy.conf
  sudo systemctl daemon-reload && sudo systemctl restart docker
配好后再执行一次安装命令。daemon 已经配了代理还这样报，就是代理本身连不上镜像仓库，先修代理。
EOF
  exit 1
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

install_dir=${WONTON_INSTALL_DIR:-/opt/wonton}
env_file="$install_dir/.env"

# 一键升级还在做时不和它抢 .env。正在被 Docker 反复重启的也算：它可能正好起不来，没人会把它停下。
updater_state=$(docker inspect --format '{{.State.Running}} {{.State.Restarting}}' wonton-updater 2>/dev/null) ||
  updater_state=
case "$updater_state" in
  *true*)
    die "一键升级还没结束，日志在 docker logs wonton-updater，状态在数据目录的 core/upgrade/state.json。确认它卡住了才这样停掉它，再执行一次安装：docker update --restart=no wonton-updater && docker stop wonton-updater"
    ;;
esac

# 上次安装记下的设置。环境变量里给了的以环境变量为准。
saved() {
  [ -f "$env_file" ] || return 0
  sed -n "s/^$1=//p" "$env_file" | tail -n 1
}

pin=${WONTON_CORE_PIN:-}
if [ -n "$pin" ]; then
  # WONTON_CORE_IMAGE 会把 WONTON_CORE_SOURCE 写成 digest，核心从此不跟渠道，不能用来退回。
  [ -z "${WONTON_CORE_IMAGE:-}" ] || die "WONTON_CORE_PIN 和 WONTON_CORE_IMAGE 不能一起用"
  case "$pin" in
    sha256:*) ;;
    *) die "WONTON_CORE_PIN 要写成 sha256:…，现在是 $pin" ;;
  esac
  [ -n "$(saved WONTON_CORE_SOURCE)" ] || die "$env_file 里没有 WONTON_CORE_SOURCE，不知道从哪个仓库退回"
elif [ -n "${WONTON_RESTORE_VOLUMES:-}" ]; then
  die "WONTON_RESTORE_VOLUMES 要和 WONTON_CORE_PIN 一起用"
fi

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

mkdir -p "$install_dir" "$data_dir"
chmod 700 "$install_dir"

if [ -n "$pin" ]; then
  # 退回时拉渠道所在仓库的旧 digest。
  repository=${image%@*}
  case "${repository##*/}" in
    *:*) repository=${repository%:*} ;;
  esac
  pull_image="$repository@$pin"
else
  pull_image=$image
fi
# 磁盘粗查：Hermes 的镜像约 4.3 GB，加上系统镜像和数据，全新安装要 8 GB 上下；已有的镜像越多要得越少，所以只提醒不拦。
docker_root=$(docker info --format '{{.DockerRootDir}}' 2>/dev/null) || docker_root=
if [ -n "$docker_root" ] && disk_kb=$(df -Pk "$docker_root" 2>/dev/null | awk 'NR==2 {print $4}') && [ -n "$disk_kb" ] && [ "$disk_kb" -lt 8388608 ]; then
  echo "警告：Docker 数据目录 $docker_root 所在盘只剩 $((disk_kb / 1048576)) GB，不到 8 GB。默认插件里 Hermes 的镜像约 4.3 GB，加上核心和系统镜像，全新安装可能装不下（已有的镜像越多，要的越少）；装不下时核心会显示拉镜像失败并隔一阵自动重试。" >&2
fi
if [ "${WONTON_PULL:-}" = never ]; then
  docker image inspect "$pull_image" >/dev/null 2>&1 || die "本机没有镜像 $pull_image"
elif [ -n "$pin" ] && docker image inspect "$pull_image" >/dev/null 2>&1; then
  : # 要退回的版本一般还在本机，这时可能正好上不了网。
else
  info "拉取核心镜像 $pull_image"
  pull_err=$(mktemp "${TMPDIR:-/tmp}/wonton-pull.XXXXXX")
  docker pull "$pull_image" >/dev/null 2>"$pull_err" || {
    cat "$pull_err" >&2
    if looks_like_registry_outage "$pull_err"; then
      rm -f "$pull_err"
      registry_outage_help
    fi
    rm -f "$pull_err"
    die "拉不到镜像 $pull_image"
  }
  rm -f "$pull_err"
fi
# 能写成 digest 就写 digest：升级、回滚都按它（ADR-0012）。本机构建的镜像没有 digest，照原样写。
pinned=$pull_image
if [ -z "$pin" ]; then
  pinned=$(docker image inspect --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' "$pull_image")
  pinned=${pinned:-$pull_image}
fi

# 系统服务都由核心镜像里的 compose 操作，和一键升级同一个（ADR-0025）：宿主不用装 compose 插件，读不到宿主的
# ~/.docker/config.json，两条路建出的容器一样。容器里没有调用者的 WONTON_*，.env 不会被盖住。
compose() {
  docker run --rm --network none -v /var/run/docker.sock:/var/run/docker.sock -v "$install_dir:$install_dir" \
    --entrypoint docker-compose "$pinned" -f "$install_dir/compose.yaml" "$@"
}
docker run --rm --network none --entrypoint docker-compose "$pinned" version >/dev/null 2>&1 ||
  die "核心镜像 $pinned 里没有 compose：这个版本太旧，这个安装脚本用不了它"

setup_plugin_slice() {
  plugin_slice=
  if ! cgroup_info=$(docker info --format '{{.CgroupDriver}} {{.CgroupVersion}}' 2>/dev/null); then
    info "Docker 没报告 systemd cgroup v2；插件只设置每个服务的资源限制"
    return
  fi
  if [ "$cgroup_info" != "systemd 2" ] || ! command -v systemctl >/dev/null 2>&1; then
    info "Docker 不是 systemd cgroup v2，或没有 systemctl；插件只设置每个服务的资源限制"
    return
  fi
  slice_tmp=$(mktemp /etc/systemd/system/.wonton_plugins.slice.XXXXXX) ||
    die "创建插件 slice 临时文件失败"
  if ! docker run --rm --entrypoint wonton-core "$pinned" plugin slice-unit >"$slice_tmp"; then
    rm -f "$slice_tmp"
    die "核心镜像 $pinned 生成插件 slice 失败"
  fi
  [ -s "$slice_tmp" ] || {
    rm -f "$slice_tmp"
    die "核心镜像 $pinned 生成了空的插件 slice"
  }
  chmod 644 "$slice_tmp" && mv "$slice_tmp" /etc/systemd/system/wonton_plugins.slice || {
    rm -f "$slice_tmp"
    die "安装 /etc/systemd/system/wonton_plugins.slice 失败"
  }
  systemctl daemon-reload || die "systemd 重新加载插件 slice 失败"
  plugin_slice=wonton_plugins.slice
}

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

# 主人指定的地址测一下；没指定又没检测过的，检测一次。结果在 egress_proxy、egress_detected 和 record_detected 里。
settle_proxy() {
  record_detected=
  if [ -f "$env_file" ]; then
    detected_value=$(saved WONTON_EGRESS_PROXY_DETECTED)
    daemon_proxy=$(docker info --format '{{.HTTPSProxy}}{{.HTTPProxy}}' 2>/dev/null) || daemon_proxy=
    # 检测值为空、而 daemon 后来配上了代理时再测一次：上一轮可能就是没代理才失败的。daemon 没配代理就不重复
    # （环境没变，测也是空）；检测到过值的不动，主人可能在 App 里调过。
    { [ -n "$detected_value" ] || [ -z "$daemon_proxy" ]; } && record_detected=yes
  fi
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
}

# 从 /dev/tty 读：脚本经管道运行时，标准输入是脚本本身。
confirm() {
  (: </dev/tty) 2>/dev/null || die "退回要你确认，这里没有终端，什么都没动"
  printf '%s\n继续吗？[y/N] ' "$1" >/dev/tty
  read -r answer </dev/tty || answer=
  case "$answer" in
    y | Y | yes) ;;
    *) die "没有确认，什么都没动" ;;
  esac
}

# 这个 digest 最新的、写完了的备份。结果不确定的那份要留着，所以同一个 digest 可能有几份。
backup_of() {
  # shellcheck disable=SC2012 # 目录名是 digest 加编号，按时间排序用 ls 最简单
  ls -dt "$data_dir/core/upgrade/backup/$1" "$data_dir/core/upgrade/backup/$1".* 2>/dev/null |
    while IFS= read -r dir; do
      if [ -f "$dir/.complete" ]; then
        printf '%s\n' "$dir"
        break
      fi
    done
}

# 停服务、恢复备份、处理目标镜像不支持的组件：这份备份就是目标镜像的升级器做的，逻辑放在它里面。
manual_rollback() {
  docker run --rm --network none -v /var/run/docker.sock:/var/run/docker.sock -v "$install_dir:$install_dir" \
    -v "$data_dir:$data_dir" "$pinned" manual-rollback "$install_dir" "$data_dir" "$pinned" "$@" ||
    die "退回失败，看上面的错误。备份还在数据目录的 core/upgrade/backup 里"
}

write_config() {
  info "生成密钥和配置（已有的沿用）"
  # 带上主人的设置：核心启动时也查，但那时容器会一直重启，要等满 10 分钟才报错，这里当场说清楚。
  docker run --rm -v "$install_dir:$install_dir" -v "$data_dir:$data_dir:ro" \
    -e WONTON_DATA_DIR="$data_dir" -e WONTON_OWNER_NAME="$owner_name" -e TZ="$tz" \
    -e WONTON_MATRIX_SERVER_NAME="${WONTON_MATRIX_SERVER_NAME:-}" \
    "$pinned" init "$install_dir" "$domain" || die "设置不对：按上面的错误改了再执行一次"

  settle_proxy
  setup_plugin_slice

  umask 077
  # 写临时文件、落盘、改名：一键升级也读这个文件，不能让它读到写了一半的。
  cat >"$env_file.tmp" <<EOF
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
TZ=$tz
WONTON_LAN_ADDRESSES=$lan_addresses
WONTON_EGRESS_PROXY=$egress_proxy
EOF
  if [ -n "$plugin_slice" ]; then
    echo "WONTON_PLUGIN_SLICE=$plugin_slice" >>"$env_file.tmp"
  fi
# 主人指定了地址时没有检测过，以后取消了还要检测，所以不留这一行。
  if [ -n "$record_detected" ]; then
    echo "WONTON_EGRESS_PROXY_DETECTED=$egress_detected" >>"$env_file.tmp"
  fi
  sync
  mv "$env_file.tmp" "$env_file"
  sync
}

if [ -z "$pin" ]; then
  write_config
else
  backup=$(backup_of "$pin")
  warning="退回到 $pin。"
  migrations=$(docker image inspect --format '{{index .Config.Labels "wonton.core.migrations"}}' \
    "$(saved WONTON_CORE_IMAGE)" 2>/dev/null) || migrations=
  if [ -n "${WONTON_RESTORE_VOLUMES:-}" ]; then
    [ -n "$backup" ] || die "没有 $pin 的备份，恢复不了卷"
    confirm "$warning 恢复备份 $backup，连同里面的卷：备份之后写的数据（包括聊天）会丢。"
    manual_rollback "$backup" --volumes
  else
    [ -z "$migrations" ] ||
      warning="$warning 当前版本迁移过这些卷（$migrations），旧版本可能读不了它们，要恢复就加上 WONTON_RESTORE_VOLUMES=1。"
    if [ -n "$backup" ]; then
      confirm "$warning 恢复备份 $backup：备份之后写的核心数据会丢。"
      manual_rollback "$backup"
    else
      confirm "$warning 没有这个版本的备份：核心数据不会退回，旧版本可能起不来。"
      manual_rollback -
      write_config
    fi
  fi
fi

info "启动核心、身份提供方和 Matrix 服务器（第一次要拉镜像，几分钟）"
# 输出经 tee 原样实时打印，同时留一份给失败时判断原因（compose 里的拉镜像报错出自 daemon，是最真实的连通性证据）。
# if 包住 compose：脚本开着 set -e，裸命令失败会把子壳直接带走，退出的原因码就丢了。
up_log=$(mktemp "${TMPDIR:-/tmp}/wonton-up.XXXXXX")
up_rc_file=$(mktemp "${TMPDIR:-/tmp}/wonton-up-rc.XXXXXX")
( if compose up -d --wait --wait-timeout 600 --remove-orphans; then echo 0; else echo $?; fi >"$up_rc_file" ) 2>&1 | tee "$up_log"
up_rc=$(cat "$up_rc_file")
if [ "$up_rc" != 0 ]; then
  if looks_like_registry_outage "$up_log"; then
    rm -f "$up_log" "$up_rc_file"
    registry_outage_help
  fi
  rm -f "$up_log" "$up_rc_file"
  die "服务没有在 10 分钟内就绪，看看日志：docker logs wonton-core"
fi
rm -f "$up_log" "$up_rc_file"

# 主人手动修好了，App 不再显示旧的结果。备份留多久不受影响。
state_file="$data_dir/core/upgrade/state.json"
[ ! -f "$state_file" ] || mv "$state_file" "$state_file.$(date +%Y%m%d%H%M%S)"

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
查看日志：docker logs -f wonton-core
升级：在 App 里升级，或者再执行一次安装命令。
以后加设备：在已配对的 App 里点“添加设备”；设备都丢了：sudo wonton pair

EOF
# 第一台设备配上前一直显示首次配对码（过期前换新的）；已经有设备时只说明怎么加设备。
"$install_dir/wonton" pair --first ||
  echo "提示：终端里出码失败，用局域网网页上的二维码配对" >&2
