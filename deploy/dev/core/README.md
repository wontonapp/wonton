# 本地核心（插件管理器 + 网关）、身份提供方和 Matrix 服务器

本地开发用的 `wonton-core`、Ory Hydra（小程序单点登录）和 Tuwunel（Matrix 服务器），见 ADR-0009；核心还会预装
模型网关 magpie（ADR-0008），再装一次默认 Agent Hermes（ADR-0007）。核心在容器里
运行，和产品一致：系统服务里只有它挂载 Docker socket，自己加入每个插件网络；浏览器只经核心网关访问插件网页，App 也只经
核心网关访问 Tuwunel。

Mailpit 与正式部署使用同一个固定摘要的官方镜像，等核心健康、生成配置后启动。SMTP 25 和 API 8025 不发布到宿主；仅授权了 `notify` 的插件网络有 `smtp.wonton.internal` 别名。配置、API 私密密码和 SQLite 位于 `<数据目录>/core/mailpit/`；家人加入或移除会重启 Mailpit 载入名单，保留邮件数据（ADR-0056）。

```bash
deploy/dev/core/dev.sh up                    # 构建核心镜像，启动核心、Hydra 和 Tuwunel，等到健康
deploy/dev/core/dev.sh install plugins/examples/grafana   # 预览权限，确认后安装
deploy/dev/core/dev.sh upgrade plugins/examples/grafana   # 预览后升级已装的插件：换容器不吊销 Bot 的设备，数据和密钥都留着
deploy/dev/core/dev.sh token                 # 主人令牌（M0 代替配对，用来登录网关）
deploy/dev/core/dev.sh list
deploy/dev/core/dev.sh logs hydra            # 跟日志：核心栈的服务（core、hydra 等）或插件的服务名
deploy/dev/core/dev.sh diag                  # 只读诊断：插件登录用的主机名、出口代理和 no_proxy
deploy/dev/core/dev.sh open app.grafana     # 主人身份走完登录，打印网页地址和 Cookie 罐路径
deploy/dev/core/dev.sh uninstall app.grafana --purge
deploy/dev/core/dev.sh down                  # 停止，顺带删掉核心按插件起的容器和网络
deploy/dev/core/dev.sh reset                 # 停掉并删除数据、密钥、插件容器、网络和卷
```

`install` 请求超时不代表核心已经停止安装；命令保留 curl 的错误输出和退出码，并提示用同一检出的
`dev.sh status` 查询状态，避免把客户端超时当成安装结果。

`down` 后插件记录和卷都还在，容器没了；再 `up` 时核心只核对、不重建，插件在 `list` 里是 `degraded`。要接着用就
重新装一次：先 `uninstall <ID>`（不带 `--purge`，卷和数据留着）再 `install <包>`。系统插件也走这个流程；
模型网关 magpie 不在锁文件里，卸载后重启核心不会从镜像装回，要用再从商店装。

浏览器打开 `http://app-grafana.wonton.localhost:18480`，用主人令牌登录一次，之后自动进入 Grafana，
不会看到 Grafana 自己的登录页。`*.localhost` 由浏览器和 curl 自动解析到本机，不用改 hosts。命令行里
`dev.sh open app.grafana` 替你把同样的登录走完（核心 → 会话交接 → 插件的 OIDC），打印能直接打开的地址和一个
Cookie 罐路径，curl 加 `-b` 复用。

- 端口 `WONTON_DEV_PORT`（默认 18480），基础域名 `WONTON_DEV_DOMAIN`（默认项目用 `wonton.localhost`，其他项目默认用 `<项目名>.wonton.localhost`）。
  端口也是浏览器看到的地址的一部分，改端口后要重新安装 SSO 插件；需要并行栈时可显式覆盖基础域名。
- core 网络 `<项目名>_core` 由 `dev.sh` 建一次（子网用 `WONTON_DEV_CORE_SUBNET`，不设由 Docker 从默认地址池分），
  compose 里是 external，只负责把容器接上去。网络已经在了就沿用；换子网要先 `down`。子网和别的网络撞了时 `up` 报出
  撞的是哪张网，换一个子网或先把那套 `down`。
  `down`、`reset` 先删插件容器再删网络（容器还接着时 Docker 拒删）；没删掉时 `down` 返回非零，别当成拆完了。
  不让 compose 管它是因为：渲染出来的配置和现有的对不上时它会删了重建，重建后接回去的系统服务不带别名
  （`hydra.wonton.internal` 等），核心再也解析不到，一直重试（#187）。
