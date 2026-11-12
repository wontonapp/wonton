# ADR-0007 Agent 与 Bot：Agent 即插件

- 状态：已接受（2026-10-28）。Agent 的总体边界和 Bot 关系见本文；ACP 桥的实现形态见 [ADR-0035](0035-acp-bridge.md)。经桥登录的条款限制见 [ADR-0034](0034-onboarding-and-help.md)；家人的 Bot、Bot 听谁的和 Bot 之间的规则由 [ADR-0045](0045-family-members.md) 细化；家人账号的角色和按角色给 AI 设的上限见 [ADR-0088](0088-family-accounts.md)，不等 M2

## 背景

旧方案把 Hermes 当成唯一的 Agent，写了约 2.2 万行适配器和 2 万行测试，专门做执行轨迹、事件帧、
恢复状态等展示。新产品的形态更接近 MuseBot 这类个人 Agent：只收发消息，不展示详细的执行过程。
Agent 内核也要能替换，比如换成 Codex。

Hermes 上游已经自带 Matrix 等 20 多个 IM 平台、MCP、cron，以及 Bot Mode：一个 profile 就是一个
有独立模型、记忆、技能和定时任务的 Bot，多个 Bot 可以组群协作。

## 决策

- **Bot = 一个 Matrix 身份 + 一个驱动它的 Agent 插件。** 核心负责给 Bot 分配身份、权限、模型访问
  （magpie）和工具（MCP），不和任何具体 Agent 耦合。
- **接入标准**：
  - 会说 Matrix 的 Agent 直接接入，如 Hermes 的 Matrix 平台。
  - 支持 ACP（Agent Client Protocol）的 Agent 通过通用的“ACP 桥”插件接入，如 Codex、Claude Code、
    Gemini CLI、OpenCode，以及 Hermes 自带的 ACP 适配器。一个聊天房间对应一个 ACP 会话；消息、工具和权限的转法见
    [ADR-0035](0035-acp-bridge.md)。
- **默认 Agent**：Hermes，走 Bot Mode，一个 profile 对应一个 Bot；只收发消息，不做执行轨迹和工具
  过程展示。
- **家人**：每个家人有自己的私人 Bot（独立记忆，互不可见）；主人可以建共享 Bot，拉进家庭群（[ADR-0088](0088-family-accounts.md)，不等 M2）。
- **Bot 之间通信**：同机走 Matrix 私聊或群聊；跨机做成独立插件（Matrix 联邦走 iroh，或 A2A 协议）。
- **权限**：
  - 私人 Bot 只接受主人的指令。
  - 其他 Bot 要经主人同意才能加为联系人；跨机还要先配对两台服务器。
  - 来自其他 Bot 的消息一律视为不可信输入，涉及敏感工具必须由主人确认。
  - 设频率和转发跳数上限，防止 Bot 之间无限对聊。跳数只数不同 Bot 之间的来回，一个 Bot 自己连着说不算，到顶时问主人要不要继续（ADR-0045）。

## 理由

- Matrix 和 ACP 都是开放标准，一个桥就能接入一整类 Agent，不用为每个 Agent 写适配器（ADR-0015）。
- 只收发消息的形态大幅减少客户端和适配层的代码量。

## 范围

默认 Agent 是 Hermes。ACP 桥和 M1 同时开工，M1 验收不等它（ADR-0034）；家人账号和家人 Bot 见 ADR-0088，不等 M2；Bot 互发消息在 M2，接口预留；跨机通信在 M3。

## 代价与风险

- ACP 面向编辑器场景，Agent 要有一个受限的工作区。实测桥不用提供文件系统和终端能力，Agent 用自己容器里的，
  容器就是这个受限的工作区（见“验证结果（#43）”）。
- 多个 Agent 同时运行会占用较多内存，资源少的设备（如 Pi 4GB）要限制数量。实测每个 Agent 进程空闲约
  260–450 MiB，Claude Code 每多一个活会话再加约 200 MiB。

## 补充：默认 Agent 的安装（2026-10-30）

一行安装、配对之后，聊天列表里就要有和 Hermes 的加密私聊（GOAL 的 M1 第 3 项）。

