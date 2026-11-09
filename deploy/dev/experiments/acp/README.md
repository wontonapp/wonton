# ACP 桥验证实验（#43）

结论见 ADR-0007“验证结果（#43）”。这里是复跑方法。实验不进主工作区和 CI。

每个 Agent 装进一个最小的“Agent 插件容器”（`node:24-bookworm-slim` 加一个说 ACP 的程序，版本全部固定），
探针 `probe/probe.mjs` 在同一个容器里像编辑器一样经标准输入输出说 ACP（不用 SDK，原始报文全部记进
`out/*.trace.jsonl`）。HOME 和工作区是持久卷，模拟插件的数据目录；每个场景一个新容器。

| 目录 | 作用 |
| --- | --- |
| `Dockerfile` | 每个 target 一个运行时：`claude`、`codex`、`kimi`、`qwen`、`qoder`，以及 `oar`（OAR 加 claude、codex） |
| `probe/probe.mjs` | ACP 客户端和验证场景；`probe/rpc.mjs` 是 JSON-RPC 连接和进程树内存采样 |
| `probe/summary.mjs` | 把 `out/` 里的结果汇总成 Markdown 表 |
| `probe/relay.mjs` | TCP 转发，模拟“插件网络是 internal，只连得上模型网关” |
| `agents/` | 各家指向模型网关的配置（Claude、Kimi、Qwen 用环境变量，见 `run.sh` 的 `agent_env`） |
| `oar/oar-run.mjs` | OAR 直接驱动 claude、codex：插话、审批、登录和额度 |
| `oar/oar-acp.mjs` | 原型：把 OAR 包成一个说 ACP 的适配器，用同一个探针测 |

## 前提

- Docker（本机验证用 OrbStack，arm64，Engine 29.8）。镜像共约 4 GB，测完 `run.sh clean`。
- 开发机的 magpie 网关（容器里是 `http://host.docker.internal:3425`，OrbStack 上任意 key 都行）。
  模型名经 `MODEL` 传入（要一个 Chat Completions、Anthropic Messages、Responses 三种接口都通的模型）；
  `MODEL_URL`、`MODEL_KEY` 可以覆盖。只经网关调用，不读 magpie 的配置。

## 复跑

```bash
deploy/dev/experiments/acp/run.sh build                 # 构建全部镜像，打印每个 agent 层的大小
deploy/dev/experiments/acp/run.sh matrix                # 每个 agent 跑全部场景，最后打印汇总表
deploy/dev/experiments/acp/run.sh claude permission --mode default   # 单跑一个场景
deploy/dev/experiments/acp/run.sh offline codex         # 插件网络 internal，只经转发连模型网关
deploy/dev/experiments/acp/run.sh codex login           # 经 ACP 发起 ChatGPT 设备码登录，拿到链接和码后拒绝
deploy/dev/experiments/acp/run.sh oar claude approval   # OAR 直接驱动：steer、approval、account
deploy/dev/experiments/acp/run.sh oar-acp codex steer   # OAR 包成 ACP 适配器后跑探针场景
node deploy/dev/experiments/acp/probe/summary.mjs deploy/dev/experiments/acp/out
deploy/dev/experiments/acp/run.sh clean                 # 删掉实验的容器、卷、网络和镜像
```

`OUT` 指定结果目录（默认本目录的 `out/`，已忽略）。

| 场景 | 做什么 |
| --- | --- |
| `init` | `initialize`（声明终端登录、URL 引导、自定义网关能力，看各家给哪些登录方式）、`session/new`、`providers/list` |
| `chat` | 记住一个暗号，下一轮问回来；会话 id 写进 `out/state/` |
| `load`、`resume` | 新容器里 `session/load`（回放历史）或 `session/resume`（不回放），再问暗号 |
| `redo` | 打断重来：长任务输出中 `session/cancel`，再带新消息提示，看历史是否保留 |
| `busy` | 不用扩展，一轮进行中再发一个 `session/prompt`，看 agent 怎么处理 |
| `steer` | 插话扩展 `_session/steering`：进行中注入、空闲时 `idleBehavior: promptRequired` |
| `permission` | 改文件允许、跑命令拒绝、改文件时取消（等确认时 `session/cancel`）、跑命令允许；`--mode` 选模式 |
| `sessions` | 一个进程里开 5 个会话（一个房间一个会话），每开一个采一次内存 |
| `login` | 声明 URL 引导能力，发起设备码登录，记下 `elicitation/create` 给的链接和提示后拒绝 |

## Qoder

Qoder CLI 只能用 Qoder 账号，没有账号时 `session/new` 报 `Authentication required`。主人在
<https://qoder.com/account/integrations> 建一个个人访问令牌（PAT），自己导出后再跑（值不出现在命令行上）：

```bash
export QODER_PERSONAL_ACCESS_TOKEN=...   # 主人自己输入
deploy/dev/experiments/acp/run.sh build qoder
deploy/dev/experiments/acp/run.sh matrix qoder
```

如果仍报需要登录，给单个场景加 `--auth qodercli-login`。

## 依赖与许可证

全部作为独立容器运行，不链接进 Wonton 的二进制；探针本身没有 npm 依赖。

| 依赖 | 版本 | 许可证 |
| --- | --- | --- |
| `@agentclientprotocol/claude-agent-acp` | 0.98.0 | Apache-2.0；内含的 Claude Agent SDK 和 claude 程序是 Anthropic 专有许可 |
| `@agentclientprotocol/codex-acp` | 2.3.0 | Apache-2.0；内含 `@openai/codex` 0.164（Apache-2.0） |
| `@moonshot-ai/kimi-code`（Kimi Code CLI） | 2.4.0 | MIT |
| `@qwen-code/qwen-code` | 0.26.2 | Apache-2.0 |
| `@qoder-ai/qodercli` | 1.1.80 | 包里附 Apache-2.0 文本，package.json 没写许可证，源码未公开 |
| `@botiverse/oar` | 0.75.0 | Apache-2.0；125 个间接依赖为 MIT、Apache-2.0、BSD-3-Clause、ISC、BlueOak-1.0.0、Unlicense、0BSD，没有 GPL/AGPL（`oar/package-lock.json`） |
| `@anthropic-ai/claude-code`、`@openai/codex`（`oar` 镜像） | 2.1.312、0.165.0 | Anthropic 专有许可；Apache-2.0 |
