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

样例里四个空代理变量用于阻止 Docker CLI 注入客户端的代理配置；核心经 Engine API 安装插件不继承这份配置，见
[核心开发环境的代理说明](../core/README.md)。

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

OMP 18.16 不读取 `OPENAI_BASE_URL` 和 `OPENAI_MODEL`。入口脚本将这三个模型变量转成它的 `models.yml` 和 `config.yml`，并把 default、smol、slow、plan 角色固定到选定的 Wonton 模型；这两个文件由样例管理。模型凭据只以 `OPENAI_API_KEY` 环境变量名引用，不把值写进配置。

`WONTON_ACP_MODE` 默认 `plan`，是 OMP 在 `session/new` 之后切过去的只读模式。换 Agent 时填该 Agent 实测过的最严格模式。`WONTON_ACP_TURN_SECONDS`、`WONTON_ACP_IDLE_SECONDS`、`WONTON_ACP_MAX_INPUT_BYTES`、`WONTON_ACP_MAX_OUTPUT_BYTES`、`WONTON_ACP_MAX_QUEUE` 可改单轮时限、空闲关闭时限和输入输出/队列上限。

聊天里普通文字排队执行；`/wonton interrupt <文字>` 取消当前轮后重来，`/wonton queue <文字>` 显式排队，`/wonton steer <文字>` 在 Agent 声明 steering 时插话。Agent 请求自身工具权限时，桥经核心的确认卡片接口等主人回答。

Agent 经 ACP 报的命令表写成房间里 `app.wonton.bot.commands` 状态事件（`state_key` 是桥自己的 Matrix ID），`/wonton` 排在最前，表没变不重写，Agent 断开时回到只有 `/wonton`。写不进去（群里桥不是管理员）时桥在房间里提示一次。群里有别的 Bot 时，命令消息要提及桥才处理；私聊不用提及。

## 登录（C3）

Agent 声明了 ACP 登录方式时，聊天里发 `/wonton login` 列出方式，`/wonton login <方式 ID>` 发起；URL 引导转成聊天里的链接，回复 `/wonton login accept` / `/wonton login decline`；终端类登录的链接和提示发进聊天，主人回复的授权码原样写入登录进程 stdin。`/wonton login cancel` 取消。

- 入口脚本按 `WONTON_AGENT_COMMAND`（默认 `omp acp`）启动 Agent；终端登录用同一命令加登录方式声明的参数（ACP 要求同一程序）。
- Agent 声明 terminal 登录方式的前提是入口脚本监听登录控制 socket：`<WONTON_ACP_SOCKET>` 同目录、后缀 `.login.sock`（如 `acp.login.sock`）。桥检测到该文件才对 Agent 声明 `auth.terminal`。
- 终端登录成功（进程退出码 0）后桥退出重连，由 Compose 拉起；登录态在 Agent 数据卷里，重启后保留。
- 用官方账号登录的插件要申请 `network.egress`（ADR-0007）；本样例的 OMP 走模型网关，不涉及。

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

真实验收需要可访问的模型网关。改成直连宿主 magpie 时，样例另需声明 `network.egress`，否则插件隔离网络没有出站路由。
自行给服务配置代理时，还要把目标宿主地址或主机名加入 `NO_PROXY` 和 `no_proxy`。核心的 Compose 子集不接收
`extra_hosts`，安装包不能依赖这个字段。无法直连时如实报告，不使用端口转发绕过。

加密消息的房间密钥可能晚于事件到达；桥在正文解密后才记入消息去重，避免把同一条普通文字误报为不支持并永久跳过。