- **谁装、什么时候装**：核心。启动后在后台先装系统插件（模型网关）、对完模型凭据的账，接着装默认插件，不等配对：
  Hermes 的镜像约 4 GB，越早拉越好，主人装 App、扫码时它多半已经装好。没装好时插件列表里是“安装中”，聊天列表里
  还没有它；装好、健康检查通过（Hermes 连上了 Matrix）后，核心才把私聊交给主人，私聊这时出现在聊天列表里。早交出去
  的话，Hermes 启动前发进私聊的话它收不到：它丢掉启动前的消息，它的设备这时也还没有加密密钥（Hermes v0.22.5），App
  也看不出来。交私聊不需要主人先登录过 Matrix：核心代主人登录（JWT 登录时自动建账号）并加入，主人第一次登录时私聊
  已经在了，不用等配对或登录触发什么。装不上时不留私聊。
- **包从哪来**：核心镜像里的默认插件目录（`WONTON_DEFAULT_PLUGINS`，镜像里是 `/usr/share/wonton/default-plugins`），
  格式同系统插件的目录。核心不管目录里的包从哪来，换来源（如官方插件目录）不用改核心。
- **只装一次**：Hermes 是 `kind: agent` 的普通插件，不是规范里的 `kind: system`（系统插件每次启动都装回来，主人
  换不掉）。装好或主人卸掉之后核心记下，此后不再自动装：主人换成别的 Agent，核心不会把 Hermes 装回来。授予它清单
  声明的全部权限（`models`、`matrix.bot`、`tools`、`network.egress`），装核心就是主人的确认。
- **失败**：按原因重试或停下，等待时插件列表里如实显示异常和原因（如“拉取镜像失败（镜像）：Docker 的原话”），
  局域网状态页上也看得到，规则在插件规范的“预装插件”。
- **没有模型时**：主人还没在“模型设置”里加 provider，Hermes 直接提示去加，不把网关的报错发进聊天，见 ADR-0008 的
  “补充：默认模型”。

没选的做法：

- 安装脚本装：核心升级、重启时没人补装，失败重试也要在脚本里另写一套。
- 第一次配对后装：主人要在 App 里干等镜像下载，核心还要多一条“配对完成”的触发路径。

### 验证结果（2026-10-30）

环境：Tuwunel v1.9.4、Hermes Agent v0.22.5、magpie 0.1.1327，macOS + OrbStack（Docker 29.8.2，arm64），镜像都已在
本机。可复跑：`deploy/dev/core/smoke.sh` 的“已配对设备经隧道登录 Matrix”“Hermes”和“核心重启后恢复插件”三步；重试、
只装一次和主人卸掉在假 Docker 上测（`cargo test -p wonton-core preinstall`）。

- 核心启动后 2 秒装好 magpie，十几秒装好 Hermes，都是第一次就装上。Hermes 第一次过健康检查后 0.4 秒，主人加入了
  私聊（Docker 的健康事件对 Tuwunel 的事件时间，同一台主机）。
- 主人的 m.direct 里有这个私聊，主人和 Hermes 都在里面，房间是加密的。全程没有手动安装 Hermes，也没有手动建路由组。
- 主人卸掉 Hermes（删除数据）后重启核心，日志写着“默认插件装过一次，不再自动装”，插件表和 Docker 里都没有它。
- 拉镜像：在一台没有 Hermes 镜像的 Linux 主机上（amd64，Docker 29.7.2，经 HTTP 代理出网）拉 Hermes v0.22.5 的镜像，
  两次分别用了 260 秒和 305 秒，下载 1.06 GB，解开后 4.27 GB。加上镜像在本机时的十几秒，全新主机上 Hermes 从开始拉到
  私聊出现约 5 分钟，主要看网速。没测：这段时间 App 里看到的样子。
- 拉不下镜像、局域网状态页在假 Docker 上测（`cargo test -p wonton-core image_that_cannot`、`lan_page`）：原因是
  “预装失败（已试 N 次，稍后自动重试）：拉取镜像失败（镜像）：Docker 的原话”，状态页上写着同样的原因，镜像拉得下来后
  自动装上。真实的 Docker 29.8.2 拉不到镜像时在开始下载前就回错误：镜像不存在是 404，仓库拒绝、连不上是 500。

## 补充：ACP 桥的形态（2026-11-05）

ACP 桥的 C1 设计、容器边界、Matrix/ACP/MCP 转换和 C3 登录路径见 [ADR-0035](0035-acp-bridge.md)。桥是独立插件，核心只提供标准接口，不认识具体 Agent。

- 桥只说 ACP；OAR 不作为桥，也不作为默认适配器。#43 的验证结论和依据保留在下面。
- Hermes 继续使用自带的 Matrix Bot 模式；ACP 桥不改变它的消息和执行轨迹取舍。

