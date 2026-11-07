# 正式安装

在装好 Docker Engine 28 以上的 Linux 主机（amd64、arm64）上用 root 执行 `install.sh`，装好核心、身份提供方
（Ory Hydra）和 Matrix 服务器（Tuwunel）；核心启动后在后台自己装上系统插件（模型网关 magpie），再装一次默认 Agent
Hermes（ADR-0007），授予它 `models`、`matrix.bot`、`tools`、`network.egress`。装好后浏览器用主机的
IP 打开 `http://<IP>:8480/`，是局域网状态页：运行状态，以及还没有设备时的首次配对二维码。安装终端最后也显示
同一个首次配对码，第一台设备配上后结束。以后加设备在已配对的 App 里点“添加设备”，设备都丢了在服务器上执行
`sudo wonton pair`（ADR-0009）。

Hermes 装好后，聊天列表里就有和它的加密私聊。它的镜像要下载约 1 GB（解开约 4 GB），全新主机上要等 5 分钟左右，
还没装好时插件列表和局域网状态页上是“安装中”；拉不下镜像时两处都写明原因，核心隔一阵自动重试。在“模型设置”里
添加任意一家模型服务商的 key，Hermes 就能回答；没添加时它会提示去添加（ADR-0008）。主人卸掉 Hermes 后，核心不会
再自动装它。

- `install.sh`：检查系统和 Docker，拉核心镜像，在镜像里执行 `wonton-core init` 生成密钥和配置，写 `.env`，
  用镜像里的 compose 执行 `up`，等到都健康。可调的环境变量写在脚本开头。
- `compose.yaml`、`wonton`：编进核心的二进制，由 `wonton-core init` 写到安装目录，跟着核心镜像的版本走。
  `wonton` 是主机上的命令，转到核心容器里的 `wonton-core pair`。

镜像发布在 `ghcr.io/geraldpeng6/wonton-core`，`stable` 和 `beta` 两个渠道（ADR-0012），1.0 之前安装脚本默认装 `beta`；安装脚本另放在一个公开的
小仓库里，因为主仓库是私有的。