- 一台 Docker 上跑几套：每套设自己的 `WONTON_DEV_PROJECT`、端口和 `WONTON_DEV_DATA`。`up` 在构建之前查网关和隧道端口
  （`port-preflight.sh`）：别的项目的容器发布了、或本机已有 socket 时直接失败，说明谁占着，并给出往后第一个空的端口。
  实例名 `WONTON_DEV_INSTANCE` 默认取项目名（去掉开头的 `wonton-` 和非字母数字，最多 16 个字符；去了标点时在截短后的名字后面接项目名哈希的前 5 位
  十六进制，只差标点的两个项目名不会得到同一个实例名），插件的资源名和标签都带上它，各管各的。
  默认项目 `wonton-dev-core` 没有实例名。已有数据的环境换了实例名，插件要重装；想留着旧的资源名，把它设成空。
  同一台主机上正式装过云吞（`deploy/install`，项目名 `wonton`）时脚本不去碰它：项目名是 `wonton`、项目的容器是在
  `/opt/wonton`（`WONTON_INSTALL_DIR`）起的、或当前目录在 `/opt/wonton` 里，都算那套栈，`up`、`down`、`reset` 和冒烟
  在标准错误上一句话说明后以退出码 2 停下，一条写命令都不发；升级冒烟例外，它本来就装一套同名项目。
  数据目录同时当作安装目录（`WONTON_INSTALL_DIR`）：联邦名单和整台服务器黑名单由核心原地重写并热加载。
  没有实例名时 `down`、`reset` 按标签分不出插件资源是这套栈的还是同一台 Docker 上正式安装的，只列出、不删，
  并打出要手删的命令。
- 数据目录 `WONTON_DEV_DATA`（默认本目录下的 `.data`）在核心容器内外路径相同，因为插件的绑定挂载由
  宿主上的 Docker 解析。Hydra 的系统密钥、核心签发 Matrix 登录用的 Ed25519 密钥对和 Tuwunel 的注册共享密钥
  在 `${XDG_STATE_HOME:-$HOME/.local/state}/wonton-dev/<项目名>/secrets`（`WONTON_DEV_SECRETS` 可改），按项目名放在
  检出目录外：同一个项目名的各份检出（worktree）共用一份，换个检出起同一个栈不用重发 OIDC 客户端（Ed25519 要
  OpenSSL 3 及以上，macOS 自带的 LibreSSL 不行：`brew install openssl@3`）。检出目录里旧的 `.secrets/` 不再读，
  也不迁移。两者都不提交。
- 宿主网络中继的 unix 套接字在 `${XDG_RUNTIME_DIR:-/run/user/<uid>}/wonton-dev/<实例名>`（`WONTON_DEV_RELAY` 可改）：
  `sun_path` 只有 108 字节，检出目录再深也不影响。建不了这个目录时 `up` 直接失败，换一个短的。
- `dev.sh` 的 HTTP 请求在一个与核心共享网络的 curl 容器里发出，所以 `DOCKER_HOST` 指向远端主机时
  也能用（远端只需要 Docker）。下文提到远端主机时都写成 `$DEV_HOST`，先 `export DEV_HOST=<那台的 SSH 目标>`，
  运行时这样用：`DOCKER_HOST=ssh://$DEV_HOST deploy/dev/core/dev.sh up`。
- 核心容器只读挂着宿主的 `/dev`（`WONTON_HOST_DEV=/host/dev`），申请了 `devices.gpu` 的插件据此检测显卡和设备所属的组
  （ADR-0018）；核心直接跑在宿主上时不用设，读的就是 `/dev`。
- 申请了 `network.lan` 的插件，端口开在哪个地址上由核心起一个宿主网络里的一次性容器检测（`wonton-<实例名>-lan-probe`，
  没设实例名时是 `wonton-lan-probe`，ADR-0019）；`WONTON_DEV_LAN_ADDRESSES` 指定地址时照样检测，但只用指定的地址。
- 插件网络地址段由 `WONTON_PLUGIN_NETWORK_POOL` 指定（默认 `10.213.0.0/16`，私有 IPv4 CIDR），核心从中分配 `/26`，
  不影响 Compose 的核心网络。用完时卸载不用的插件，或扩大这个地址段后重启核心。
