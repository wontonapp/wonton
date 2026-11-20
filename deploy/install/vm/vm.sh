#!/bin/sh
# 发版前重装演练用的 Ubuntu 虚拟机：qemu 跑在一个普通容器里（只给 /dev/kvm），虚拟机用 qemu 的 user 网络，
# 宿主上只占一个 127.0.0.1 上的 ssh 转发端口。用法和变量见同目录的 README.md。
#
#   vm.sh up           构建 qemu 镜像、下载并校验 Ubuntu cloud image、建叠加盘和种子盘、起虚拟机、等 cloud-init 装完 Docker
#   vm.sh ssh [命令]   进虚拟机（用户 wonton，免密 sudo）
#   vm.sh cp 本地 远端  把文件拷进虚拟机
#   vm.sh status       容器状态和 ssh 端口
#   vm.sh down         删虚拟机：容器、叠加盘、种子盘、密钥、控制台日志
#   vm.sh purge        down 之后再删下载的 cloud image 和 qemu 镜像
set -eu

prefix=wonton-rehearsal
name=${WONTON_VM_NAME:-$prefix-vm}
dir=${WONTON_VM_DIR:-$HOME/$prefix}
release=${WONTON_VM_UBUNTU:-24.04}
image=$prefix/qemu:latest
base_image=${WONTON_VM_BASE_IMAGE:-debian:trixie-slim}
host_proxy=${WONTON_VM_HOST_PROXY:-}
docker_version=${WONTON_VM_DOCKER_VERSION:-}
here=$(cd "$(dirname "$0")" && pwd)

die() {
  echo "vm.sh：$*" >&2
  exit 1
}

case "$name" in
  "$prefix"*) ;;
  *) die "WONTON_VM_NAME 要以 $prefix 开头：清理只认这个前缀" ;;
esac

# 宿主回环上的代理在虚拟机里是 qemu user 网络的网关 10.0.2.2（容器用 host 网络，qemu 的出站就是宿主的出站）。
vm_proxy() {
  printf '%s' "$host_proxy" | sed -e 's#//127\.0\.0\.1:#//10.0.2.2:#' -e 's#//localhost:#//10.0.2.2:#'
}

port_file() { echo "$dir/$name.port"; }

ssh_port() {
  [ -r "$(port_file)" ] || die "$name 没有起来（找不到 $(port_file)）"
  cat "$(port_file)"
}

# 只绑 127.0.0.1 的高端口；没指定就在 20000–29999 里挑一个 ss 看着没人监听的。
pick_port() {
  if [ -n "${WONTON_VM_SSH_PORT:-}" ]; then
    port=$WONTON_VM_SSH_PORT
  else
    port=$(awk 'BEGIN { srand(); print 20000 + int(rand() * 10000) }')
  fi
  [ "$port" -ge 20000 ] || die "ssh 转发端口要在 20000 以上：$port"
  if ss -Htln "sport = :$port" | grep -q .; then
    die "端口 $port 已经有人监听，换一个（WONTON_VM_SSH_PORT）"
  fi
  echo "$port"
}

# 容器里以 root 执行（ssh-keygen 要能在 passwd 里查到当前用户），产物交还给调用者。
in_container() {
  docker run --rm --network host -e OWNER="$(id -u):$(id -g)" \
    -e HOME=/tmp -e https_proxy="$host_proxy" -e http_proxy="$host_proxy" \
    -v "$dir:/work" -v "$here:/vm:ro" -w /work "$image" sh /vm/vm.sh "$@"
}

build_image() {
  docker build --network host --build-arg BASE="$base_image" \
    --build-arg http_proxy="$host_proxy" --build-arg https_proxy="$host_proxy" \
    -t "$image" "$here"
}

# 容器里执行：下载 cloud image，用 Ubuntu 的 SHA256SUMS 和它的 GPG 签名核对。任何一步不过就停。
fetch() {
  url=https://cloud-images.ubuntu.com/releases/$release/release
  file=ubuntu-$release-server-cloudimg-amd64.img
  target=$prefix-ubuntu-$release.img
  curl -fsSLo "$prefix-SHA256SUMS" "$url/SHA256SUMS"
  curl -fsSLo "$prefix-SHA256SUMS.gpg" "$url/SHA256SUMS.gpg"
  gpgv --keyring /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg \
    "$prefix-SHA256SUMS.gpg" "$prefix-SHA256SUMS"
  [ -f "$target" ] || curl -fSLo "$target.part" "$url/$file"
  [ -f "$target" ] || mv "$target.part" "$target"
  want=$(awk -v f="*$file" '$2 == f { print $1 }' "$prefix-SHA256SUMS")
  [ -n "$want" ] || die "SHA256SUMS 里没有 $file"
  echo "$want  $target" | sha256sum -c -
  chown "$OWNER" "$target" "$prefix-SHA256SUMS" "$prefix-SHA256SUMS.gpg"
}

