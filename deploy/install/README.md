# 正式安装

在装好 Docker Engine 28 以上的 Linux 主机（amd64、arm64）上用 root 执行 `install.sh`，装好核心、身份提供方
（Ory Hydra）和 Matrix 服务器（Tuwunel）；核心启动后在后台自己装上系统插件（模型网关 magpie），再装一次默认 Agent
Hermes（ADR-0007），授予它 `models`、`matrix.bot`、`tools`、`network.egress`。装好后浏览器用主机的
IP 打开 `http://<IP>:8480/`，是局域网状态页：运行状态，以及还没有设备时的首次配对二维码。

安装输出是终端时，最后显示同一个首次配对码，第一台设备配上后结束；`curl | sudo sh` 的标准输入是管道，不影响这个体验。
输出被重定向到文件或管道时，安装在服务就绪后结束，不在安装输出里显示配对码。之后打开局域网状态页扫码，
或在服务器终端执行 `sudo wonton pair --first`。安装成功不代表设备已经配对。
以后加设备在已配对的 App 里点“添加设备”，设备都丢了在服务器上执行 `sudo wonton pair`（ADR-0009）。

手动运行配对命令时，终端放不下完整二维码就只显示文本配对码，并提示需要的最少列数和当前列数；可把窗口拉宽后重新运行，
或直接在 App 里粘贴文本码。二维码使用 L 级纠错，保留四模块静区；输出到管道或文件时仍显示二维码，不按终端宽度隐藏。

Hermes 装好后，聊天列表里就有和它的加密私聊。它的镜像要下载约 1 GB（解开约 4 GB），全新主机上要等 5 分钟左右，
还没装好时插件列表和局域网状态页上是“安装中”；拉不下镜像时两处都写明原因，核心隔一阵自动重试。在“模型设置”里
添加任意一家模型服务商的 key，Hermes 就能回答；没添加时它会提示去添加（ADR-0008）。主人卸掉 Hermes 后，核心不会
再自动装它。

- `install.sh`：检查系统和 Docker，拉核心镜像，在镜像里执行 `wonton-core init` 注册服务器名、准备证书、生成密钥和配置，写 `.env`，
  最后用镜像里的 compose 启动 Tuwunel 等服务，等到都健康。可调的环境变量写在脚本开头和下面。
- `compose.yaml`、`wonton`：编进核心的二进制，由 `wonton-core init` 写到安装目录，跟着核心镜像的版本走。
  `wonton` 是主机上的命令，转到核心容器里的 `wonton-core pair`。

镜像发布在 `ghcr.io/wontonapp/wonton-core`，`stable` 一个渠道（ADR-0070），安装脚本默认装它；安装脚本另放在一个公开的
小仓库里，因为主仓库是私有的。