## 验证结果（#43，2026-10-28）

**结论：桥只接原生 ACP 和 ACP 组织的适配器，不采用 OAR，也不把它包成适配器。**

- Claude Code、Codex、Kimi Code、Qwen Code 经 ACP 就有桥要的全部能力：续聊、在新容器里恢复会话、打断重来、
  权限请求映射成允许、拒绝、取消三种回答；四家都能在只连得上模型网关的容器里运行。
- Qoder 只能用 Qoder 账号，ACP 握手正常，会话没测（要主人的账号）。
- 真插话只有 Claude、Codex 的适配器支持，两家已经实现了同一个扩展 `_session/steering`，桥直接采用。
- OAR 0.75 对任何运行时都不回答原生审批，Claude 固定带 `--dangerously-skip-permissions`，所以不能用在要主人
  确认的地方。它补的插话、登录、额度三项，ACP 这边已经有，或者对我们用处不大。

环境：本机 macOS + OrbStack（arm64，Docker 29.8）。每个 Agent 一个容器（`node:24-bookworm-slim` 加一个说 ACP
的程序），HOME 和工作区放在持久卷里，每个场景一个新容器。模型经开发机的 magpie（网关默认模型）。
版本：

- `claude-agent-acp` 0.98.0（内含 Claude Agent SDK 0.3.310）；
- `codex-acp` 2.3.0（内含 codex 0.164）；
- Kimi Code CLI 2.4.0（`@moonshot-ai/kimi-code`，命令 `kimi acp`；Python 版 kimi-cli 已于 2026-09 归档）；
- Qwen Code 0.26.2、Qoder CLI 1.1.80、OAR 0.75.0。

客户端是一个不用 SDK 的最小 ACP 客户端，原始报文全部留档。复跑：`deploy/dev/experiments/acp/`（`run.sh matrix`，
约 30 分钟），镜像和依赖的许可证写在该目录的 README。

### ACP 路线

| | Claude Code | Codex | Kimi Code | Qwen Code | Qoder |
| --- | --- | --- | --- | --- | --- |
| 续聊 | ✅ | ✅ | ✅ | ✅ | 未测 |
| 新容器里 `session/load`（回放历史） | ✅ | ✅ | ✅ | ✅ | 未测 |
| 新容器里 `session/resume`（不回放） | ✅ | ✅ | ✅ | ✅ | 未测 |
| 打断重来：`session/cancel` 到停下 | 46 ms | 17 ms | 16 ms | 36 ms | 未测 |
| 打断后重发提示，历史还在 | ✅ | ✅ | ✅ | ✅ | 未测 |
| 一轮进行中再发 `session/prompt` | 适配器排队 | 适配器排队 | 拒绝（`turn.agent_busy`） | 前一轮报 -32603 中断 | 未测 |
| 插话扩展 `_session/steering` | 立即生效 | 立即生效 | 不支持 | 不支持 | 不支持 |
| 改文件，允许一次 | 问，写入 | 问，写入 | 问，写入 | 问，写入 | 未测 |
| 跑命令，拒绝 | 不执行，本轮继续 | 不执行，本轮结束 | 不执行，本轮继续 | 不执行，本轮继续 | 未测 |
| 等确认时取消 | `cancelled`，没执行 | 同左 | 同左 | 同左 | 未测 |
| 跑命令，允许一次 | 问，执行 | 问，执行 | 问，执行 | 问，执行 | 未测 |
| 默认模式 → 最严格的可用模式 | `default` → 同 | `agent`（自动审核）→ `read-only` | `default` → 同 | `auto`（模型分类器自动批准）→ `default` | — |
| 只连得上模型网关 | ✅ | ✅ | ✅ | ✅ | — |
| 镜像里的 agent 层 | 368 MB | 422 MB | 63 MB | 297 MB | 100 MB |
| 内存：空闲 / 单会话峰值 | 311 / 483 MiB | 264 / 326 MiB | 406 / 493 MiB | 445 / 555 MiB（见下） | 启动 800–900 MiB |
| 每多一个活会话 | +195 MiB | +10 MiB | 约 0 | 约 0 | — |
| ACP 里的登录方式 | 终端登录（订阅或 Console）、自定义网关 | ChatGPT 设备码、API key、自定义网关 | 终端登录（设备码） | 只有 API key | 先在外面 `qodercli login` 或用 PAT |
| 许可证 | 适配器 Apache-2.0，Claude Code 专有 | Apache-2.0 | MIT | Apache-2.0 | 附 Apache-2.0 文本，源码未公开 |