# 容器里执行：叠加盘（cloud image 只读作底）、ssh 密钥、cloud-init 种子盘。
prepare() {
  disk=$name.qcow2
  [ -f "$disk" ] || qemu-img create -q -f qcow2 -F qcow2 -b "$prefix-ubuntu-$release.img" "$disk" 40G
  [ -f "$name-key" ] || ssh-keygen -q -t ed25519 -N '' -C "$name" -f "$name-key"
  seed=$(mktemp -d)
  printf 'instance-id: %s\nlocal-hostname: %s\n' "$name" "$prefix" >"$seed/meta-data"
  user_data "$(cat "$name-key.pub")" >"$seed/user-data"
  genisoimage -quiet -output "$name-seed.iso" -volid cidata -joliet -rock \
    "$seed/user-data" "$seed/meta-data"
  rm -rf "$seed"
  chown "$OWNER" "$disk" "$name-key" "$name-key.pub" "$name-seed.iso"
}

user_data() {
  cat <<EOF
#cloud-config
users:
  - name: wonton
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys: ["$1"]
ssh_pwauth: false
write_files:
  - path: /etc/wonton-rehearsal.env
    content: |
      PROXY=$VM_PROXY
      DOCKER_VERSION=$DOCKER_VERSION
  - path: /usr/local/sbin/wonton-rehearsal-setup
    permissions: "0755"
    content: |
EOF
  sed 's/^/      /' "/vm/setup.sh"
  echo "runcmd: [[/usr/local/sbin/wonton-rehearsal-setup]]"
}

qemu() {
  exec qemu-system-x86_64 -machine q35,accel=kvm -cpu host -m 8G -smp 4 \
    -display none -monitor none -serial "file:/work/$name-console.log" \
    -drive "if=virtio,format=qcow2,file=/work/$name.qcow2" \
    -drive "if=virtio,format=raw,readonly=on,file=/work/$name-seed.iso" \
    -netdev "user,id=net0,hostfwd=tcp:127.0.0.1:$1-:22" -device virtio-net-pci,netdev=net0
}

vm_ssh() {
  ssh -q -i "$dir/$name-key" -p "$(ssh_port)" -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 wonton@127.0.0.1 "$@"
}

up() {
  mkdir -p "$dir"
  docker container inspect "$name" >/dev/null 2>&1 && die "$name 已经在了；先 vm.sh down"
  build_image
  in_container _fetch
  in_container _prepare "$(vm_proxy)" "$docker_version"
  port=$(pick_port)
  echo "$port" >"$(port_file)"
  if ! docker run -d --name "$name" --label "$prefix=1" --init --device /dev/kvm \
    --group-add "$(stat -c %g /dev/kvm)" -u "$(id -u):$(id -g)" \
    --network host --cpus 4 --memory 10g -v "$dir:/work" -v "$here:/vm:ro" \
    "$image" sh /vm/vm.sh _qemu "$port" >/dev/null; then
    # 起不来时容器可能留着，删掉再报：下一次 up 不用先手工 down。
    docker rm -f "$name" >/dev/null 2>&1 || true
    rm -f "$(port_file)"
    die "qemu 容器起不来，看上面的报错"
  fi
  echo "虚拟机起来了，ssh 转发在 127.0.0.1:$port；等 cloud-init 装 Docker"
  tries=0
  until vm_ssh true 2>/dev/null; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || die "5 分钟内 ssh 不通，看 $dir/$name-console.log；容器还留着，排完用 vm.sh down"
    sleep 5
  done
  vm_ssh sudo cloud-init status --wait --long || die "cloud-init 没装好，看 vm.sh ssh sudo cat /var/log/cloud-init-output.log；容器还留着，排完用 vm.sh down"
  vm_ssh docker --version
}

down() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -f "$dir/$name.qcow2" "$dir/$name-seed.iso" "$dir/$name-key" "$dir/$name-key.pub" \
    "$dir/$name-console.log" "$(port_file)"
}

purge() {
  down
  rm -f "$dir/$prefix-ubuntu-"*.img "$dir/$prefix-ubuntu-"*.img.part "$dir/$prefix-SHA256SUMS" \
    "$dir/$prefix-SHA256SUMS.gpg"
  docker image rm "$image" >/dev/null 2>&1 || true
  rmdir "$dir" 2>/dev/null || true
}

cmd=${1:-}
[ $# -gt 0 ] && shift
case "$cmd" in
  up) up ;;
  ssh) if [ -t 0 ]; then vm_ssh -t "$@"; else vm_ssh "$@"; fi ;;
  cp)
    [ $# -ge 2 ] || die "用法：vm.sh cp <本地文件> <虚拟机里的路径>"
    scp -q -i "$dir/$name-key" -P "$(ssh_port)" -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null "$1" "wonton@127.0.0.1:$2"
    ;;
  status) docker ps -a --filter "name=^$name\$" --format '{{.Names}} {{.Status}}'
    echo "ssh 端口：$(ssh_port)" ;;
  down) down ;;
  purge) purge ;;
  _fetch) fetch ;;
  _prepare) VM_PROXY=$1; DOCKER_VERSION=$2; prepare ;;
  _qemu) qemu "$1" ;;
  *) sed -n 's/^#   //p' "$0" >&2; exit 2 ;;
esac