Tuwunel 经 `core` 网络直连核心的推送专用监听，不用发布额外端口；网络、地址和配置约定见
[ADR-0003 的“接口：推送”](../../docs/adr/0003-matrix-and-encrypted-push.md#接口推送m0-3)。

Mailpit 收取 `notify` 插件发给主人和已加入家人的内部邮件（[ADR-0056](../../docs/adr/0056-internal-mailbox.md)）。SMTP 25 和网页/API 8025 都不发布到宿主；API 密码只由核心持有。
核心启动时生成 `<数据目录>/core/mailpit/config/`，Mailpit 等核心健康后启动；收件人变化时核心原子改写名单并停止、启动同一容器，`core/mailpit/data/` 中的 SQLite 保留。只按七天过期清理，不使用上游默认的 500 封数量清理，以免聊天离线时提前丢信。备份数据目录时一并备份 `core/mailpit/`，其中含私有 API 密码。

## 服务器名和证书

第一次安装默认用家名服务送的名字（`WONTON_MATRIX_NAME_MODE=given`，可省略）。没有家名服务时，换一种装法（一条比一条命令，加在原来的安装命令上）：

```sh
# 不要名字：以后不能改；要和外面的 Matrix 互通得清空聊天数据重装
curl -fsSL <安装地址>/install.sh | sudo WONTON_MATRIX_NAME_MODE=none sh
# 用自己的域名：证书按下表准备
curl -fsSL <安装地址>/install.sh | sudo WONTON_MATRIX_SERVER_NAME=chat.example.com sh
```

先配置服务、邀请码和证书输入，
`init` 完成注册和证书准备后才启动 Tuwunel；缺配置、注册或签证书失败就停止。家名服务没有内置的公共默认地址，ACME 也没有默认 CA。
开发只用[本地 entry-cloud](../dev/entry-cloud/) 的 Pebble、DNS 和 `.test` 名字，并显式信任测试 CA。

| 输入 | 用途 |
| --- | --- |
| `WONTON_CLIENT_SERVICE` | 家名服务的 HTTPS 地址；默认送名字时必填 |
| `WONTON_CLIENT_INVITE_FILE` | 存放邀请码的文件，读取后去掉首尾空白；不把邀请码本身放进环境变量或命令行 |
| `WONTON_CLIENT_CA_FILE` | 家名服务的 PEM CA 文件；使用测试或私有 CA 时提供，未给时用系统信任 |
| `WONTON_FRP_CA_FILE` | frps 的 PEM CA 文件；送名字时必填，用于校验连接器到中继的 TLS 证书 |
| `WONTON_ACME_DIRECTORY` | 显式指定 ACME directory 的 HTTPS URL；使用 ACME 签证书时必填 |
| `WONTON_ACME_ACCEPT_TERMS=true` | 运营者明确接受所选 CA 的条款后才设置；ACME 模式缺少此确认就停止 |
| `WONTON_ACME_CA_FILE` | ACME 和 acme-dns 使用的 PEM CA 文件；测试 CA 显式传入，不关闭 TLS 验证 |
| `WONTON_ACME_RESOLVERS` | 可选 DNS 解析器，逗号分隔的 IP 或 IP:端口（端口可省略，默认 53），每项首尾空白忽略，例如 `192.0.2.10,198.51.100.10:5353`；安装容器、挑战记录和 CAA 查询都经它 |
| `WONTON_ACME_EMAIL` | 可选 ACME 账号联系邮箱 |
| `WONTON_ACME_NETWORK` | 已存在的 Docker 网络，默认 `bridge`；初始化和 lego 必须能经它访问家名服务、ACME、acme-dns 和所选 DNS |
| `WONTON_ACME_DNS_PROVIDER`、`WONTON_ACME_DNS_ENV_FILE` | 自己的域名使用的 lego DNS provider 及其 dotenv 凭据文件，必须成对提供；送的名字使用注册结果中的 acme-dns 凭据，不混用自定义 provider |
| `WONTON_ENTRY_CERTIFICATE=external` | 显式使用外部证书，跳过 ACME 签发和自动续期；安装前准备 `<安装目录>/entry/certs/site.pem` |

所有 `*_FILE` 输入必须是宿主机的绝对路径。安装脚本把文件复制到 `<安装目录>/entry-input/<变量名>`，文件权限 0600、目录 0700，
并在 `entry-input/init.env` 中传递复制后的路径。不要删除这些副本：保存的入口配置继续引用 CA 和 DNS 凭据文件，家名状态查询和续期仍要读取它们。
重跑安装（升级、修复）时没再给的输入沿用已记录的值和副本，不会清空；要换文件就再给一次路径。

高级选项在第一次安装时明确选择：

- **自己的域名**：设 `WONTON_MATRIX_SERVER_NAME` 为自己的小写 DNS 名（可同时设 `WONTON_MATRIX_NAME_MODE=own`），不注册送的名字。
  主人负责 DNS、对外入口和服务发现；证书用上表的自定义 DNS provider，或显式选择外部证书。自己的入口连接器默认发布在 `127.0.0.1:8443`，
  由主人的公网转发接入；`.env` 里的 `WONTON_ENTRY_BIND`、`WONTON_ENTRY_PORT` 改绑定地址和端口。
- **不要名字**：设 `WONTON_MATRIX_NAME_MODE=none`，不能同时设 `WONTON_MATRIX_SERVER_NAME`。生成随机的 `<24 位>.wonton.internal`，
  只与同类已配对云吞通信。安装时会警告：以后不能改名，要接入 Matrix 生态必须清空聊天数据、所有人换新账号。

送的名字和自己的域名都开放 Matrix 联邦，严格验证证书（`allow_invalid_tls_certificates = false`）。配对目的地走 iroh，其余由 Tuwunel 直接访问，
没有配对时不配置全局代理。不要名字的模式保留 ADR-0050 的封闭联邦、自签证书和 TLS 例外，只能与同类通信，和有名字的服务器不通（ADR-0051）。
三种选择都在首次安装时确定身份；重装、升级沿用 `secrets/matrix-server-name`，有名字时也复用 `entry/settings.json`，不重新注册，也不能借重跑安装改名。
送的名字属于运营家名服务的一方；服务停止或名字停用会使外部服务器无法联系这台，账号不能搬到另一个名字上。

### 证书文件和续期

ACME 为服务器名本身、`*.<服务器名>` 和 `*.lan.<服务器名>` 签证书。`entry/certs/site.key` 是留在家里的私钥，lego 只收到
`entry/requests/site.csr`；`entry/acme/` 保存 ACME 账号、DNS 凭据和签发结果，`entry/prepared.json` 记录账号与 provider 的绑定。
送的名字会先把 ACME 账号 URI 绑定到家名服务的 CAA，并查询生效后才下单。备份时保留整个 `entry/` 和 `entry-input/`，不要单独删私钥或账号文件。

核心启动时和此后每 24 小时使用 `entry/settings.json` 保存的 directory、条款确认、网络、CA、DNS 解析器及 provider/凭据输入调用证书流程，
成功后原子发布 `entry/certs/site.pem`，给入口容器发 `SIGUSR2` 热加载。签发失败保留原证书并通知主人；热加载失败也单独通知。
`init` 发现已有 `site.pem` 就沿用，重新执行安装不会重新选择证书配置。

外部证书的 `site.pem` 必须包含证书链和对应的 PEM 私钥，覆盖入口所用的服务器名；Wonton 不替它续期。
主人负责在到期前更新此文件，保持入口容器可读的所有者和权限，并执行
`docker kill -s SIGUSR2 wonton-entry` 热加载。

入口容器（`wonton-entry`）和连接器（`wonton-connector`）不在 Compose 里：核心按 `entry/settings.json` 启动时和此后每隔一阵
经 Docker API 对账建、删它们，改了 `.env` 里的发布地址或端口会重建。升级到带这个行为的核心后第一次启动会自动按已有设置
建出来，上一版由 Compose 建的同名容器会被认领、删掉重建，不用手动做什么；回退到旧核心时升级器会按标签删掉它们。

它们挂着 `clients`、`edge`、`relay`，`docker compose down` 删不掉还连着网络的容器：拆栈或卸载前先
`docker rm -f wonton-entry wonton-connector`（有实例名时是 `wonton-<实例名>--entry`、`--connector`），再 `down`。

## 装在哪

| 路径 | 内容 |
| --- | --- |
| `/opt/wonton/compose.yaml`、`/opt/wonton/wonton` | 由当前的核心镜像写出，每次 `init` 重写 |
| `/usr/local/bin/wonton` | 链接到 `/opt/wonton/wonton` |
| `/opt/wonton/.env` | 核心镜像的来源（`WONTON_CORE_SOURCE`）和按 digest 写下的镜像（`WONTON_CORE_IMAGE`）、端口、基础域名、用户名、时区、数据目录、中继目录，由 `install.sh` 写 |
| `/opt/wonton/secrets/` | Matrix 签发私钥、Matrix 服务器名（`matrix-server-name`）、注册共享密钥、Hydra 的系统密钥；0700 |
| `/opt/wonton/secrets/tuwunel/` | Tuwunel 自己的文件：配置（含公钥）、联邦名单、不要名字模式的自签证书；只读挂进 Tuwunel，0700 |
| `/opt/wonton/entry-input/` | 安装输入文件的私有副本与 `init.env`，CA 和 DNS 凭据文件供状态查询、续期复用 |
| `/opt/wonton/entry/` | `settings.json`（名字、注册凭据、证书设置）、入口和连接器配置、`certs/site.pem` 与 `site.key`、ACME 状态；含私钥，不对外分享 |
| `/etc/systemd/system/wonton_plugins.slice` | systemd + cgroup v2 主机上由安装脚本生成的插件总量限制；删除时先删 `.env` 里的 `WONTON_PLUGIN_SLICE` 行，再删这个文件并执行 `systemctl daemon-reload` |
| `/var/lib/wonton/`（`WONTON_DATA_DIR`） | 核心和插件的数据，容器内外路径相同 |
| `/run/wonton/relay/`（`WONTON_RELAY_DIR`） | 宿主网络中继的 unix 套接字，按哈希命名；`sun_path` 只有 108 字节，所以不放在数据目录下 |
| Docker 卷 `wonton_hydra`、`wonton_tuwunel` | Hydra 的 SQLite、Tuwunel 的数据库 |

显示名和邮箱在 App 的设置里填（ADR-0030）。用户名不能改（ADR-0022）：核心把它们都记在
`<数据目录>/core/owner.json`，用户名对不上就拒绝启动，要换只能用新的数据目录重新安装。

## 升级

再执行一次 `install.sh` 就是升级或修复：拉 `WONTON_CORE_SOURCE`（默认 `stable` 渠道）的最新版本，已有的密钥、
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
配了代理时，链接预览经代理用 DoH 解析域名，服务器默认 `https://1.1.1.1/dns-query`，要换就设 `WONTON_DOH_URL`（ADR-0065）。

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