“最严格的可用模式”指还能干活的模式；Claude、Kimi、Qwen 的 `plan` 模式更严，但只读、不执行。

**会话与打断**

- 恢复会话：四家都支持 `session/load` 和 `session/resume`，新容器里恢复只要几十到 200 ms，`session/list` 也能列出。
  前提是 HOME 和工作区都在持久的数据目录里：工作区不在时 `session/load` 报 -32602（`cwd` does not exist）。
  会话、配置、登录态都在 ACP Agent 自己的 `${WONTON_BOT_DATA}/agent` 下；workspace 与 HOME 一起持久化在那里。bridge 服务另用 `${WONTON_BOT_DATA}/bridge` 保存 Matrix crypto store、恢复密钥和房间状态，Agent 服务看不到它。
- 打断重来：发 `session/cancel`，等原来的 `session/prompt` 以 `cancelled` 返回，再发新提示。四家都在 50 ms 内停下，
  历史保留。Codex、Qwen 偶尔在 `cancelled` 之后 1 秒内还到一条文字片段，桥以 `session/prompt` 的响应为界丢弃。
- 忙时再发提示，各家行为不一致：Claude、Codex 的适配器自己排队；Kimi 直接拒绝；Qwen 会中断前一轮，前一轮报
  内部错误。所以桥必须自己排队，一个会话同时只发一个 `session/prompt`。

**权限请求映射成确认按钮**

- 四家都给出同样三个选项：`allow_once`、`allow_always`、`reject_once`，没有 `reject_always`。桥按 `kind` 决定语义，
  按钮文字用 agent 给的 `name`：“本会话都允许”的范围各家不同。比如 Claude 是“允许本会话所有编辑”或
  “允许 `echo hello *` 这类命令”，Codex 是“这些文件不再问”。
- 请求里的 `toolCall` 是对已报过的工具调用的补充，桥要按 `toolCallId` 合并。Claude、Qwen 在请求里就带命令或 diff；
  Codex 改文件时只带路径，diff 在之前的 `tool_call` 里；Kimi 的请求只有标题和一句说明，参数在之前流式的
  `tool_call_update` 里。
- 拒绝的语义不同：Claude、Kimi、Qwen 继续这一轮，回复“已拒绝”后 `end_turn`；Codex 的拒绝是
  “No, and tell Codex what to do differently”，这一轮以 `cancelled` 结束，等主人的新指示。桥不能把它当成失败。
- 取消：等确认时发 `session/cancel`，并对挂起的请求回答 `cancelled`。四家都以 `cancelled` 结束，什么都没执行。
- 模式要由桥显式设：Codex 默认 `agent`（自动审核），Qwen 默认 `auto`（模型分类器自动批准），都不是最严格的。
  桥在 `session/new` 之后切到上表的最严格模式（切法见 ADR-0035），聊天里不提供切到 bypass、yolo、full-access 的入口。
- Claude 适配器启动 Claude Code 时总带 `--allow-dangerously-skip-permissions`。这个参数不跳过权限，只是允许之后
  切到 `bypassPermissions`。镜像里放 managed settings（`/etc/claude-code/managed-settings.json` 写
  `permissions.disableBypassPermissionsMode: "disable"`）后，切换被拒（实测）。全程没有用
  `--dangerously-skip-permissions`。
- Codex 自己的沙箱（bubblewrap）在默认的 Docker 容器里建不了命名空间：`workspace-write` 模式下的命令要么失败，
  要么转成权限请求。容器本身就是沙箱，Codex 用 `read-only`，改文件和跑命令都先问。
- 桥不声明 `fs`、`terminal` 能力，各家都用自己容器里的文件和终端，桥不用提供受限的工作区。

**容器、网络与内存**

- 安装：都是 `npm install -g` 一个包，Claude、Codex 的原生程序随包按平台带上。
- 出网：插件网络是 internal、只经转发连得上模型网关时，四家都正常（`run.sh offline`）。用模型网关时 Agent 插件
  不需要 `network.egress`；只有用官方账号（订阅、设备码登录）时才需要。
