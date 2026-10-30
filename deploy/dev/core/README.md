# 本地核心（插件管理器 + 网关）、身份提供方和 Matrix 服务器

本地开发用的 `wonton-core`、Ory Hydra（小程序单点登录）和 Tuwunel（Matrix 服务器），见 ADR-0009；核心还会预装
模型网关 magpie（ADR-0008）。核心在容器里
运行，和产品一致：唯一挂载 Docker socket，自己加入每个插件网络；浏览器只经核心网关访问插件网页，App 也只经
核心网关访问 Tuwunel。

```bash
deploy/dev/core/dev.sh up                    # 构建核心镜像，启动核心、Hydra 和 Tuwunel，等到健康
deploy/dev/core/dev.sh install plugins/examples/grafana/docker-compose.yml   # 预览权限，确认后安装
deploy/dev/core/dev.sh install plugins/hermes   # 插件包目录（compose 引用了包里的文件时）打成 tar 上传
deploy/dev/core/dev.sh token                 # 主人令牌（M0 代替配对，用来登录网关）
deploy/dev/core/dev.sh list
deploy/dev/core/dev.sh uninstall app.grafana --purge
deploy/dev/core/dev.sh down                  # 停止；reset 连数据一起删除
```

浏览器打开 `http://app-grafana.wonton.localhost:18480`，用主人令牌登录一次，之后自动进入 Grafana，
不会看到 Grafana 自己的登录页。`*.localhost` 由浏览器和 curl 自动解析到本机，不用改 hosts。

- 端口 `WONTON_DEV_PORT`（默认 18480），基础域名 `WONTON_DEV_DOMAIN`（默认 `wonton.localhost`）。
  端口也是浏览器看到的地址的一部分，Hydra 的签发方地址随之变化，改端口后要重新安装 SSO 插件。
- 数据目录 `WONTON_DEV_DATA`（默认本目录下的 `.data`）在核心容器内外路径相同，因为插件的绑定挂载由
  宿主上的 Docker 解析。Hydra 的系统密钥、核心签发 Matrix 登录用的 Ed25519 密钥对和 Tuwunel 的注册共享密钥
  在 `.secrets/`，首次 `up` 时生成（Ed25519 要 OpenSSL 3 及以上，macOS 自带的 LibreSSL 不行：
  `brew install openssl@3`）。两者都不提交。
- `dev.sh` 的 HTTP 请求在一个与核心共享网络的 curl 容器里发出，所以 `DOCKER_HOST` 指向远端主机时
  也能用（远端只需要 Docker）：`DOCKER_HOST=ssh://npchy deploy/dev/core/dev.sh up`。
- 镜像里编进了构建它的提交号（`dev.sh commit`；Cargo 工作区有没提交的改动时带 `-dirty`），核心启动日志的
  第一行就是它（`docker logs <核心容器> 2>&1 | grep 核心启动`），`docker run --rm wonton-core:dev --version`
  也能看到。验证前先看一眼，确认镜像里是这个检出的代码。
- 所有 worktree 共用一份 cargo 编译缓存，而 cargo 按修改时间判断要不要重编：源码比别的 worktree 上次编译时
  还旧，就会直接用别人的产物。所以 Dockerfile 每次编译前把工作区的源码 touch 一遍，依赖仍命中缓存，只重编
  工作区的 crate；源码没变时整层命中 Docker 的构建缓存，不重编。

## 经隧道访问

核心内置隧道入口（iroh，UDP `WONTON_DEV_TUNNEL_PORT`，默认 18482，只发布到本机；手机经局域网直连时设
`WONTON_DEV_TUNNEL_BIND=0.0.0.0`）。已登记的设备经隧道访问时就是主人，打开插件不用登录。M0 还没有扫码
配对，用主人令牌登记设备：

```bash
deploy/dev/core/dev.sh tunnel                  # 服务端 endpoint ID、中继地址和直连地址
deploy/dev/core/dev.sh allow <设备 endpoint ID>  # 登记；revoke 吊销并立即断开，devices 列出
```

在本机用隧道客户端示例扮演手机（`cargo run -p wonton-tunnel --example tunnel-client -- id` 输出它的 ID）。
`--auth` 和 App 一样起带会话凭据的 HTTP 正向代理，启动时打印凭据：

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

## 模型网关

