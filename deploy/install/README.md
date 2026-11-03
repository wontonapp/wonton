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
  `docker compose up` 等到都健康。可调的环境变量写在脚本开头。
- `compose.yaml`、`wonton`：编进核心的二进制，由 `wonton-core init` 写到安装目录，跟着核心镜像的版本走。
  `wonton` 是主机上的命令，转到核心容器里的 `wonton-core pair`。

镜像发布在 `ghcr.io/geraldpeng6/wonton-core`，`stable` 和 `beta` 两个渠道（ADR-0012），1.0 之前安装脚本默认装 `beta`；安装脚本另放在一个公开的
小仓库里，因为主仓库是私有的。

## 装在哪

| 路径 | 内容 |
| --- | --- |
| `/opt/wonton/compose.yaml`、`/opt/wonton/wonton` | 由当前的核心镜像写出，每次 `init` 重写 |
| `/usr/local/bin/wonton` | 链接到 `/opt/wonton/wonton` |
| `/opt/wonton/.env` | 核心镜像的来源（`WONTON_CORE_SOURCE`）和按 digest 写下的镜像（`WONTON_CORE_IMAGE`）、端口、基础域名、主人的用户名、显示名和邮箱、时区、数据目录，由 `install.sh` 写 |
| `/opt/wonton/secrets/` | Matrix 签发私钥、Tuwunel 配置（含公钥和注册共享密钥）、Hydra 的系统密钥；0700 |
| `/var/lib/wonton/`（`WONTON_DATA_DIR`） | 核心和插件的数据，容器内外路径相同 |
| Docker 卷 `wonton_hydra`、`wonton_tuwunel` | Hydra 的 SQLite、Tuwunel 的数据库 |

主人的显示名和邮箱可以改：改 `.env` 再 `docker compose -f /opt/wonton/compose.yaml up -d`。用户名不能改（ADR-0022）：核心把它记在
`<数据目录>/core/owner.json`，对不上就拒绝启动，要换只能用新的数据目录重新安装。

## 升级

再执行一次 `install.sh` 就是升级或修复：拉 `WONTON_CORE_SOURCE`（1.0 之前默认 `beta` 渠道）的最新版本，已有的密钥、
数据和已配对的设备都沿用，compose 文件按新的核心镜像重写，Matrix 的服务器名（基础域名）和主人的用户名不能改。App 里的一键升级
（ADR-0012）按同样的顺序做：拉新镜像，改 `.env` 里的 `WONTON_CORE_IMAGE`，在新镜像里执行 `wonton-core init`，
再 `docker compose -f /opt/wonton/compose.yaml up -d`；健康检查不过就把 `.env` 改回旧的 digest 再 `up`。

## 拉不到镜像

核心、Tuwunel 和 magpie 的镜像在 GHCR，Hydra、tusd 和 Hermes 的在 Docker Hub；国内的主机常常连不上 Docker Hub 或 GHCR。
安装时拉镜像失败，先给 Docker 配镜像加速或代理，再执行一次安装命令。magpie 和 Hermes 由核心拉：拉不到时插件列表里
如实显示原因，核心隔一段时间（最长 10 分钟）自动重试，配好代理后不用重装。国内的镜像仓库按 ADR-0012 以后加上。

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