Tuwunel 经 `core` 网络直连核心的推送专用监听，不用发布额外端口；网络、地址和配置约定见
[ADR-0003 的“接口：推送”](../../docs/adr/0003-matrix-and-encrypted-push.md#接口推送m0-3)。

## 装在哪

| 路径 | 内容 |
| --- | --- |
| `/opt/wonton/compose.yaml`、`/opt/wonton/wonton` | 由当前的核心镜像写出，每次 `init` 重写 |
| `/usr/local/bin/wonton` | 链接到 `/opt/wonton/wonton` |
| `/opt/wonton/.env` | 核心镜像的来源（`WONTON_CORE_SOURCE`）和按 digest 写下的镜像（`WONTON_CORE_IMAGE`）、端口、基础域名、用户名、时区、数据目录，由 `install.sh` 写 |
| `/opt/wonton/secrets/` | Matrix 签发私钥、Matrix 服务器名（`matrix-server-name`）、Tuwunel 配置（含公钥和注册共享密钥）、联邦的自签证书、Hydra 的系统密钥；0700 |
| `/opt/wonton/fed/` | 联邦名单（`config.toml`）：由核心生成初值、运行时原地重写（ADR-0050）；Tuwunel 只读挂载 |
| `/etc/systemd/system/wonton_plugins.slice` | systemd + cgroup v2 主机上由安装脚本生成的插件总量限制；删除时先删 `.env` 里的 `WONTON_PLUGIN_SLICE` 行，再删这个文件并执行 `systemctl daemon-reload` |
| `/var/lib/wonton/`（`WONTON_DATA_DIR`） | 核心和插件的数据，容器内外路径相同 |
| Docker 卷 `wonton_hydra`、`wonton_tuwunel` | Hydra 的 SQLite、Tuwunel 的数据库 |

显示名和邮箱在 App 的设置里填（ADR-0030）。用户名不能改（ADR-0022）：核心把它们都记在
`<数据目录>/core/owner.json`，用户名对不上就拒绝启动，要换只能用新的数据目录重新安装。

## 升级

再执行一次 `install.sh` 就是升级或修复：拉 `WONTON_CORE_SOURCE`（1.0 之前默认 `beta` 渠道）的最新版本，已有的密钥、
数据和已配对的设备都沿用，compose 文件按新的核心镜像重写，基础域名、Matrix 的服务器名和主人的用户名不能改。App 里的一键升级由一次性的 `wonton-updater` 容器执行，步骤和回滚见 ADR-0025。

退回旧版本时带上它的 digest，之后照样跟渠道。有当时升级留下的备份就恢复核心数据和安装目录，备份之后写的数据会丢，要在终端里确认一次：

```bash
curl -fsSL <安装脚本地址> | sudo WONTON_CORE_PIN=sha256:… sh
```

## 拉不到镜像

核心、Tuwunel 和 magpie 的镜像在 GHCR，Hydra、tusd 和 Hermes 的在 Docker Hub；国内的主机常常连不上 Docker Hub 或 GHCR。
安装脚本拉不到镜像、且 Docker 报的是拨号超时一类的网络错误时，会直接说明连不上镜像仓库，并给出给 Docker daemon 配代理
的做法（systemd drop-in），配好后重新执行安装命令即可；别的拉取失败（认证、清单不存在、磁盘满这些）按原样报。

装 Docker 本身也常被网络卡住：用官方 `get.docker.com` 时，脚本内部的下载不会带你的代理环境变量（`sudo` 会剥掉），要
`sudo env https_proxy=http://代理地址:端口 sh get-docker.sh`，或先给 apt 配代理。magpie 和 Hermes 由核心拉：拉不到时
插件列表里如实显示原因，核心隔一段时间（最长 10 分钟）自动重试，配好代理后不用重装。国内的镜像仓库按 ADR-0012 以后加上。

安装前脚本会看一眼 Docker 数据目录所在盘的剩余空间，不到 8 GB 时提醒一次（Hermes 的镜像约 4.3 GB；已有的镜像越多要得
越少），只提醒不拦。

## 出网代理

第一次安装时脚本检测一次这台主机的代理（调用者的 `HTTPS_PROXY`、Docker 客户端配置、Docker 守护进程的配置），测得通就用，并打印出来；
重跑安装时，上次检测没测到、而 Docker 守护进程后来配上了代理的，会重新检测一次（先因为没代理装失败的，配好代理重跑就能用上）；
检测到过值的、或主人显式给了 `WONTON_EGRESS_PROXY` 的不动。不要的话在 App 的设置里清空。要指定地址就给 `WONTON_EGRESS_PROXY`，测不通时安装失败，设成空再执行一次就取消。
宿主的防火墙要放行 Docker 网段到代理的端口，代理只监听回环地址时连不上。规则和报错见 [ADR-0023](../../docs/adr/0023-egress-proxy.md)。

拉镜像不走这个代理，走 Docker 守护进程自己的配置，见上一节。

## 在本机测

没发布的改动先在本机构建镜像，用 OrbStack 的虚拟机当干净的主机：

```bash
docker build -f deploy/dev/core/Dockerfile -t wonton-core:install-test .
```

```bash
orb create ubuntu:noble wonton-install-arm
```

虚拟机里按 Docker 官方文档装好 Docker，把镜像导进去（`docker save wonton-core:install-test | orb -m wonton-install-arm
sudo docker load`），在本机用 `python3 -m http.server --directory deploy/install` 提供安装脚本，然后在虚拟机里：

```bash
curl -fsSL http://<本机在 OrbStack 网桥上的 IP>:8000/install.sh | sudo WONTON_CORE_IMAGE=wonton-core:install-test WONTON_PULL=never sh
```

amd64 用 `orb create --arch amd64 …` 和 `docker buildx build --platform linux/amd64 --load …`（交叉编译，见 Dockerfile
开头）。用 `cargo run -p wonton-tunnel --example tunnel-client -- pair '<配对码>'` 在本机扮演手机配对。测完
`orb delete` 删掉虚拟机。

安装脚本会把镜像写成仓库 digest，本机 `docker build` 出来的镜像要带仓库 digest（containerd 镜像存储会带；经典存储不带，
得先推到一个 registry 再拉）。没有时脚本停下并说明，不会退回写标签。
