# ADR-0061 Agent 的运行时和前端分开：运行时经一道 ACP 网关提供能力，聊天、网页、看板都是前端

- 状态：已接受（2026-11-08）
- 修订：[ADR-0035](0035-acp-bridge.md) 的桥的拆分（见第 5 节）；[ADR-0058](0058-agent-side-sessions.md) 的“改同一个文件时排队”挪到网关
- 2026-11-11 修订（主人定）：`agent_acp` 双向，Agent 既能被调也能调别人；编排器插件（[ADR-0059](0059-team-ops-plugins.md)）是使用方；
  Wonton MCP 加“交给另一个 Agent”“给某人发消息”两个工具；调用链有层数、绕圈、轮数和花费的限制，按链记账，随时能停整条链（第 9、10 节）；
  网关和桥用官方的 Rust ACP 库（第 11 节）。被调的 Agent 在房间里怎么出现见 [ADR-0083](0083-bot-contacts-and-group-chat.md)（#248）
- 延伸：运行时的 Agent 进程可以在用户自己的电脑上，见 [ADR-0081](0081-local-agent-host.md)

## 背景

主人（2026-11-07）：“runtime 插件跑的是 codex，但是前端自己写一个简单的对话 UI 插件，之类的都是支持的……因为解耦，维护起来，
甚至后续产品形态变更，也更容易，我们官方出一些插件就能迅速跟进。”

每个 Agent 插件把“跑 Agent”和“在云吞的聊天里说话”捆在一起：插件里有 `agent` 和 `bridge` 两个服务，桥经共享卷上的 unix socket
用 ACP 和 Agent 说话（ADR-0035）。别的界面（更简单的对话页、团队作战看板）想用这个 Agent，只能各自再捆一份。

ADR-0015 要求组件只经标准接口连接、每个都能单独替换。这篇把 Agent 拆成**运行时**和**前端**。接入现成编排器的插件（ADR-0059）
是 `agent_acp` 的使用方：经它指挥主人已装的 Agent，复用那些 Agent 已登录的账号和设置。Agent 自己也能经它把事交给另一个 Agent（第 9 节）。

## 决策

### 1. 接口是 ACP，能力名 `agent_acp`

