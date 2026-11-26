# 重装演练虚拟机

发版前在一台干净的 Ubuntu 虚拟机里按正式方式装一遍，跑演练检查。宿主是任意一台 amd64 Linux，要有 Docker 和
`/dev/kvm`，不要 sudo。当前用户要在 `docker` 组里；`kvm` 组不用进，`/dev/kvm` 的组号由 `vm.sh` 带进容器。

- qemu 跑在普通容器里：只加 `--device /dev/kvm`，不加特权、不挂 docker.sock、不加 capability；容器 `--cpus 4 --memory 10g`，
  虚拟机 4 核 8 GB、40 GB 磁盘（qcow2 叠加层，底是只读的 cloud image）。
- 容器用 host 网络，虚拟机用 qemu 的 user 网络：虚拟机里的端口（8480、8482、80、443 等）都留在里面，宿主上只占
  `127.0.0.1` 上一个 20000 以上的 ssh 转发端口，起之前用 `ss` 确认没人监听。
- 系统镜像只用 cloud-images.ubuntu.com 的官方 cloud image，下载后用 `SHA256SUMS` 核对，`SHA256SUMS.gpg` 用 Debian
  `ubuntu-keyring` 包里的 cloud image 签名公钥验，任何一步不过就停。
- 虚拟机第一次启动时 cloud-init 执行 `setup.sh`：配代理，按 Docker 官方文档从 download.docker.com 装 Docker Engine。
- 容器、镜像、文件名都带 `wonton-rehearsal` 前缀，文件放在 `~/wonton-rehearsal/`。

## 起、进、删

```sh
deploy/install/vm/vm.sh up        # 约 5 分钟：构建 qemu 镜像、下载校验、起虚拟机、装 Docker
deploy/install/vm/vm.sh ssh       # 进去，用户 wonton，免密 sudo；后面跟命令就只执行它
deploy/install/vm/vm.sh cp deploy/install/install.sh install.sh
deploy/install/vm/vm.sh down      # 删虚拟机（容器、叠加盘、种子盘、密钥、日志、端口转发）
deploy/install/vm/vm.sh purge     # 再删下载的 cloud image 和 qemu 镜像
```

| 变量 | 用途 |
| --- | --- |
| `WONTON_VM_HOST_PROXY` | 宿主上的 HTTP 代理，如 `http://127.0.0.1:7892`：构建镜像、下载 cloud image 用它；虚拟机里的 apt、Docker 守护进程和登录环境用同一个端口（回环地址换成 qemu 网关 `10.0.2.2`） |
| `WONTON_VM_DOCKER_VERSION` | 装指定版本的 Docker Engine，如 `29.7.2`；不设装最新的 |
| `WONTON_VM_UBUNTU` | `24.04`（默认）或 `26.04` |
| `WONTON_VM_NAME` | 同一台宿主上起第二台时换名字，必须以 `wonton-rehearsal` 开头 |
| `WONTON_VM_SSH_PORT` | 指定 ssh 转发端口（≥ 20000）；不设随机挑一个空闲的 |
| `WONTON_VM_BASE_IMAGE` | qemu 镜像的基础镜像，默认 `debian:trixie-slim`；守护进程拉不到时先用别的办法导入再指向它 |
| `WONTON_VM_DIR` | 磁盘和密钥放哪，默认 `~/wonton-rehearsal` |

控制台输出在 `~/wonton-rehearsal/<名字>-console.log`，cloud-init 的输出在虚拟机的 `/var/log/cloud-init-output.log`。

## 装 Wonton

把安装脚本拷进去，在虚拟机里用 root 执行。`sudo` 会剥掉代理变量，要显式带上；指定版本的核心见
[安装说明](../README.md) 和 `install.sh` 开头：

```sh
vm.sh cp deploy/install/install.sh install.sh
vm.sh ssh 'sudo env HTTPS_PROXY=http://10.0.2.2:7892 WONTON_MATRIX_NAME_MODE=none \
  WONTON_ACME_DIRECTORY=https://acme.rehearsal.test/directory WONTON_ACME_ACCEPT_TERMS=true sh install.sh'
# 指定版本：WONTON_CORE_IMAGE=ghcr.io/wontonapp/wonton-core@sha256:…；本地构建的镜像先 docker save 拷进去再 load
```

“不要名字”的安装要求给 ACME 坐标，只在主人到 App 里配了自己的域名后才用到。上面给的是一个不存在的 `.test` 地址，
没有和任何 CA 打交道；要测证书就换成 [entry-cloud](../../dev/entry-cloud/) 的 Pebble。

## 演练检查

`docker-restart-check.sh`：重启 Docker 守护进程后，Compose 项目 `wonton` 里原来在跑的服务和核心自己建的入口、连接器
容器都要回到 running（有健康检查的要 healthy），150 秒内没有容器重启，退出码都是 0。不通过时打印每个容器的状态和
Docker 记下的 `HasBeenManuallyStopped`。

```sh
vm.sh cp deploy/install/vm/docker-restart-check.sh check.sh
vm.sh ssh 'sudo sh check.sh restart'      # systemctl restart docker
vm.sh ssh 'sudo sh check.sh stop-start'   # stop docker docker.socket，停 10 秒再 start
```
