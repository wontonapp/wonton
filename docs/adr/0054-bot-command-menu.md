# ADR-0054 Bot 的斜杠命令菜单：每个 Bot 一个房间状态事件，整表替换

- 状态：已接受（2026-11-07）

## 背景

主人在聊天里输入 “/” 时，想像 Telegram 那样弹出 Bot 能用的命令和说明。各家 Agent 经 ACP 的 `available_commands_update`
报命令：每条只有 `name`、`description` 和可选的 `input.hint`（一句参数提示），没有结构化参数；调用就是把
“/名字 参数” 当普通文字发过去。每次通知都是整张表；omp、Claude Code、Qwen Code 装了新 skill 会在会话中途重发。
2026-11-07 的实测：omp 57 条，Claude Code 50 条，Qwen Code 54 条，Gemini CLI 20 条，codex-acp 6 条，OpenCode 5 条；
dsh 的 ACP 不报命令。命令名不只有字母：有 `skill:pdf`、`mcp:xxx`，还有带空格的 `review branch`。

Telegram 的做法是 Bot 调 `setMyCommands` 把命令表存在服务器上，客户端输入 “/” 时弹出，选中即发送；群里用
`/cmd@bot` 区分 Bot。Matrix 没有成文的标准。MSC4332（In-room bot commands）已被 MSC4391 取代：每条命令一个状态事件
`m.bot.command_description`，参数带类型。MSC4391 只有 gomuks 和 mautrix-go 实现，Element 的两端都没有。
MSC4133（资料里的自定义字段）按用户而不是按房间，字段变了不会推送给客户端，还要和头像、昵称共用 64 KiB 的资料上限。

## 决策

- 每个 Bot 在它所在的每个房间写一个状态事件 `app.wonton.bot.commands`，`state_key` 是 Bot 自己的 Matrix ID，内容是
  整张命令表（`wonton_protocol::BotCommands`）。每次整表替换，菜单不会停在更新到一半的状态。
- 每条命令写名字、说明和可选的参数提示，和 ACP 一一对应。名字原样保留冒号、点和中间的空格。条数和大小有上限，
  截掉的条数写进 `omitted`，客户端如实提示还有几条没列出。
- 调用命令就是一条普通文字消息 “/名字 参数”，不另设事件类型。群里有多个 Bot 时用 Matrix 的提及（`m.mentions`）
  指定 Bot，不学 Telegram 的 `/cmd@bot`。
- 客户端只认 `state_key` 和发送者相同、且发送者是房间成员的事件。
- 写事件的是 Bot 自己，核心不经手：ACP Agent 由 wonton-acp-bridge 写，桥自己的 `/wonton` 命令排在最前面；不是 ACP 的
  Bot（如 Hermes）写同一个事件，也能有菜单。没有权限写状态事件时，桥在房间里提示一次，不静默。

## 理由

- 整表替换是 ACP 的语义。MSC4391 一条命令一个事件：57 条要发 57 个，更新要逐条比对，删除要发空内容，中途客户端
  看到的是残缺的表。它的参数类型我们从 ACP 拿不到，填不出来。主流客户端也没有实现，用它换不来别家 App 的支持。
- 菜单放在 Matrix 里：核心不为 Agent 做特殊处理（ADR-0015），任何会说 Matrix 的 Bot 都能用。

## 代价与待验证项

- 这是私有事件类型，别家客户端看不到菜单。MSC4391 合进规范、Element 实现之后，改用它。
- 群里 Bot 不是管理员时可能写不了状态事件，那个房间就没有这个 Bot 的菜单。