- 前端说的是 [ACP](https://agentclientprotocol.com)：桥已经用它和 Codex、Claude Code、omp、Hermes 说话，编辑器也用它接各家 Agent。
- 运行时插件按 ADR-0053 声明 `provides: {agent_acp: …}`，前端声明 `requires: [agent_acp]`。`agent_acp` 进能力表，标为高权限，
  确认规则只照 ADR-0053 的“高权限能力”。
- 照 [新能力检查单](README.md#新能力检查单)：
  - **契约**：ACP（下面的传输）加核心注入的请求头（`X-Wonton-Plugin`、`X-Wonton-User`、`X-Wonton-Chain`、上限，第 3、10 节）。提供方必须支持
    `initialize`、`session/new`、`session/prompt`、`session/update`、`session/cancel`、`session/load`，并按第 2 节在网关里改写字段、回答权限请求；
    错误按类别返回：`permission_denied`、`busy`、`limit_reached`、`stopped`（链被人停了，第 10 节）、上游出错。
  - **提供方**：Agent 插件，网关在 Agent 底座里（ADR-0079），每家 Agent 一样。
  - **使用方**：编排器插件（ADR-0059）；核心替 Bot 调用（第 9 节的“交给另一个 Agent”）；以后的网页对话前端。
  - **契约测试**：`crates/wonton-core/src/plugin/tests/agent_acp_contract/`，照 `model_gateway_contract` 的样子。同一段使用方脚本
    （开会话、发一轮、权限请求变成卡片、取消、断线后 `session/load`、超上限被拒）对两个提供方各跑一遍：底座网关接假 ACP Agent，
    和一个只按契约写的最小提供方；断言使用方一侧的报文和结果一样，换提供方使用方不用改。
- **传输照官方的 [Streamable HTTP & WebSocket 传输 RFD](https://agentclientprotocol.com/rfds/streamable-http-websocket-transport)**（状态 Active）
  的 WebSocket 那一种：端点 `/acp`，`GET` 带 `Upgrade: websocket` 升级，升级的响应头里带 `Acp-Connection-Id`，每个文本帧一条 JSON-RPC 消息；
  一条连接是一个 ACP 客户端。认证 RFD 留给上层，我们的认证和 `X-Wonton-*` 头（第 3、10 节）就是那一层。
  前端连自己网络里的 `agent_acp.wonton.internal/acp`，核心转发给主人选定的运行时和 Bot。
  RFD 定的是断开即清理会话；断线后保留进程、重连 `session/load` 是我们加的（第 2 节）。

### 2. 运行时一侧有一道 ACP 网关，前端的话只经它

前端不直接对 Agent 说话。运行时插件里有一道**网关**，夹在前端和 Agent 进程之间，下面这些都在网关里做，前端怎么填都改不了：

- **`session/new` 的字段由网关定**：丢掉前端给的 `mcpServers`，换成运行时自己的 Wonton MCP 入口（不然 stdio 类型的条目就是在运行时容器里
  起任意命令）；`cwd` 固定为运行时配置的工作区（或其下按前端分的子目录），不接受前端给的；去掉前端的 `_meta`。
- **权限请求由网关回答**：Agent 的 `session/request_permission` 由网关转成核心的确认卡片（`POST /agent/permissions`），卡片写明是哪个前端、
  替谁在问；前端只收到“在等主人确认”。卡片过期时，网关先对这个会话发 `session/cancel`，再回 `cancelled`（照官方库 cookbook 的建议）。ADR-0058 的“改同一个文件时排队”也挪到网关：它看得到所有前端的所有会话改了哪些文件，
  排队因此跨前端也有效。
- **工作模式只许配置好的那一个**：模式 ID 是各家自定的字符串，分不出宽严，所以网关只放行配置里那个模式的 `session/set_mode`
  和类别为 `mode` 的 `session/set_config_option`，别的一律拒绝；要换由主人在运行时的设置里改。
- **会话按（前端、用户）归属**：别人的会话 `session/load`、`session/resume` 拒绝（`permission_denied`），`session/list` 只列自己的。
- **一个连接一个 Agent 进程**；运行时声明连接数上限，满了返回 `busy`。连接断开后进程保留 10 分钟，前端重连后 `session/load` 接着用
  （核心重启、升级时所有连接都会断）。这是我们在 RFD 之上加的：手机换网络重连时要靠 `session/load` 接回原来的会话。

网关用 Rust 写，从桥里拆出来（第 5 节），ACP 层用官方库（第 11 节）：桥已经做了权限卡片、模式切换、MCP 入口和工具调用合并，不在 Python 里重写一遍。

### 3. 身份由核心给，运行时只认这个 Bot 听的人

- 核心转发时带上 `X-Wonton-Plugin`（哪个前端）、`X-Wonton-User`（替谁说话），前端自己带的同名头丢掉。
  网页类前端经核心网关登录（OIDC），连运行时时带上这次登录的访问令牌，核心验过后换成 `X-Wonton-User`。
- **运行时只接受 `X-Wonton-User` 是这个 Bot 听的人的连接**，也就是 ADR-0045 的 `WONTON_MATRIX_ALLOWED_USERS`（联系人除外），别的拒绝：
  私人 Bot 只认归属人，共享 Bot 认所有家人。编排器、别的 Agent 连进来时算谁见第 10 节。和聊天里的共享 Bot 一样，工具调用按这个 Bot 自己的权限执行（主人开放给它的那些插件），
  不按连上来的人的身份，所以不用给总线加新机制。
- 一个 Bot 里按人分权限（同一个 Bot，家人 B 能用的工具比主人少）不做，要做另写 ADR。
- **运行时认得出连接是核心转发来的**：核心给运行时插件发一个提供方令牌（只注入网关所在的服务），转发时带在 `Authorization` 里，网关不认没有它的连接。
  同一插件里的其他服务、伪造请求头都连不进来。网关监听的端口只开在插件网络里。

### 4. 前端怎么选运行时和 Bot

- 选哪个运行时按 ADR-0053 的“默认应用”：每种能力一个全局默认，前端的详情页可以另指一个。
- 一个运行时插件可以有几个 Bot（ADR-0045），每个 Bot 是一份独立的运行时。前端详情页上主人选“用哪个 Bot”，一个前端可以绑几个。
  ADR-0053 的路由和确认记录按（使用方、提供方、能力）记，没有 Bot 这一维，要补上，归插件底座线改 ADR-0053。
- 只有主人在 App 里改绑定；Bot 不能替前端选、也不能换。

### 5. 桥拆成两半

| | 现在（ADR-0035） | 改成 |
|---|---|---|
| 权限卡片、模式、MCP 入口、改文件排队 | 在桥里 | 挪进运行时的网关（第 2 节） |
| Matrix、话题、工作板、斜杠命令、显示方式 | 在桥里 | 留在桥里，桥成为“云吞聊天”前端（ADR-0058、ADR-0054、ADR-0060） |
| 登录 | 桥连 `acp.login.sock` 起终端登录，链接和提示发进私聊 | 不变，只给同一个插件里的桥；不经 `agent_acp` 开放（否则任何前端都能替运行时重新登录官方账号，登录成功后容器重启还会断开所有前端） |

**桥放在哪**分两步：
1. 桥仍是 Agent 插件里的一个服务，经插件内的 unix socket 连本插件的网关，不经核心转发：桥只用本插件的 Bot，绕核心一圈没有好处。主人看不出变化。
2. 有了第二个前端、第 1 步跑稳之后，再看要不要把桥抽成独立的前端插件。这牵涉 Bot 账号归谁、多 Bot（ADR-0045、#683），到时另写。

### 6. 核心要补什么

都归插件底座线，main 上还没有：

- `<能力>.wonton.internal` 的转发（#582 第 2 步），加上 WebSocket 升级和不限时的流；
- 转发时注入 `X-Wonton-Plugin`、`X-Wonton-User` 和提供方令牌，丢掉前端带的同名头；
- ADR-0053 的路由加 Bot 这一维（第 4 节）。
- 调用链（第 10 节）：`POST /agent/chains`、`X-Wonton-Chain`、层数和绕圈检查、轮数和花费的计数、模型网关入口认链路径、停下整条链；
- 总线上的 `wonton__hand_off`（按调用 ID 幂等）、`wonton__hand_off_status`、`wonton__send_message` 和 `GET /agent/outbox`（第 9 节）；
- `agent_acp` 的契约测试（第 1 节）。

### 7. 安全提示要说全

确认卡片（装前端、换运行时或 Bot、新前端第一次连上，照 ADR-0053）写明：这个插件能指挥 <Bot> 执行命令、改文件，
**也能让它读出运行时容器里的任何东西**，包括官方登录的凭据和别的会话的历史。工作区子目录只是起始目录，不是沙箱。
共享 Bot 的运行时里有它的登录凭据和所有家人的会话记录，任何一个家人都能让它读出来。
运行时的数据前端拿不到的前提是前端不让 Agent 去读，挡不住。
Agent 插件 `wants: [agent_acp]` 时，确认页同样写明它能把事交给 <Bot>，被交的 Bot 能做的事它都能借来做（第 9 节）。

### 8. 官方插件怎么跟进产品变化

产品形态要变时，出一个新的前端插件，核心和运行时都不动：简单对话 UI（输入框和流式回复的网页插件）、团队作战（ADR-0059 的编排器插件，
用主人已装的 Agent 当工人，见第 9 节）。

### 9. 谁能调谁：`agent_acp` 双向

主人（2026-11-11）要“用户—Matrix—某 Agent”和“用户—Matrix—某 Agent—其他 Agent”都能走通，插件之间能互相调用和被调用，
也能自己写一个说 ACP 的编排器去调别的 Agent。核心不认“编排器”这个类别，下面的规矩对所有 `agent_acp` 的使用方一样。

- **Agent 插件既提供也使用**：清单写 `provides.agent_acp`，再写 `wants: [agent_acp]`（自己就能用，有别的 Agent 时才能交出去）。
  编排器插件既提供也使用：它在 Matrix 里就是一个 Bot，用户直接找它，也能被别的前端或 Agent 调（`provides.agent_acp`）；
  它经核心把活派给已装的 Agent（`requires: [agent_acp]`）。
- **ADR-0053 的规则跟着放开**：提供某个能力的插件可以 `requires` 或 `wants` **同一个**能力，核心给它选提供方时排除它自己，
  `requires` 要有别的插件提供才装得上；别的能力照旧不能 `requires`。插件之间成环（P、Q 都提供又都要 `agent_acp`）不用核心解依赖图：
  每个使用方仍只选一层提供方，调用时的绕圈由第 10 节按调用链拦。
- **不许调到自己**：目标 Bot 就是发起的 Bot 时拒绝（`invalid_input`）。同一个插件的另一个 Bot 可以调，经核心转发，和别的插件一样。
- **Wonton MCP 加两个核心工具**（ADR-0016 的总线，确认规则照它）：
  - `wonton__hand_off`（交给另一个 Agent）：参数是目标 Bot 和要交代的话。调用方的插件要 `wants` 或 `requires` 了 `agent_acp`。
    核心作为这个插件的使用方，经 `agent_acp` 给目标 Bot 开一个新会话（第 2 节的网关照常把关），会话属于第 10 节的同一条调用链，
    并带上发起的对话（房间、话题）。工具开好会话就立即返回会话 ID 和链 ID，不等 B 做完；B 的进度和结果由 B 自己在对话里说。
    - **不重复交接**：Alma 出过同一条消息派了两次活（[alma-releases#163](https://github.com/yetone/alma-releases/issues/163)），下面两条都返回已开的那个会话，不开新的：
      - 同一个调用方会话里，MCP `tools/call` 的请求 ID 相同的重试或重复调用；
      - 同一个调用方会话交给同一个目标 Bot、交代的话一字不差，且它之前交出的那个会话还在做。只比原文，不做模糊匹配。
    - **B 结束时核心不叫醒 A**，也没有“交回结果”的机制：A 要知道进展，用下面的 `wonton__hand_off_status` 查，或者等 B 点名。
      要 A 接着干，B 在对话里叫 A 的名字，规则见 [ADR-0083](0083-bot-contacts-and-group-chat.md)（#248）第 4 节的“互相认名字”。
      A 被点名后的新一轮**留在原来的链上**：链代表的人、A 原来的层数都不变，轮数和花费照记在这条链上（第 10 节）。
      不然两个 Agent 靠互相点名来回传活，就绕过了层数和轮数的上限。
  - `wonton__hand_off_status`（查交出去的会话）：只读，不改任何东西，不算轮数，不叫醒 B。调用方要求同 `wonton__hand_off`。
    - 参数：`session`（可选，会话 ID）；不填时列出调用的这个 Bot 从同一个对话（房间、话题）交出去的全部会话。
    - 返回每个会话一项：`session`、`bot`（目标 Bot）、`chain`、`status`（`running`、`waiting_permission`、`ended`、`stopped`、
      `limit_reached`、`failed`）、`stop_reason`（ACP 的结束原因，没结束时为空）、`started_at`、`last_activity_at`、`summary`
      （B 最近一轮回复的末尾，至多 2000 字）。查别的 Bot 交出去的会话返回 `permission_denied`。
  - `wonton__send_message`（给某人发消息）：参数是收件人和正文，主人让 Agent 主动找家人、好友聊天用。收件人须是这个 Bot
    归属人的家人或好友（ADR-0045、ADR-0051）。每条都发确认卡片给 Bot 的归属人，卡片里是收件人和全文；主人可以按 ADR-0024
    对某个收件人放宽。投递由这个 Bot 的聊天前端以 Bot 的身份发（只有它有 Bot 的加密设备）：核心把批准的消息放进
    `GET /agent/outbox`（和 `/agent/permissions` 同一个地址、同一道门），前端长轮询取走后发进和收件人的私聊，回报发出的事件 ID。
- **被调的 Agent 以自己的身份说话**：主人原话的意思是把各 Agent 当成真人同事，B 在同一房间、同一话题里自己说话、
  发进度和结果，不由上一层转述。这一篇只管管道：核心把发起的对话随会话交给 B 的运行时；B 怎么进房间、在话题里怎么说、@ 的规则、
  和群里跳数上限（ADR-0045）的关系，见 [ADR-0083](0083-bot-contacts-and-group-chat.md)（#248）。
- **身份**：调用链代表最先说话的那个人（第 10 节），不是发起的 Bot；不许 Bot 以自己的名义调别的 Bot。
- **确认卡片**：
  - `wonton__hand_off` 交给的 Bot 不归这个人，或者是第一次从这个插件连到这个 Bot（ADR-0053 的“新使用方第一次连上”，加 Bot 这一维），
    卡片发给目标 Bot 的归属人；人把事交给自己名下的 Bot，确认过一次以后不再问。
  - 被调的 Agent 的权限请求照第 2 节变成卡片，发给它的归属人。标题写“发起处 → 哪个 Bot → 想做什么”：发起处是编排器的任务名，
    或交出去的那个 Bot，如“「修登录页」→ Codex → 想运行 git push”“Hermes → Codex → 想改 src/main.rs”。
  - 来源照 ADR-0016（已定第 6 条和“敏感操作的确认”第 2 步）：`requester` 是发起的插件或 Bot，带任务名；`origin` 是发起的对话，
    核心照常核对，对不上时落到目标 Bot 的主房间。编排器在 `session/new` 的 `_meta["app.wonton/task"]` 里给任务名和房间、话题，
    网关只留这一项，其余 `_meta` 照第 2 节丢掉。群里只显示“在等某某确认”。

### 10. 调用链：身份、层数、上限和记账

一次“人说了一句话”引出的所有 Agent 会话是一条**调用链**。

- **链 ID 由核心发**（128 位随机）：
  - 聊天里：桥每轮开始时调 `POST /agent/chains`（同 `/agent/permissions` 的门），报说话人、房间和话题，拿到链 ID；桥在 `session/new`
    给 Wonton MCP 的请求头里放会话 ID，核心按会话找到当前这一轮的链。Agent 改不了说话人，最多冒用自己 Bot 的另一个会话。
  - 经 `agent_acp` 连进来的（网页、编排器）：核心在建连接时发，放在 `X-Wonton-Chain` 里给网关，网关放进这条连接上所有会话的 MCP 请求头。
  - `wonton__hand_off`：新会话沿用调用方的链 ID。
  - 点名叫回（[ADR-0083](0083-bot-contacts-and-group-chat.md)（#248）第 4 节）：被点名的 Bot 的新一轮沿用点名的那个 Bot 发消息时所在会话的链，
    核心怎么把这条消息认到那条链上由实现定。不在任何链上的点名（比如人点的）开一条新链。
- **链代表谁**：核心在链上记最先说话的那个人，之后每一跳的 `X-Wonton-User` 都是他：
  - 聊天里是说话人（桥报，核心只认这个 Bot 听的人，ADR-0045）；
  - 网页里下的目标带 OIDC 访问令牌，同第 3 节；
  - 在编排器的 Bot 那里说的，编排器在 `X-Wonton-On-Behalf-Of` 写说话人，核心只在他是编排器的 Bot 听的人时采用。
  - 每一跳的目标 Bot 都必须听这个人（第 3 节），不然在建连接时就拒（403 `permission_denied`），不转给运行时。家人不能借别的 Bot
    或编排器用主人的私人 Bot；Bot 自己、联系人都不算“说话的人”。
- **层数**：人直接对话的编排器或第一个 Agent 是第 1 层，它派的是第 2 层，再往下派是第 3 层，默认最多 3 层（A→B→C），
  第 4 层核心拒绝：`wonton__hand_off` 和 `agent_acp` 的连接都返回 `limit_reached`，写明是层数。
- **不许绕圈**：链上已经有的 Bot（含发起的那个）不能再被调（A→B→A），一律拒绝，不算放宽项。B 点名叫回 A 不是调用，
  不开新的一层：A 在原来那一层接着做（第 9 节）。
- **每条链的轮数和花费**：
  - 轮数是链上所有会话的 `session/prompt` 加起来，默认 50，被点名叫回的那些轮也算。核心数：`agent_acp` 的帧经核心转发，桥每轮开始时也来领链 ID。
  - 花费是链上经模型网关的用量，含被点名叫回的那些轮，默认 200 万 token；模型网关有价格时同时显示金额。用官方登录的 Agent 不经网关，花费记“不经网关”，
    只受轮数上限管。
  - 到了上限，核心停下这条链（新的轮次、新的 `agent_acp` 请求、链上的模型请求都返回 `limit_reached`），给链代表的人和链上各 Bot 的
    归属人发卡片“这条链用到上限了”，按钮“再给一份”和“先停着”。
- **随时能停整条链**：链代表的人和链上各 Bot 的归属人，不等到上限，随时能停下整条链（App 里这条链的那一行和链上的卡片上）。
  - 停时核心对链上每个经 `agent_acp` 的会话发 `session/cancel`；发起的那一轮在桥里，核心通知桥取消这一轮，走哪条通道由实现定。
  - 网关和桥把各自未决的权限请求都回 `cancelled`，这是 [ACP 规范](https://agentclientprotocol.com/protocol/v1/prompt-turn#cancellation)
    对发 `session/cancel` 一方的要求；核心撤回对应的卡片。
  - 停下的链不再接新的轮次、`agent_acp` 请求和模型请求，返回 `stopped`；要接着做，人重新说一句，开一条新链。
- **按链记账，核心记**：核心既在 `agent_acp` 的路上，也在模型网关的入口（`models.wonton.internal`）。运行时一个连接一个 Agent 进程
  （第 2 节），网关起进程时把模型网关的地址设成带链 ID 的路径（`/chains/<链 ID>/…`），核心把这个进程的模型请求记到这条链上，
  转发时去掉这段。桥自己的会话不带链路径，照旧记在 Bot 头上；所以链的花费是交出去的那些会话的，发起的那一轮记在发起的 Bot 上。
  例外是被点名叫回的那一轮：它在桥里，但要记到原来的链上，怎么把这一轮的模型请求认到链上由实现定。
  App 按“哪条链（哪个人、哪个对话）、哪个 Bot”分开显示，缺的显示“未知”。
- **并发**：每个使用方同时开着的 `agent_acp` 连接默认最多 4 条，核心数，满了返回 `busy`；运行时自己的连接数上限（第 2 节）照样有效。
- **上限在哪改**：层数、每条链的轮数和花费在设置的“Agent”一页；每个使用方的并发在它的详情页。只有主人能改，Bot 不能改。
  核心转发时把上限放在请求头里给网关，使用方改不了。默认值按实测再调。

### 11. 网关和桥用官方的 Rust ACP 库

- 网关用官方的 [`agent-client-protocol`](https://github.com/agentclientprotocol/rust-sdk)（5.2.x，Apache-2.0）。网关要在两头之间转发、
  改写 `session/new`、自己回答权限请求、逐跳传递取消，这正是这个库的 proxy 和 HTTP/WebSocket 传输做的事，手写最容易出错。
- 桥的 ACP 层跟网关一起换，不单独重写桥：单独换只省两百来行收发，却要先引入整个库。
- 这个库的风险（2026-11 查，5.2.0）：约 4.5 个月出了 4 个大版本，主要一个人维护；`SessionUpdate` 没有兜底变体，认不出的更新会被丢掉；
  没有帧大小上限；HTTP 入口拿不到请求头。所以守四条：
  1. 钉精确版本（`=5.2.x`），照上游的迁移指南升级。
  2. 网关转发用无类型的处理，认不出的 `SessionUpdate` 落到 `Other` 照样转发，不丢。
  3. 按行读消息由我们自己写，带长度上限。
  4. WebSocket 入口我们自己写：注入 `X-Wonton-*` 头（第 3 节），把 axum 的默认上限（单条消息 64 MiB、单帧 16 MiB）改小。
- 它的 schema 给整个 workspace 的 serde_json 打开 `preserve_order`：同一次构建里所有 crate 的 JSON 对象都按插入顺序。
  `serde_json::Value` 因此变大，wonton-core 里两处返回 `CallToolResult` 的地方会触发 `clippy::result_large_err`（#255 发现），换库前先把它们装箱或改小。

## 考虑过的做法

- **自定一套“Agent 接口”**（REST 加事件流）：ACP 已经是开放标准，主流 Agent 都有适配；自定接口要给每家 Agent 再写一遍适配。
- **前端直连运行时、不经核心**：插件之间不再隔离，核心认不出是谁在连，也做不了确认卡片和审计。
- **权限请求交给前端回答**：任何前端都能自己点“允许”，主人的确认形同虚设。
- **网关用 Python 写在 Agent 底座里**：底座的转发器只是字节管道；网关要记会话归属、改字段、回权限、排队，等于把 Rust 桥重写一遍。
- **按 `X-Wonton-User` 让任何家人用任何 Bot**：私人 Bot 的工具调用按归属人开放的权限执行，家人 B 用主人 A 的私人 Bot 等于借了 A 的权限；
  共享 Bot 本来就是开放给家人的那一份权限，所以只放行 Bot 听的人。
- **一个 Agent 进程给所有前端共用**：省内存，但会话、取消、排队要在多个客户端之间协调，ACP 没有多客户端的约定。
- **提供方不许使用同一能力，派活只有一层**（这次修订的上一稿）：编排器只能指挥、不能被用户直接找、不能被调，Agent 不能把事交给别的 Agent，
  主人要的“某 Agent—其他 Agent”走不通。改成放开规则、按调用链拦层数和绕圈。
- **按插件间的依赖图防绕圈**：P、Q 互相 `requires` 不等于会绕圈，真绕圈发生在 Bot 之间、在调用时；按调用链拦更准，核心也不用解依赖图。
- **调用链一律算主人**：家人说的话会以主人的身份使用主人的私人 Bot，等于借了主人的权限。
- **调用算发起的 Bot 自己**（当作 Bot 联系人）：等于 Bot 指挥 Bot，第 3 节已经不让联系人走这个入口。
- **B 结束时自动叫醒 A**：省一句话，但会越聊越多。Cindy 旧版做完自动跟进，伙伴被叫醒的次数是用户发言的两倍多，一个委派续跑了近百轮，
  后来删掉了（[teammate-capability-ledger.md](https://github.com/makecindy/cindy/blob/4312f38/docs/product-rules/teammate-capability-ledger.md#L10-L25)）。
- **ACP 继续手写**：违反“不造轮子”；网关要做的转发、改写、回权限、传取消，官方库已经有（第 11 节）。
- **B 的结果由 A 转述**：主人要把 Agent 当真人同事，B 在同一个对话里自己说话；转述还会丢掉 B 的进度和问题。
- **花费按 Bot 记、不分链**：几个链同时用同一个 Bot 时分不出是谁花的，主人看不懂账，链也停不下来。
- **立刻把桥抽成独立插件**：同时改了 Bot 账号归属和多 Bot 模型，风险集中；先拆网关、后挪桥。

## 代价与待验证项

- 一个连接一个进程，付的是整个进程（node、适配器、CLI）的基础开销，不只是每个会话的增量；连接数上限按实测定。
- 同一个 Agent CLI 的几个进程共用一份 HOME，有的 CLI 把会话、配置写在同一个文件里，并发写可能互相覆盖，要逐家实测。
- 外部前端多走一跳核心转发，延迟和断线重连要实测。
- 把桥拆成网关和前端是一次不小的重构，先在一个 Agent 插件上做，再推到全部。
- 聊天里的说话人由桥或编排器自报（`POST /agent/chains`、`X-Wonton-On-Behalf-Of`），核心只能核对这个人在这个 Bot 听的人里，
  核对不了这句话真是他说的。要更强的保证，得让核心看得到加密房间里的消息，代价不值；人跨人交出去时第 9 节的卡片兜底。
- 带链 ID 的模型网关路径要每家 Agent 的 CLI 都认自定义的网关地址；不认的，那部分花费只能按 Bot 记，链上显示“未知”。
- `wonton__send_message` 只管住经桥发的消息；自带 Matrix 的 Agent（Hermes、OpenClaw）本来就能给同房间的人发消息，核心拦不住。
- 3 层、50 轮、200 万 token、并发 4 是我们自己定的，按真实任务再调。查实过的参照：ACP 的 [Subagents RFD](https://agentclientprotocol.com/rfds/subagents)
  不限层数，只禁成环；Cindy 的工人软上限 5、硬上限 8（[orcaTeam.ts](https://github.com/makecindy/cindy/blob/4312f38/packages/maker-shared/src/orcaTeam.ts#L229-L233)），
  群聊里前 5 位自动接着答、第 6 位起等人点“继续”（[bot-group-chat.md](https://github.com/makecindy/cindy/blob/4312f38/docs/product-rules/bot-group-chat.md#L403)）；
  Alma 的子代理到 100 步停（[v0.0.729](https://github.com/yetone/alma-releases/releases/tag/v0.0.729)）。
- 编排器的 ACP 客户端要能指到 `agent_acp.wonton.internal`。AO 按各家找官方程序起 ACP，能不能配置自定义命令没查实，可能要一个
  在容器里冒充某家 ACP 程序的小转发器。
- 被派活的 Agent 各用自己运行时的工作区，彼此不共享目录，交付靠 git 分支，编排器不能假设工人在同一个目录里。