- 镜像里编进了构建它的提交号（`dev.sh commit`；Cargo 工作区有没提交的改动时带 `-dirty`），核心启动日志的
  第一行就是它（`docker logs <核心容器> 2>&1 | grep 核心启动`），`docker run --rm wonton-core:dev --version`
  也能看到。验证前先看一眼，确认镜像里是这个检出的代码。
- Docker 客户端的 `proxies`（`~/.docker/config.json`）只由 CLI 在创建容器时注入，不由 Docker 守护进程注入；
  核心经 Engine API 安装的插件不继承它。核心自己的 Compose 用 `x-no-proxy` 清空代理；插件的出网代理规则见
  [ADR-0023](../../../docs/adr/0023-egress-proxy.md#给插件什么)。`dev.sh`、`smoke.sh`、`smoke-onboarding.sh` 的 CLI curl
  探针用 `--noproxy '*'` 直连内部地址；隧道探针用 `--noproxy ''`，确保经过显式指定的本地代理，不必替换 `DOCKER_CONFIG`。
- 这些 CLI curl 探针都带超时（接口 600 秒、网页 120 秒、连接 10 秒），网页或接口不回时不至于把开发栈和它占的验证栈名额一直占着；
  `SMOKE_API_MAX_TIME`、`SMOKE_WEB_MAX_TIME`、`SMOKE_CONNECT_TIMEOUT` 可以覆盖，脚本里更慢的调用在自己的参数里给更长的 `--max-time`。
- 所有 worktree 共用一份 cargo 编译缓存，而 cargo 按修改时间判断要不要重编：源码比别的 worktree 上次编译时
  还旧，就会直接用别人的产物。所以 Dockerfile 每次编译前把工作区的源码 touch 一遍，依赖仍命中缓存，只重编
  工作区的 crate；源码没变时整层命中 Docker 的构建缓存，不重编。

## 文件的端到端

装好文件搜索插件，共享文件夹里放好文件后，`wonton-client` 的 `examples/file-smoke.rs` 扮演手机和真实的 Bot 收发文件，用法写在文件开头。
上传前核心要查磁盘留 5% 空闲。Mac 的磁盘快满时，把 `WONTON_DEV_DATA` 设成 OrbStack 虚拟机里的路径（如 `/var/tmp/wonton-dev`），不然传输会如实报 `storage_full`。

## 经隧道访问

核心内置隧道入口（iroh，UDP `WONTON_DEV_TUNNEL_PORT`，默认 18482，只发布到本机；手机经局域网直连时设
`WONTON_DEV_TUNNEL_BIND=0.0.0.0`）。已登记的设备经隧道访问时就是主人，打开插件不用登录。

设备扫码配对（ADR-0009 的“接口：扫码配对”）：浏览器用 IP 打开 `http://127.0.0.1:18480/` 是局域网状态页，还没有
设备时上面有首次配对码；`dev.sh pair [直连地址]` 随时再发一个，在终端里出二维码，最后一行是码的文字：

```bash
deploy/dev/core/dev.sh pair                    # 发配对码；直连地址默认 127.0.0.1:18482
deploy/dev/core/dev.sh tunnel                  # 服务端 endpoint ID、中继地址和直连地址
deploy/dev/core/dev.sh devices                 # 已登记的设备；revoke <endpoint ID> 吊销并立即断开
```

码里的直连地址要写 App 能到达的地址：同机的 iOS 模拟器用默认的 `127.0.0.1:18482`，Android 模拟器写
`10.0.2.2:18482`，局域网里的手机写本机的局域网 IP，并用 `WONTON_DEV_TUNNEL_BIND=0.0.0.0` 起核心。App 怎么用码见
`apps/ios/README.md`、`apps/compose/README.md`。

`DOCKER_HOST` 指向远端主机时，`127.0.0.1`、`localhost`、`::1` 是那台主机的本机地址，手机和模拟器到不了，`dev.sh pair`
会提醒；局域网里的设备用那台主机的局域网 IP 发码，模拟器用跑着模拟器的那台机器上的栈。

`reset` 删掉核心数据目录，隧道身份（endpoint ID）跟着换；App 用新码添加服务器，旧服务器的数据保留；
`reset --keep-tunnel-identity` 保留 `core/tunnel-key` 和 `core/devices.json`，endpoint ID 不变，已配对的设备不用重新配对
（家人设备记的用户名随核心数据删了，要重新登记）。

加设备时交接聊天密钥（ADR-0087）没有第二台 App 时，用真实的客户端核心扮演旧设备：
`cargo run -p wonton-client --example key-handoff-old-device -- <数据目录> <配对码> <写码的文件>` 登录、开好备份后打开
“添加设备”，等加密历史真的进了房间预览才把给新设备的码写进文件；对表情时另起一行按序打印符号（外面拿它跟新设备屏幕上
的比），并替主人点“相同”，之后留两分钟回答新设备要密钥。恢复密钥另外写进 `<写码的文件>.recovery`。
历史句用 `KEY_HANDOFF_HISTORY_TEXT` 换（走查要和 App 那边同一个字符串）——
它先往默认 Bot 的私聊发一句加密历史，给恢复密钥恢复和历史解密留证据；问不到默认 Bot（没装、没就绪）时用
`KEY_HANDOFF_BOT_USER` 给它的 Matrix 用户 ID。历史发不出去示例直接退，不写码。

在本机用隧道客户端示例扮演手机：`cargo run -p wonton-tunnel --example tunnel-client -- pair '<配对码>'` 用配对码
登记它（`-- id` 输出它的 ID）。`--auth` 和 App 一样起带会话凭据的 HTTP 正向代理，启动时打印凭据：

```bash
cargo run -p wonton-tunnel --example tunnel-client -- proxy --auth --server <服务端 ID> --addr 127.0.0.1:18482 --listen 127.0.0.1:18008
```

```bash
curl --noproxy '' -x http://127.0.0.1:18008 -U wonton:<密码> http://app-grafana.wonton.localhost:18480/api/health
```

`--noproxy ''` 清掉环境变量里的 `NO_PROXY`，否则 `*.localhost` 会绕过代理。加 `-p` 改用 `CONNECT`，和 iOS 的
WKWebView 一样。

已登记的设备经隧道向核心要 Matrix 登录令牌，再用它登录 `matrix.<基础域名>`（ADR-0009 的“App 登录和使用
Matrix”）。令牌两分钟内有效，账号就是主人：

```bash
curl --noproxy '' -x http://127.0.0.1:18008 -U wonton:<密码> -X POST http://wonton.localhost:18480/_wonton/matrix/login-token
curl --noproxy '' -x http://127.0.0.1:18008 -U wonton:<密码> -H 'content-type: application/json' \
  -d '{"type":"org.matrix.login.jwt","token":"<令牌>"}' http://matrix.wonton.localhost:18480/_matrix/client/v3/login
```

`matrix.<基础域名>` 只转发客户端和媒体接口，局域网的浏览器进不来；Tuwunel 的管理接口只有核心能用。

## 模型网关和默认 Agent

核心镜像构建时按 [`plugins/system.lock`](../../../plugins/system.lock) 从独立仓库取包并核对提交和摘要；升级只改
锁文件对应的一行，详见 [插件目录说明](../../../plugins/README.md#预装插件)。启动时从镜像内的
`/usr/share/wonton/default-plugins` 装一次 Hermes（`WONTON_DEFAULT_PLUGINS`），首次启动无需联网取插件包；规则见插件规范的“预装插件”。
不要 Hermes 时用 `WONTON_DEV_DEFAULT_PLUGINS= dev.sh up` 起核心。模型在 Hermes 自己的网页里接（Keys 页填 key 或登录订阅，
Models 页选模型），不经核心。模型网关 magpie 不预装，要在开发里试网关时从商店装；声明了 `models` 的插件经
`models.wonton.internal` 访问它，凭据由核心签发（ADR-0008）。装上 magpie 后，“模型设置”网页
（`http://ai-usemagpie-magpie.wonton.localhost:18480/`）由网关代登录：登录过核心的浏览器和已配对设备直接打开，
不用输 magpie 的网页密钥。也可以用 magpie 的命令行配置 provider，比如指向开发机上已在运行的 magpie。手动配置时
上游地址别写 `http://host.docker.internal:3425`：核心不给插件容器加 host-gateway（有意的隔离），Linux 上这个名字
解析不了；[端到端冒烟](#端到端冒烟)自己会按能不能解析选地址，解析不到时在插件网络的网关上起 socat 转发（宿主防火墙
要放行插件网段到 13425）。加了 provider，默认路由组 `wonton-default` 自动有了它的模型（ADR-0008）。

## MCP 总线

申请了 `tools` 的插件经 `mcp.wonton.internal` 连核心的 MCP 总线（ADR-0016），看到核心自己的工具（列出插件、在官方
索引里找插件、装卸插件、帮助文档）和在运行的插件声明的工具。帮助文档默认读镜像里的 `/usr/share/wonton/docs`，`WONTON_DOCS` 能换目录。Bot 只能装官方索引里的插件；开发用的核心镜像（Dockerfile 的 `dev`
阶段）默认读 GitHub 上的官方索引，要看镜像里那份开发索引（`WONTON_STORE_INDEX`，见 [store/README.md](store/README.md)）就在起栈时把它设成 `/usr/share/wonton/store`；发布的镜像不带本地索引。装卸插件和会改数据的工具要主人
确认：核心保存确认卡片并通知已配对设备，App 在发起的会话输入栏上方和“等你确认”列表里显示，不往 Matrix 房间发卡片。
已配对设备经隧道回答（`POST /_wonton/confirmations/{id}`，请求体 `{"answer": "approve"}`），10 分钟没回答就过期（`WONTON_CONFIRMATION_TTL_SECS`）。每次调用记一行审计，在数据目录的 `core/audit.jsonl`。
没有 App 的主机上用真实的客户端核心回答：`cargo run -p wonton-client --example confirm-card -- <数据目录> <配对码> approve`。
开发栈还没有配对设备时，用主人令牌经网关批，别干等（插件套件的 `scripts/dev.sh approve` 就是这么做的）：
`dev.sh api GET /_wonton/api/confirmations` 列出等回答的卡片（`id`、`title`），
`dev.sh api POST /_wonton/confirmations/<id> -d '{"answer":"approve"}'` 批准。`answer` 取 `approve`、`approve_wider`、
`allow_window`、`reject`，必须和卡片上的按钮一致；按钮带 `choice_id` 的要原样带上，密钥类卡片随回答带 `secrets`。

## 推送

Tuwunel 直连核心在 `core` 网络上的推送专用监听，与正式安装一致；地址、来源检查和宿主开发模式见
[ADR-0003 的“接口：推送”](../../../docs/adr/0003-matrix-and-encrypted-push.md#接口推送m0-3)。

## 端到端冒烟

核心一键升级有单独的一条（ADR-0025）：`deploy/dev/core/smoke-upgrade.sh`，在要测的主机上直接执行，用 install.sh 安装，走升级、自动回滚和手动退回，这台主机上不能有真装的云吞。

引导和帮助有单独的一条（ADR-0034）：`deploy/dev/core/smoke-onboarding.sh`，用独立的 compose 项目和临时数据，
验证官方索引读得懂、引导如实推荐、帮助文档跟着索引更新（更新、失败保旧、min_core 挡旧核心），以及模型网关
三种错误的原因各不相同。GitHub 连不上时 export `https_proxy`；「错的 key」和「断网」两个用例要经出口容器到真实
上游，设 `WONTON_SMOKE_EGRESS_PROXY=http://host.docker.internal:<宿主代理端口>`，不设时这两个用例跳过。

两台云吞之间的有两条冒烟和一个开发命令，共用一套隔离夹具（`share-smoke/`：两台核心经 iroh 配对、各自的 Tuwunel、自己的 DNS 和 Pebble）：
分享插件（ADR-0051）是 `dev.sh share-smoke`；跨机 Bot 联系人（ADR-0050）是 `deploy/dev/core/smoke-contacts.sh`，
依赖和步骤写在脚本开头。插件作者看好友打开自己的插件时插件拿到什么身份，用
`dev.sh friend <插件包目录或 compose 文件>`（做法和限制见 `friend-probe.sh` 开头）。

### MatrixRTC 通话冒烟（ADR-0096）

用 `deploy/dev/calls/smoke.sh`：两台 Android 设备／模拟器装 Debug APK、同一开发栈上的加密房间里跑三条媒体路径，按退出码判定路径、双向音视频、本轮成员和挂断。前置（插件包、远程栈、PyJWT、模拟器）和用法见 `deploy/dev/calls/README.md`。它取代原来的 `call-e2e.sh`。

内部邮箱有单独的一条（ADR-0056），**在目标主机的本机 Docker 上运行**，由调用方申请并释放栈名额：

```bash
(
  export WONTON_DEV_PROJECT="wonton-mailbox-$(date +%s)-$$"
  slot="$WONTON_DEV_PROJECT"
  trap '"$HOME/wonton-dev/stack-slot.sh" release "$slot"' EXIT
  "$HOME/wonton-dev/stack-slot.sh" acquire --project "$WONTON_DEV_PROJECT" "$slot" -- \
    deploy/dev/core/smoke-mailbox.sh
)
```

需要 Bash、Docker/buildx、Python 3 和 OpenSSL 3；端口默认 18561/18563，可用 `WONTON_DEV_PORT` /
`WONTON_DEV_TUNNEL_PORT` 改。脚本构建本检出的核心，以临时 Compose override 关闭系统/默认插件（不改正式配置），
安装上游官方 Python 镜像的最小 `notify` 插件，不接真实模型、不装 Hermes。验证插件无宿主端口、三项 SMTP 注入、
服务器宣告 AUTH 但不宣告 STARTTLS、外部/不存在/插件地址在 RCPT 阶段被 550 拒绝、信封独有收件人经 Mailpit 补入 Bcc、
主人自己的 Wonton 私聊收到
`app.wonton.plugin_mail`；短 HTML 转文本保留链接，长 HTML 截到 4000 字并附原文，小附件和原文都从 Matrix 下载核对，
另有一封随便填凭据、经 AUTH LOGIN 发出的信也照常投进私聊。
还会停掉本栈 Tuwunel，观察 Mailpit 至少 40 秒不删信，再恢复并等到投递后队列清空。身份认证用本次临时栈新生成的
Ed25519 密钥签一次主人 JWT，直接登录真实 Tuwunel；不覆盖设备配对/隧道（由主冒烟覆盖），也不注入家人记录来假造加入，
家人加入/移除流程不在此脚本范围。凭据只在私有临时文件和本栈容器内使用，不进命令参数或输出；退出 trap 清本栈
容器、网络、卷和临时目录（不保留现场、不 prune）。

通用能力依赖有单独的一条（ADR-0053），同样只在目标主机的本机 Docker 上运行。**先用当前检出构建开发核心镜像**，显式传入
`WONTON_CAPABILITY_CORE_IMAGE`；脚本不构建、不用已有的 main 镜像兜底。调用方提供未占用的独立 Docker 子网：

```bash
(
  export WONTON_CAPABILITY_CORE_IMAGE='wonton-core:<当前检出的开发镜像标签>'
  export WONTON_DEV_PROJECT="wonton-capability-$(date +%s)-$$"
  export WONTON_DEV_CORE_SUBNET='10.214.1.0/24' # 换成测试主机未占用的子网
  deploy/dev/core/smoke-capabilities.sh
)
```

需要 Bash、Docker Compose、Python 3 和 OpenSSL 3；默认端口 18571/18573，可用 `WONTON_DEV_PORT` /
`WONTON_DEV_TUNNEL_PORT` 改。脚本沿用开发栈的联邦/TLS 准备，以临时 override 关闭系统/默认插件。用待测核心镜像的
`plugin index-entry` 从本地测试包生成私有索引，服务和使用方都是固定摘要的官方 Python 镜像；真实核心、Hydra、Tuwunel
和 Docker Engine 执行安装与网关请求，不连接模型或外部业务服务。覆盖完整提供方安装决定及独立输入、真实凭据签发/校验、可信来源头、
必需依赖卸载拦截、可选依赖后装自动连接/卸载提示、过期计划零副作用、真实非零退出后的整批回滚和保留数据。
停止本栈提供方后验证 503，重启核心核对原凭据和选择，再显式启动必需使用方，验证先启动提供方且同一凭据恢复 HTTP；
不把重启核心描述为自动取消手动停止。退出时只清本次精确项目/实例的容器、网络、卷和临时目录，清理完成后才输出最终成功；
失败保留私有服务日志、失败命令的标准输出/错误、HTTP 错误响应及探针状态；日志可能含测试凭据，不上传。终端只输出步骤、错误类别与日志路径。
插件中的 `restart: "no"` 表示一次性服务，安装会等待其以 0 退出；常驻 HTTP 服务用 `unless-stopped`，即使带健康检查也不写 `no`。
此段是运行说明，不代表已执行通过。


```bash
deploy/dev/core/smoke.sh        # 本机 Docker；要设 WONTON_SMOKE_MODEL（见下）
DOCKER_HOST=ssh://$DEV_HOST deploy/dev/core/smoke.sh  # 远端 Linux 主机
```

用独立的 compose 项目、端口（18481，隧道 18483）和临时数据目录，依次验证：一条命令安装 Grafana 示例插件并通过
健康检查；插件不暴露端口、只在自己的网络里、访问不到 Hydra 的管理端口；未登录的请求被拒；登录核心
一次后打开插件自动登录（Grafana 里是 `owner`、管理员角色）；已配对设备经隧道打开插件不用登录、直接是
`owner`，未登记和已吊销的设备被拒，设备用局域网状态页上的首次配对码配对，配上后状态页不再出码、同一个码不能再用
（设备由隧道客户端示例扮演，跑在核心的网络命名空间里）；没有 OIDC 的
插件收不到网关 Cookie 和伪造的转发头；已配对设备经隧道拿到 Matrix 登录令牌、登录 Tuwunel，局域网拿不到
令牌也进不了 `matrix.<基础域名>`，管理接口和绕路到它的路径都不转发；回声 Bot（`plugins/examples/echo-bot`）装好后
有自己的账号、显示名和私聊主房间，主人的一台新设备经隧道发消息能收到它的回声，卸载后它的令牌立即
失效，不删数据重装沿用账号和主房间，删除数据时账号停用；核心不预装模型网关，申请 `models` 的插件没有网关时照样
装得上、拿到空变量；核心在主人配对之前就自己装好了 Hermes，它不经模型网关，过了健康检查
之后，和它的私聊才出现在主人的聊天列表里，还没接模型时 Hermes 回提示去它网页的 Models 页；冒烟给它的 Bot
在宿主 magpie 上建一条限定模型的凭据，再经 Hermes 自己的接口把宿主 magpie 配成它的 provider，之后主人的新设备
经隧道和它对话两轮：逐轮检查宿主 magpie 的用量里新增了至少一条成功、有输出 token 的调用（这个 magpie 版本不记网关凭据名，按 OpenAI 的 UA 和本轮开始时间认），
第二轮回复必须提到第一轮让它“介绍”或“自己”；本轮没有成功调用就带回复原文停止，不依赖 Hermes 的报错措辞。
删除数据卸载后 Bot 账号停用、冒烟的凭据删掉；“模型设置”网页由网关代登录，局域网浏览器登录过核心、已配对设备经
隧道都直接打开并能改设置，没登录和别的插件网页发来的请求被拒，网页密钥不出现在任何响应和 Cookie 罐里；对 Hermes
说装文件搜索，核心在确认列表里发卡片，Hermes 的插件网络和局域网浏览器都答不了，
主人的手机确认后才装好，Hermes 再经 MCP 总线在授权的文件夹里找到文件，审计记下这两次调用；主人在 App 里停掉 Hermes 正在跑的一轮（终端里的命令），命令被停掉、链不再显示在跑，之后 Hermes 照常回答；妈妈作为家人加入、成为 Hermes 的联系人后，Hermes 发给她的私聊主人拒绝的不发、卡片等着时停链的撤回不发、批准的经出箱发到她的手机，审计如实记下；妈妈说的那一轮链代表她，她让发的私聊不沾主人的放行时段，以 Hermes 的身份直接问总线时，说话人请求头决定按谁的权限给工具，没带时按这一轮的会话、再按在跑的轮次里最严的、都没有才按归属人；没申请 `matrix.bot`、
`models`、`tools` 的插件进不了对应的内部地址；
核心重启后插件恢复，卸掉的 Hermes 不会被装回来；卸载后容器、网络、卷、应用数据和 OIDC 客户端都没有残留。
结束时清理全部资源，`KEEP=1` 保留现场。Hermes 一步要真实模型：开发机上要有在运行的 magpie，模型名经
`WONTON_SMOKE_MODEL` 传入：填 `/v1/models` 里的 id（带 provider 前缀），magpie 里一个能对话的模型。
Hermes 怎么连宿主见下面「远端主机」一段；
不读取开发机 magpie 的配置和密钥。

远端主机拉不到 Docker Hub 时，先把镜像从本机传过去（以 amd64 主机为例），传完删掉本机的 amd64 副本。核心镜像的 Rust、
Debian 基础镜像从 ECR Public 拉（见 Dockerfile），不用传：

```bash
for image in oryd/hydra:v26.2.0 ghcr.io/matrix-construct/tuwunel:v2.0.0 tusproject/tusd:v2.10.1 \
  grafana/grafana:12.2.1 traefik/whoami:v1.11 curlimages/curl:8.23.0 alpine/socat:1.8.0.3; do
  docker pull --platform linux/amd64 "$image"
  docker save --platform linux/amd64 "$image" | ssh "$DEV_HOST" docker load
  docker image rm --platform linux/amd64 "$image"
done
```

插件包里按摘要锁定的镜像（magpie、Hermes、文件搜索镜像的基础镜像 node）不能只传 amd64：那样传过去的是一个新的
镜像，摘要对不上，核心还会去拉。整个索引传过去（本机有的平台都带上），再在远端打上标签：

```bash
image=nousresearch/hermes-agent:v0.23.4@sha256:ef0650d523ee1b5ab32c779acabc446d9936869a9932591553688a62bcf0c42f
docker pull --platform linux/amd64 "$image"
docker save "$image" | ssh "$DEV_HOST" docker load
ssh "$DEV_HOST" docker tag "sha256:${image#*@sha256:}" "${image%@*}"
```

核心和系统容器不继承 Docker 客户端配置里的代理。开发环境要经代理访问 GitHub 或者测出网插件时，设 `WONTON_EGRESS_PROXY`（ADR-0023）；
链接预览经代理用的 DoH 服务器用 `WONTON_DOH_URL` 换（默认 `https://1.1.1.1/dns-query`，ADR-0065）。

远端主机直连 crates.io、GitHub 太慢时，构建镜像经那台主机自己的代理（`WONTON_DEV_BUILD_PROXY`，构建用宿主网络）。
Linux 的 Docker 不给容器加 `host.docker.internal`，冒烟自己判：Hermes 容器能解析这个名字就用它，解析不到就在
宿主机上起一个 socat 转发，监听在所有地址上，结束时删掉，Hermes 经它自己网络的网关地址连。宿主 magpie 不在 3425
时直接用 `WONTON_SMOKE_MODEL_RELAY=<宿主上的地址:端口>`：设了它冒烟就用这个地址（两种宿主都一样），不再自己起转发；
自己起转发时主机的防火墙要放行插件网段到 13425，否则 Hermes 一步连不上模型。`WONTON_SMOKE_MODEL` 填
`/v1/models` 里的 id（带 provider 前缀）。宿主 magpie 在容器里时冒烟按发布 3425 的容器找它；名字不同设
`WONTON_SMOKE_HOST_MAGPIE=<容器名>`（宿主上同时有 `magpie` 也进那个容器）：

```bash
DOCKER_HOST=ssh://$DEV_HOST WONTON_DEV_BUILD_PROXY=http://127.0.0.1:7899 WONTON_DEV_BUILD_JOBS=16 \
  WONTON_SMOKE_MODEL='<开发机 magpie 的 /v1/models 里的 id（带 provider 前缀）>' deploy/dev/core/smoke.sh
```

## 在宿主上直接运行核心

调试时也可以 `cargo run -p wonton-core`。这时核心按容器 IP 访问插件（Linux 和 OrbStack 可以，
Docker Desktop for Mac 不行），SSO 插件经 `host-gateway` 访问宿主上的网关。bollard 不读 docker context，
套接字不在 `/var/run/docker.sock` 时先设置：

```bash
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
WONTON_DATA_DIR=/tmp/wonton-dev WONTON_PUID=$(id -u) WONTON_PGID=$(id -g) cargo run -p wonton-core
```

## 选型实验

- `experiments/gateway-bench.sh`：内置网关和 Traefik + forwardAuth 的压测对比（先 `dev.sh up`）。
- `experiments/mcp-bus/`：MCP 总线的三个候选，见其中的 README。

结论写在 ADR-0006、ADR-0009 的“验证结果（M0-6）”。