核心镜像里带着系统插件 magpie（`plugins/magpie`），核心启动后在后台预装它（`WONTON_SYSTEM_PLUGINS`）。声明了
`models` 的插件经 `models.wonton.internal` 访问它，凭据由核心签发。“模型设置”网页
（`http://ai-usemagpie-magpie.wonton.localhost:18480/`）由网关代登录：登录过核心的浏览器和已配对设备直接打开，不用输
magpie 的网页密钥。也可以用 magpie 的命令行配置 provider，比如指向开发机上已在运行的 magpie（同 `deploy/dev/agent`）：

```bash
docker exec wonton-ai-usemagpie-magpie--magpie magpie provider add upstream url=http://host.docker.internal:3425/v1 key=placeholder models=rm/glm-5.3-flash
docker exec wonton-ai-usemagpie-magpie--magpie magpie group add wonton-default models=upstream/rm/glm-5.3-flash routing=order
```

App 用 `dev.sh ticket [直连地址] [网页地址]` 生成的连接信息连接（见 `apps/ios/README.md`、`apps/compose/README.md`），
里面带着基础域名：App 的本地代理和 WebView 只让这个域名下的地址走隧道。
直连地址默认 `127.0.0.1:18482`（同机的 iOS 模拟器），Android 模拟器写 `10.0.2.2:18482`，局域网里的手机写本机的
局域网 IP，并用 `WONTON_DEV_TUNNEL_BIND=0.0.0.0` 起核心。

## 推送

Tuwunel 默认不往私网地址推送（`ip_range_denylist`），compose 里的 `TUWUNEL_PROXY` 让它只把 `push.wonton.internal`
（核心的推送代理）经核心网关代理出去，见 ADR-0003 的“接口：推送”。

## 端到端冒烟

```bash
deploy/dev/core/smoke.sh                           # 本机 Docker
DOCKER_HOST=ssh://npchy deploy/dev/core/smoke.sh   # 远端 Linux 主机
```

用独立的 compose 项目、端口（18481，隧道 18483）和临时数据目录，依次验证：一条命令安装 Grafana 示例插件并通过
健康检查；插件不暴露端口、只在自己的网络里、访问不到 Hydra 的管理端口；未登录的请求被拒；登录核心
一次后打开插件自动登录（Grafana 里是 `owner`、管理员角色）；已配对设备经隧道打开插件不用登录、直接是
`owner`，未登记和已吊销的设备被拒（设备由隧道客户端示例扮演，跑在核心的网络命名空间里）；没有 OIDC 的
插件收不到网关 Cookie 和伪造的转发头；已配对设备经隧道拿到 Matrix 登录令牌、登录 Tuwunel，局域网拿不到
令牌也进不了 `matrix.<基础域名>`，管理接口和绕路到它的路径都不转发；回声 Bot（`plugins/examples/echo-bot`）装好后
有自己的账号、显示名和加密主房间，主人的一台新设备经隧道发加密消息能收到它的加密回声，卸载后它的令牌立即
失效，不删数据重装沿用账号和主房间，删除数据时账号停用；核心预装了模型网关，使用方拿到只属于自己的凭据，
经 `models.wonton.internal` 能用、卸载后凭据被吊销；Hermes 按插件包目录装好后连上 Matrix，主人的新设备经隧道和它
加密对话两轮，用量记在它的凭据名下，删除数据卸载后 Bot 账号停用、凭据吊销；“模型设置”网页由网关代登录，局域网
浏览器登录过核心、已配对设备经隧道都直接打开并能改设置，没登录和别的插件网页发来的请求被拒，网页密钥不出现在任何
响应和 Cookie 罐里；没申请 `matrix.bot`、`models` 的插件进不了对应的内部地址；
核心重启后插件恢复；卸载后容器、网络、卷、应用数据和 OIDC 客户端都没有残留。
结束时清理全部资源，`KEEP=1` 保留现场。Hermes 一步要真实模型：开发机上要有在运行的 magpie，提供 `rm/glm-5.3-flash`，
冒烟里的 magpie 经 `host.docker.internal:3425` 用它（同 `deploy/dev/agent`），不读取开发机 magpie 的配置和密钥。

远端主机拉不到 Docker Hub 时，先把镜像从本机传过去（以 amd64 主机为例）：

```bash
for image in rust:1.99.1-slim-trixie debian:trixie-slim oryd/hydra:v26.2.0 ghcr.io/matrix-construct/tuwunel:v1.9.4 \
  ghcr.io/yetone/magpie:0.1.1327 nousresearch/hermes-agent:v0.22.5 grafana/grafana:12.2.1 traefik/whoami:v1.11 \
  curlimages/curl:8.23.0; do
  docker pull --platform linux/amd64 "$image"
  docker save --platform linux/amd64 "$image" | ssh npchy docker load
done
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
