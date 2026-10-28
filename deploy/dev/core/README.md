# 本地核心（插件管理器 + 网关）与身份提供方

本地开发用的 `wonton-core` 和 Ory Hydra（小程序单点登录，见 ADR-0009 的验证结果）。核心在容器里运行，
和产品一致：唯一挂载 Docker socket，自己加入每个插件网络；浏览器只经核心网关访问插件网页。

```bash
deploy/dev/core/dev.sh up                    # 构建核心镜像，启动核心和 Hydra，等到健康
deploy/dev/core/dev.sh install plugins/examples/grafana/docker-compose.yml   # 预览权限，确认后安装
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
  宿主上的 Docker 解析。Hydra 的系统密钥在 `.secrets/`。两者都不提交。
- `dev.sh` 的 HTTP 请求在一个与核心共享网络的 curl 容器里发出，所以 `DOCKER_HOST` 指向远端主机时
  也能用（远端只需要 Docker）：`DOCKER_HOST=ssh://npchy deploy/dev/core/dev.sh up`。

## 端到端冒烟

```bash
deploy/dev/core/smoke.sh                           # 本机 Docker
DOCKER_HOST=ssh://npchy deploy/dev/core/smoke.sh   # 远端 Linux 主机
```

用独立的 compose 项目、端口（18481）和临时数据目录，依次验证：一条命令安装 Grafana 示例插件并通过
健康检查；插件不暴露端口、只在自己的网络里、访问不到 Hydra 的管理端口；未登录的请求被拒；登录核心
一次后打开插件自动登录（Grafana 里是 `owner`、管理员角色）；没有 OIDC 的插件收不到网关 Cookie 和
伪造的转发头；核心重启后插件恢复；卸载后容器、网络、卷、应用数据和 OIDC 客户端都没有残留。
结束时清理全部资源，`KEEP=1` 保留现场。

远端主机拉不到 Docker Hub 时，先把镜像从本机传过去（以 amd64 主机为例）：

```bash
for image in rust:1.99.1-slim-trixie debian:trixie-slim oryd/hydra:v26.2.0 grafana/grafana:12.2.1 \
  traefik/whoami:v1.11 curlimages/curl:8.23.0; do
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
