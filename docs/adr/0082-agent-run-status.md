# ADR-0082 Agent 进度用一条状态消息：按 MSC4564，每轮一条，编辑推进到“等你确认”“完成”“失败”

- 状态：已接受（2026-11-11）。方向（按 MSC4564 发一条状态消息、答案仍一次发完、App 也显示 Hermes 发的同类状态）由主人定
- 修订 [ADR-0035](0035-acp-bridge.md)：一轮中间不再只有 typing，另有这条状态消息

## 背景

ACP 桥一轮里只发 typing，答案等整轮结束拼成一条消息发出（ADR-0035）。Agent 干几分钟活时，聊天里看不出它做到哪、是不是在等人。
各家 Bot 的做法五花八门：在请求上加表情、编辑一条列出工具步骤的消息、自己给提示消息加反应当选项。客户端要么当噪音显示，要么逐家适配。

[MSC4564](https://github.com/matrix-org/matrix-spec-proposals/pull/4564)（2026-11-11 读取，提案最后更新 2026-11-07，开着，`needs-implementation`）
把这件事定成格式：

- Agent 在 `m.room.message` 的内容里（编辑时在 `m.new_content` 里）放一个对象，稳定前的键是 `org.fluxchat.agent`，稳定后是 `m.agent`，
  客户端两个都读。字段全部可选：`v`（结构版本，现在是 `1`）、`run`（这一轮针对的那条请求的事件 ID）、`state`、`step`（当前步骤的一句话）、
  `steps`（到现在几步）、`progress`（0～1）、`result`（结束时的一句话总结），以及 `model`、`context`、`cwd`、`cost_usd`、`choices`。
- `state` 只有四个值：`working`、`waiting`（等用户）、`done`、`failed`。
- 一轮一条消息：开始时发（`working`），之后用 `m.replace` 编辑推进，以 `done` 或 `failed` 结束；客户端看最新的编辑。`body` 始终是能读的文字。
- `choices` 加 `state: waiting` 是一个待回答的提问，用户用 `m.annotation` 反应作答，键是选项的 `key`。
- 字段是发送者的自述：客户端只对用户认定是 Agent 的发送者解读，从不自动作答。

已有实现：FluxChat 客户端和它的发送 SDK；Hermes 的实现 PR 还开着（NousResearch/hermes-agent#138698），合并后 Wonton 上的 Hermes 会直接发这个格式。

## 决策

### 1. 格式照 MSC4564，不另起一套

- 桥发 `org.fluxchat.agent`；App 读 `org.fluxchat.agent` 和 `m.agent`，两个都有时以 `m.agent` 为准。提案稳定后桥直接改发 `m.agent`，不留旧键。
- 状态消息是 `m.notice`（Matrix 约定 Bot 不回应 notice，别的 Bot 不会把它当成要回的话），`body` 是桥按房间语言写的一句话，
  如“正在执行命令（第 3 步）”“等你确认”“完成，用了 5 步”“失败：模型网关出错”。
- 桥发的字段：`v`、`run`、`state`、`step`、`steps`，有计划时加 `progress`，结束时加 `result`。不发：
  - `model`、`context`、`cost_usd`：桥拿不到可靠的值（官方登录的 Agent 不经模型网关），按“失败即停”不编；
  - `cwd`：容器里的路径，对主人没有意义；
  - `choices`：桥的权限请求是核心的确认卡片（ADR-0016），权威在核心，不在房间里另开一个能用反应回答的提问。
- MSC4564 没有“打断了”和“结果不确定”。这两种结束仍落在四个值里（见第 3 节），另在状态对象旁边放 `app.wonton.end`
  （`cancelled` 或 `uncertain`）给 App 区分；别的客户端不认识这个键，看到的是 `failed` 和 `body` 里写明的原因。

### 2. 每轮一条，放在请求所在的地方

- 桥开始一轮（发出 `session/prompt`）时发一条状态消息，`run` 是这一轮的请求消息。一轮在跑时攒下的几条合成一轮发出时才发，
  `run` 是这批里最后一条，不为攒着单发一条。steering（ADR-0035 的 `_session/steering`）插进去的话算进同一轮。
- 放在请求所在的地方：主对话里的请求，状态消息发在主对话，不挂话题关系（MSC 示例里的 `m.thread` 只是示例，挂上就会给每个请求开一个话题，
  和 ADR-0058 的话题冲突）；话题里的请求，状态消息发在同一个话题。
- 答案仍是一轮结束时一次发完的一条消息（ADR-0035 不变），先发答案，再把状态编辑成 `done`，所以时间线里状态在上、答案在下。
- 编辑合并：状态值变了（进入或离开 `waiting`、结束）立刻编辑；只有步骤变了的，最多每 3 秒一次。一轮最多 60 次编辑，到了以后只等最终那次。
  编辑不算 ADR-0045 的跳数（#240 的做法），频率限制照算。
- typing 照发：ADR-0044 的 Bot 状态和别的 Matrix 客户端都靠它。

### 3. 桥怎么从 ACP 推出状态

| ACP 里发生的事 | `state` | `step`、其他 |
| --- | --- | --- |
| 发出 `session/prompt` | `working` | “在想” |
| `session/update` 的 `tool_call`（新工具调用） | `working` | 按工具类别写：读文件、改文件、删除、移动、搜索、执行命令、上网、在想、其他；`steps` 加一 |
| `session/update` 的 `agent_thought_chunk`、`agent_message_chunk` | `working` | “在想”“在写回复” |
| `session/update` 的 `plan` | `working` | `progress` = 已完成条目 / 全部条目 |
| `session/request_permission`（变成核心卡片） | `waiting` | “等你确认”；主人回答后回到 `working` |
| `session/prompt` 返回，`stopReason` 是 `end_turn` | `done` | 先发答案 |
| `stopReason` 是 `cancelled`（主人打断、取消） | `failed` | `app.wonton.end: cancelled`，`result` “打断了” |
| `stopReason` 是 `max_tokens`、`max_turn_requests`、`refusal` | `failed` | `result` 写明是哪一种 |
| 桥到 Agent 的连接断了、超时、Agent 报错 | `failed` | `result` 按 ADR-0035 的错误类别写 |
| 工具已开始执行时连接断了（ADR-0035 的“不确定”） | `failed` | `app.wonton.end: uncertain`，`result` “结果不确定” |

- `step` 只按工具**类别**写，不写工具调用的标题：标题是 Agent 给的，可能带命令参数、路径或密钥，ADR-0035 也不把执行轨迹放进聊天。
- 经 MCP 总线的敏感工具在等核心卡片时，桥看不到（它只看到一个进行中的工具调用），状态停在 `working`；主人从卡片本身和 ADR-0044 的
  头像（`waiting`）看得到。
- 桥重启：桥把每轮状态消息的事件 ID 存在数据目录里。重启后，还停在 `working`、`waiting` 的都编辑成 `failed` 加 `uncertain`，
  `result` “桥重启了，这一轮结果不确定”。不留一条永远在“干活中”的消息。
- 编排器的任务话题（ADR-0059）里的每一轮同样处理；工作板（ADR-0058）不变，状态消息不替代它。

### 4. App 怎么显示

- **只信 Bot**：客户端核心只对核心认得的 Bot 账号（ADR-0044、ADR-0045 的 Bot 列表）发的消息解读状态对象；别人发的同样内容按普通消息显示。
  不为某个 Agent 开特例（ADR-0015）：桥发的、Hermes 发的、以后别的 Agent 发的，只要格式对，显示都一样。
- 客户端核心把它变成时间线里的一种条目（`TimelineContent::AgentRun`）：请求的事件 ID、状态、步骤、步数、进度、结果和结束原因。
  状态是枚举，`working`、`waiting`、`done`、`failed`，不认识的值落到 `Other`；`v` 不是 `1` 时整条当普通消息显示。
- 壳按类别写字（ADR-0043），一行显示：
  - `working`：转圈加步骤，有进度时显示进度；
  - `waiting`：醒目的“等你确认”，这个 Bot 在这个对话里有等回答的核心卡片时，点了打开卡片；
  - `done`：收成一行小字“完成 · 5 步”；
  - `failed`：一行错误，带 `result`；`cancelled` 显示“打断了”，`uncertain` 显示“结果不确定”，不画成失败的红色；
  - `Other`：显示 `body`，同普通的 Bot 提示消息。
- **和 Bot 的在线状态核对**：ADR-0044 的 `BotActivity` 是 `offline` 或不确定时，还停在 `working`、`waiting` 的状态行显示成“不确定”，
  不转圈。只在客户端核心里合成，壳不自己判断。
- Hermes 这类 Agent 发的 `choices`：`waiting` 时画成按钮，主人点了才以主人的身份发 `m.annotation` 反应，不自动作答。
  按钮和核心卡片样式不同，写明是这个 Bot 在问；它答的是 Agent 自己的问题，不经核心，也不是 Wonton 的确认。
- 推送不加：编辑默认不推送（Matrix 的推送规则压掉 `m.replace`）；要主人动手的已有核心卡片的推送，答案消息本身照常推送。
- 聊天列表的最后一条预览跳过状态消息，显示答案或上一条真正的消息。

### 5. 不认识这个格式的客户端

- 旧版 Wonton App 和 Element：不认识 `org.fluxchat.agent`，看到的是一条 Bot 的提示消息，编辑后显示最新的 `body`
  （“正在执行命令（第 3 步）”→“完成，用了 5 步”），每轮多一行字，不会显示错。不做“不认识就不显示”：藏掉以后旧 App 里看不出 Bot 在干活。
- 新 App 遇到以后 MSC 加的状态值：落到 `Other`，显示 `body`。

### 6. 和 ADR-0044 的 Bot 状态是两回事

| | ADR-0044 Bot 状态 | 本篇的状态消息 |
| --- | --- | --- |
| 回答什么 | 这个 Bot 现在在不在线、忙不忙 | 这一轮任务做到哪了 |
| 粒度 | 一个 Bot 一个，不分房间 | 一轮一条，在请求所在的房间和话题 |
| 来源 | 核心看得到的事实加 typing，不信 Agent 自述 | Agent（桥）的自述 |
| 显示在 | 头像 | 时间线里的一行 |
| 谁都看得到 | 只有 Wonton App | 任何 Matrix 客户端（`body`） |

- 头像不读状态消息：ADR-0044 不信 Agent 的自述，这条不变。
- 状态消息不替代核心卡片：`waiting` 只是提示，能回答的永远是核心的卡片。
- 两者对不上时以头像的 `offline` 为准（第 4 节）。

## 考虑过的做法

- **保持只有 typing**：看不出做到哪、在不在等人，长任务像卡住了。
- **正文也流式编辑**（OpenClaw、Hermes 的做法）：每轮编辑很多次，加密房间里每次都是一个完整的新事件；等流式消息的 MSC4471 定稿再说。
- **在请求上加反应表情**（MaplePrivacyLabs 的桥）：分不清是人加的还是 Bot 加的，带不了步骤和进度。
- **自定事件类型或状态事件**：别的客户端完全看不到，也没有 `body` 兜底；状态事件还要额外权限、进房间状态。
- **放进 ADR-0044 的长轮询**：核心看不到一轮做到哪，Element 也看不到；而且 Hermes 发的状态走 Matrix，不经核心。
- **自己的前缀（`app.wonton.agent`）**：Hermes 会发 `org.fluxchat.agent`，两套格式要都认；跟着 MSC 走只有一套。
- **桥也用 `choices` 做确认**：任何能在房间里加反应的人都能“回答”，Agent 自己也能伪造；确认只在核心（ADR-0016）。

## 代价与待验证项

- MSC4564 还没稳定，字段和前缀可能变；跟着改（有第一个外部用户之前不留兼容）。
- 每轮多一条消息和若干次编辑，加密房间里每次编辑都是一个事件；3 秒合并和 60 次上限按实测调。
- `app.wonton.end` 是我们加的键，别的客户端只看到 `failed`。
- Hermes 的实现还没合并，App 对它的兼容只按提案和 PR 读过，没有实测；合并后用真实 Hermes 在开发栈上过一遍。
- 自带 Matrix、又不发这个格式的 Agent（OpenClaw）照旧只有 typing。

## 关联

ADR-0015（标准接口）、ADR-0016（确认卡片）、ADR-0035（ACP 桥）、ADR-0043（多语言）、ADR-0044（Bot 状态）、ADR-0045（Bot 列表、跳数）、
ADR-0058（话题和工作板）、ADR-0059（编排器的任务话题）。