- 模型网关：四家都能经环境变量或配置文件指向 magpie。Claude 用 Anthropic Messages，Codex 用 Responses，
  Kimi、Qwen 用 Chat Completions。Claude、Codex 的适配器还能经 ACP 配置网关：`authenticate` 的 `gateway` 方法
  （`_meta.gateway.baseUrl`、`headers`）和不稳定的 `providers/set`（`providers/list` 实测可用，另两个只读了代码）。
  Kimi、Qwen、Qoder 不支持，所以插件规范仍以环境变量为准。
- Codex 遇到目录里没有的模型时，每一轮都会把一条“Model metadata … not found”警告当回复文字发出来。接网关模型要在
  配置里补上模型元数据（`model_catalog_json`，没验证），否则这句话会进聊天。
- 内存：Claude 适配器给每个活会话起一个 claude 进程（每个约 200 MiB）；Codex 一个 app-server 承载所有会话；
  Kimi、Qwen 一个进程。一个房间一个会话时，桥应在房间空闲一段时间后 `session/close`，下条消息再 `session/resume`。
- Qwen 观察到瞬时峰值：同一进程里第三个会话开始一轮时，内存 0.6 秒内冲到 2.7–2.9 GiB，然后回落。复现了 4 次，
  单独跑一个会话时不出现，原因没查。Qwen 插件要设内存上限，并验证不会因此被杀掉。
- Qwen 的 `auto` 模式会自动把“记住暗号”写进 HOME 下的全局记忆，于是同一个 Bot 的所有房间共享这份记忆。
- 这几家 Agent 都只说 stdio（ACP 的 HTTP、WebSocket 远程传输还是 RFD 草案）；C1 的两服务方案和 Agent 镜像入口里的 stdio↔socket 转发见 [ADR-0035](0035-acp-bridge.md)。

**登录（不经终端）**

都只验证到拿到登录链接和码为止，没有用任何账号完成登录：

| | 方式 | 结果 |
| --- | --- | --- |
| Codex | ACP 原生：`authenticate("chat-gpt-device-code")`，适配器发 `elicitation/create`（URL 模式），给出 `https://auth.openai.com/codex/device` 和设备码；要求客户端声明 `elicitation.url` | 拿到链接和码，拒绝后登录取消（-32800） |
| Kimi | ACP 的终端类登录：命令 `kimi login`，不需要 TTY，纯文本打出授权链接和设备码（30 分钟有效） | 拿到链接和码 |
| Claude | ACP 的终端类登录，客户端声明 `auth.terminal` 才提供：`claude auth login` 打出授权链接，再从标准输入读回授权码 | 拿到链接 |
| Qwen | 只有 API key（Qwen OAuth 已移除） | — |
| Qoder | ACP 里没有登录流程，只能先 `qodercli login`（浏览器）或设个人访问令牌 | 没测 |

桥可以做成两条通用路径：URL 引导（`elicitation/create`）直接转成聊天里的链接卡片；终端类登录在容器里起那条命令，
把链接和码转给主人，主人回的授权码写回标准输入。这就是 OAR `runtime.login` 做的事，不需要 OAR。
订阅账号经第三方产品使用是否合规，要另看各家条款（Antigravity 明确不允许）。

**事件到 Matrix**

- 每轮有几十到几百条 `agent_thought_chunk`，另有工具调用、`usage_update`、`available_commands_update`、
  `session_info_update` 等。
- 桥默认只在一轮结束时把 `agent_message_chunk` 拼成一条消息发进房间，和 Hermes 的 Bot 模式一致；一轮进行中
  发输入状态（`m.typing`），不额外发“正在处理”。权限请求发确认卡片；其余事件不发。
- 子 Agent：Claude、Codex 的适配器只在客户端声明 `clientCapabilities.subagents` 时才把子 Agent 作为单独会话报上来。
  桥不声明，子 Agent 的工作就只是根会话里的工具调用，默认不发。

### 插话扩展：采用现成的 `_session/steering`

claude-agent-acp 和 codex-acp 实现了同一个约定（两家都写在各自的 AIR 扩展文档里），桥直接用，不另起
`_wonton/` 方法：

- 能力：`initialize` 响应顶层 `_meta.steering.supported: true`。
- 请求：`_session/steering`，参数 `{sessionId, prompt: ContentBlock[], _meta: {steering: {idleBehavior: "promptRequired"}}}`。
- 结果：`injected` 表示并入进行中的这一轮，原来那个 `session/prompt` 照常以这一轮的结束返回；`promptRequired`
  表示 agent 空闲，由桥用 `session/prompt` 发。
