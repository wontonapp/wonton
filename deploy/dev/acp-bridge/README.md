# ACP bridge 开发样例

本目录是 C2 最小 bridge/agent 插件样例，不是独立插件仓库。`bridge` 运行 Wonton 的 Rust ACP bridge；`agent` 运行固定版本的 `omp 18.16.0 acp`。两个服务只通过专用 Unix socket 卷和插件网络通信。

## 构建镜像

在仓库根目录执行：

```bash
docker build -f deploy/dev/acp-bridge/Agent.Dockerfile -t wonton-acp-agent:dev .
docker build -f deploy/dev/acp-bridge/Dockerfile -t wonton-acp-bridge:dev .
```

## 由核心安装

先起开发核心，再安装本目录。核心代 Bot 登录、建加密私聊，并注入 Matrix、模型网关、MCP 和 Bot 数据目录变量：

```bash
deploy/dev/core/dev.sh up
deploy/dev/core/dev.sh install deploy/dev/acp-bridge --yes
```

卸载并删除样例数据：

```bash
deploy/dev/core/dev.sh uninstall dev.wonton.acp-bridge --purge
```

## 独立 Compose

自己准备 Bot 数据目录，填入真实的 Wonton 变量；Compose 会建自己的网络：

```bash
export WONTON_BOT_DATA="$PWD/deploy/dev/acp-bridge/.data"
mkdir -p "$WONTON_BOT_DATA"
export WONTON_MATRIX_HOMESERVER=http://matrix.wonton.internal
export WONTON_MATRIX_ACCESS_TOKEN='…'
export WONTON_MATRIX_OWNER='@owner:wonton.localhost'
export WONTON_MATRIX_HOME_ROOM='!room:wonton.localhost'
export WONTON_MODEL_BASE_URL=http://models.wonton.internal/v1
export WONTON_MODEL_API_KEY='…'
export WONTON_MODEL_DEFAULT=group/wonton-default

docker compose -f deploy/dev/acp-bridge/docker-compose.yml up -d
```

`WONTON_MCP_URL` 默认使用核心插件网络里的 `mcp.wonton.internal`。模型请求由 Agent 直接访问 `WONTON_MODEL_BASE_URL`；本样例不提供宿主端口转发、不绕过防火墙，也不把 Matrix 令牌或 bridge 数据目录挂给 Agent。

`WONTON_ACP_MODE` 默认 `plan`，是 OMP 在 `session/new` 之后切过去的只读模式。换 Agent 时填该 Agent 实测过的最严格模式。`WONTON_ACP_TURN_SECONDS`、`WONTON_ACP_IDLE_SECONDS`、`WONTON_ACP_MAX_INPUT_BYTES`、`WONTON_ACP_MAX_OUTPUT_BYTES`、`WONTON_ACP_MAX_QUEUE` 可改单轮时限、空闲关闭时限和输入输出/队列上限。

聊天里普通文字排队执行；`/wonton interrupt <文字>` 取消当前轮后重来，`/wonton queue <文字>` 显式排队，`/wonton steer <文字>` 在 Agent 声明 steering 时插话。Agent 请求自身工具权限时，桥经核心的确认卡片接口等主人回答。

停止并清理本样例自己的资源：

```bash
docker compose -f deploy/dev/acp-bridge/docker-compose.yml down -v
rm -rf "$WONTON_BOT_DATA"
```

## 隔离

- `bridge` 只挂 `${WONTON_BOT_DATA}/bridge`（权限 `0700`）和 `acp-socket`，并持有 Matrix 令牌。
- `agent` 只挂 `${WONTON_BOT_DATA}/agent`、`${WONTON_BOT_DATA}/workspace` 和 `acp-socket`；看不到 bridge 数据、Matrix 令牌和 crypto store。
- socket 卷只用于 Agent ACP stdio 与 bridge 的 Unix socket 转发，权限为 `0600`。
- OMP 版本和 Linux amd64/arm64 二进制 SHA-256 固定在 `Agent.Dockerfile`；上游许可证文件随样例分发。

## 验证边界

模型通路和完整 Wonton 加密私聊冒烟需要真实核心部署环境。npchy 的容器不能访问宿主 magpie 时，按 C2 任务要求记录为未测，不使用 socat、Node 或其他转发绕过。
