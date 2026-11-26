#!/bin/sh
# 虚拟机第一次启动时由 cloud-init 以 root 执行：配代理（apt、Docker 守护进程、登录环境），按 Docker 官方文档
# 从 download.docker.com 装 Docker Engine。变量来自 /etc/wonton-rehearsal.env，由 vm.sh 写进种子盘。
set -eu
# shellcheck disable=SC1091 # 运行时才有，由 vm.sh 写进种子盘
. /etc/wonton-rehearsal.env

if [ -n "$PROXY" ]; then
  printf 'Acquire::http::Proxy "%s";\nAcquire::https::Proxy "%s";\n' "$PROXY" "$PROXY" \
    >/etc/apt/apt.conf.d/90wonton-rehearsal-proxy
  mkdir -p /etc/systemd/system/docker.service.d
  printf '[Service]\nEnvironment="HTTP_PROXY=%s" "HTTPS_PROXY=%s" "NO_PROXY=localhost,127.0.0.1"\n' \
    "$PROXY" "$PROXY" >/etc/systemd/system/docker.service.d/http-proxy.conf
  # 进虚拟机后手动跑 curl、install.sh 时用；sudo 会剥掉环境变量，装 Wonton 时要显式带上（见 README）。
  printf 'export http_proxy=%s https_proxy=%s no_proxy=localhost,127.0.0.1\n' "$PROXY" "$PROXY" \
    >/etc/profile.d/wonton-rehearsal-proxy.sh
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl -fsSL ${PROXY:+-x "$PROXY"} https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
# shellcheck disable=SC1091
codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $codename stable" \
  >/etc/apt/sources.list.d/docker.list
apt-get update

version=
if [ -n "$DOCKER_VERSION" ]; then
  # 官方源里的版本串形如 5:29.7.2-1~ubuntu.24.04~noble。
  version=$(apt-cache madison docker-ce | awk -v v="$DOCKER_VERSION" '$3 ~ ":" v "-" { print $3; exit }')
  [ -n "$version" ] || { echo "download.docker.com 里没有 docker-ce $DOCKER_VERSION" >&2; exit 1; }
fi
apt-get install -y "docker-ce${version:+=$version}" "docker-ce-cli${version:+=$version}" \
  containerd.io docker-buildx-plugin docker-compose-plugin
usermod -aG docker wonton
systemctl daemon-reload
systemctl restart docker
docker version --format 'Docker Engine {{.Server.Version}}'