- 桥只在自己有一轮进行中时发。codex-acp 2.3.0 不认 `idleBehavior`（codex-acp#440），空闲时会返回 `startedNewTurn`、
  自己开一轮，而这一轮没有对应的 `session/prompt` 响应。桥怎么处理见 ADR-0035。
- 不支持的运行时（Kimi、Qwen、Qoder）由桥攒着，这一轮结束后合成一轮发（ADR-0035）。Qwen 有私有的插话方法
  （`craft/drainMidTurnQueue`、`craft/midTurnSendNow`，不带 `_` 前缀，不符合 ACP 的扩展约定），不接。
- ACP v2 草案把 `session/prompt` 改成“插入即返回”，但仍没规定排队和插话；claude-agent-acp 的插话只在 v1 上。
  v2 定稿后再看是否迁移。

### OAR 路线（0.75.0）

OAR 直接驱动 claude 2.1.312、codex 0.165 实测，其余运行时按 OAR 自己的文档（runtime matrix）：

- **审批：不能由我们回答，排除。**
  - Claude：启动参数固定带 `--dangerously-skip-permissions`；`launchArgs` 里带 `--permission-mode default` 会被拒：
    “OAR runs claude with --dangerously-skip-permissions, which overrides it”。
  - Codex：以 `-c sandbox_mode="danger-full-access"`、`approvalPolicy: never` 启动。
  - 两家都没发审批请求，命令直接执行、写了文件。
  - 按文档，Kimi、Qwen 用 yolo，万一来了审批请求 OAR 也自动允许；Grok 带 `--always-approve`。
    OAR 的原话是“OAR allows every tool by design (host approvals deferred)”。
- **插话**：
  - Codex 经 `turn/steer` 真插话，立即生效。
  - Claude 是把消息写进 stdin：纯文本生成时要等这一轮写完才处理，实测数完 300 行才回答，等于排队。
    claude-agent-acp 的 `_session/steering` 用 `priority: now` 打断生成，反而更好。
  - 按文档，Kimi、Qwen、Antigravity 在 OAR 里也没有插话；Qoder、Grok 有。
- **登录、额度**：
  - `runtime.login` 只支持 Claude、Codex、Cursor。经 magpie 时 `accountUsage` 两家都返回 unsupported：
    额度只对官方订阅有意义。
  - 登录 ACP 这边已有（见上），不值得为它多一个 Node 24 进程。
- **接入没有 ACP 的运行时**：补充里举的 Grok Build、Antigravity，OAR 内部其实也是经 ACP 驱动的（和 Kimi、OpenCode
  共用 ACP 传输），可以直接接桥。只靠 OAR 才能驱动的是 Cursor、Pi 这类经 SDK 在进程内跑的，不在计划里。
- **维护**：2026-08-26 首发，到 10-27 已发 133 个版本，其中 10 月 103 个；DeepSeek harness 打补丁的 0.67.0
  是 10-17 发的，十天后已到 0.75.0。

### 把 OAR 包成 ACP 适配器的成本

原型 `oar/oar-acp.mjs`（约 130 行）用同一个探针测：

- 续聊、`session/resume`、打断重来、在适配器里排队都能做。
- 插话：Codex 真插话；Claude 返回 `injected`，实际仍要等这一轮写完。
- 审批：一个 `session/request_permission` 都发不出来，允许、拒绝、取消三种回答下文件都被写了。
- 代价：
  - 进程：多一个 Node 宿主进程（OAR 约 170 MiB）；Codex 每个会话起一个 app-server（3 个会话 737 MiB，
    codex-acp 约 340 MiB）。
  - 镜像：多约 180 MB（125 个依赖，含 Pi 编码 Agent 和各家 SDK，许可证都是宽松许可）。
  - 维护：还要跟上 OAR 的快速迭代。

包一层只换来更弱的插话、没有审批，所以不做。重新评估的条件：OAR 提供宿主审批通道，并且出现了只有 OAR 能驱动、
没有 ACP 实现的运行时。

### 没验证、需要主人的

- Qoder：要一个 Qoder 账号的个人访问令牌，主人自己导出后复跑（步骤见实验目录的 README）。
- 订阅登录的完整流程（Claude 订阅、ChatGPT、Kimi 会员）：只验证到拿到链接和设备码，没有登录，也就没验证登录态在
  容器重启后保留。
- Qwen 的瞬时内存峰值：原因没查，在设了内存上限的容器里复测。
