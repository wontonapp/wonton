# ADR-0119 待处理面板：要用户决定、要用户做、要用户知道的事，统一进核心里的一页

- 状态：提议（2026-11-25 按 Linear、GitHub 通知、Slack、Things、Todoist、滴答清单、Apple 提醒事项、Superhuman、Alma 的公开文档重做调研后改写。上一版只分“要选择”和“要知道”，5 条建议主人 11-25 没直接批，原话：“这个地方本质上是类似于 Linear 还有 todo 那种，去调研看一下怎么设计比较好。”）
- 取代：[ADR-0016](0016-mcp-bus-and-confirmations.md) 第 2 步里的“等你确认”列表（`GET /_wonton/api/confirmations` 的列表、聊天列表里的入口和它的角标）。卡片本身、来源会话、回答接口都不变，只是列表和入口换成面板
- 修订：[ADR-0047](0047-plugin-notices.md)（插件通知不再发进 Wonton 私聊）、[ADR-0056](0056-internal-mailbox.md)（应用来信同上）、[ADR-0045](0045-family-members.md)（文件链接、家人加入的通知同上）、[ADR-0050](0050-cross-server-chat.md)（配对请求同上）、[ADR-0101](0101-notifications.md) 第 3 节（应用角标的数里加上待处理条目，算法按下文第 7 节改）
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（推送链路）、[ADR-0118](0118-im-three-layers-push-with-body.md)（推送带正文、房间不再端到端加密；分支待合）、[ADR-0038](0038-multi-step-cards.md)（问题卡片，M1 不做）、[ADR-0117](0117-bot-task-live-progress.md)（Bot 长任务进度）、[ADR-0082](0082-agent-run-status.md)（运行状态）、[ADR-0088](0088-family-accounts.md)（管理员卡片）、[ADR-0102](0102-app-lock.md)（App 锁）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)（外观）、[ADR-0005](0005-native-clients-thick-core.md)（厚核心，壳只渲染）
- 实施拆分：协议 → 核心 → 两端壳 → 拆私聊 → 派活事件（router 的 worker 桥），每步的范围和验证见任务结果文件 `~/wonton-dev/tasks/inbox-design.result.md`。

## 用户看到什么

打开云吞，聊天列表最上面固定一行“待处理”，右边的数字是有几件事要你动。点进去分三段：最上面“要你决定”（装个应用、以你的名义发条消息、家人想加入），卡片上直接有按钮，当场就能点；中间“要你做”（你自己记的待办、交给机器人的活，带截止时间），能自己勾完、改期，或者交给一个机器人；下面“新通知”（下载完了、备份失败、一封找回密码的信），看一眼点“知道了”，带地址的另有“打开”。做完的进最下面“已处理”，写着谁在什么时候处理的，24 小时内能撤销，之后归档留 30 天。没有事情时这一行变灰，页面里写着“都处理完了”。原来那个“云吞”私聊不再塞通知，旧消息还能从页面最下面翻回去。

三段用同一种卡片骨架，只是按钮不同：

| 段 | 用户要做什么 | 卡上有什么 |
| --- | --- | --- |
| 要你决定 | 在一组按钮里选一个 | 标题、说明、来源、到期时间、按钮；确认卡片本身就是一种决定 |
| 要你做 | 自己完成，或交给 Bot | 标题、可选截止时间、完成框、“交给 Bot”；交给 Bot 后显示进度和“停止” |
| 要你知道 | 看一眼，点“知道了”（已读） | 标题、正文、可选附件/链接、级别；有地址时给“打开” |

## 背景

主人 2026-11-25 的原话要点：助理类产品都该有一个类似“主管理后台”的地方，让用户清楚自己要处理哪些东西；有的要用户做选择，有的只要用户知道（点“已读 / 知道了”），本质上都是推给用户的通知，只是类型不同。这类往往重要、全局，不该只在对话流里出现。上一版只做了“要选择 / 要知道”两类，回答不了“todo 那种”：用户自己的待办、交给机器人的活、做完的活没有地方放。

现在的两套东西都不完整：

- **通知往对话里塞。** ADR-0016 曾把确认卡片发进“Wonton”私聊；卡片搬到会话里以后，私聊继续接插件通知（`crates/wonton-core/src/gateway/notify.rs:93`）、应用来信（`crates/wonton-core/src/mailbox/delivery.rs:289-320`，每人一个私聊）、文件链接、配对请求（`crates/wonton-core/src/peers/mod.rs:645-655`）、入口对账和联邦名单的失败（`crates/wonton-core/src/entry_runtime.rs:200`、`crates/wonton-core/src/matrix/federation_config/mod.rs:124-131`）、频率超限提示（`gateway/notify.rs:491`）。私聊在聊天列表里就是一间普通房间：清了以后找不回来，没读的混在消息里，长通知（一封信）挤占聊天流。
- **“等你确认”列表只装卡片。** 它是 ADR-0016 第 6 条加的，列等回答和结束 24 小时内的卡片，另有 Bot 联系请求（`crates/wonton-protocol/src/confirmation.rs:234-250`）。核心里有表、有版本号、长轮询、推送、按人过滤（`crates/wonton-core/src/mcp/confirm.rs:480`、`crates/wonton-core/src/gateway/confirmations.rs:34`）；两端有页面（`apps/ios/Wonton/ConfirmationsListView.swift:4`、`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/ConfirmationsScreen.kt:78`）、聊天列表顶部的入口和角标（`apps/ios/Wonton/ChatListView.swift:98`、`ChatScreen.kt:238`）和全局提示条（`apps/ios/Wonton/MainView.swift:206`）。这些都能接着用，缺的是把它扩成“所有推给用户的事”，并把“要做的活”从聊天里摘出来。

已经有的地基，不用新造：

- 条目在核心里、由核心给 ViewModel、壳只渲染（AGENTS.md“厚核心薄壳”）。
- 按收件人过滤：卡片发给 Bot 的归属人，或所有管理员，谁先答算谁的（ADR-0088）；列表按请求者只列自己的（`crates/wonton-core/src/mcp/confirm.rs:480-486`）。
- 结束的卡片留 24 小时（`mcp/confirm.rs:38`）；拒绝、过期、已经被谁答了（`answered_by`）都已经有状态。
- 推送按类别、没有表单的卡片能在通知上直接答（ADR-0101 第 1 节）；通知的分类开关和 App 锁（ADR-0101 第 5 节、ADR-0102）。
- 聊天列表的入口、角标、零计数时的次要色、空态、提示条不重复显示的规则（ADR-0016 第 2 步、第 6 条）。
- Bot 的回合状态和 `wonton__report_progress`（ADR-0082、ADR-0117）：进度是 Bot 的自述，核心只聚合，不核对。

## 调研结论

调研页在 `~/wonton-dev/tasks/inbox-design.html`，九家逐项对比。对决策有用的五条：

1. **没人把通知做成待办。** Linear Inbox、GitHub Notifications、Slack Activity 都只负责“清”；真要做事去 Triage、Issue、To-do 或任务页。通知和待办混在一起时，用户会把通知当工作，把工作漏在通知里。
2. **“稍后”必须给固定出口。** Alma 的 Snooze 只有 1 小时 / 今天晚些 / 明天 / 下周；Superhuman 用 H 设 Reminder；Things 用 When。预设降低决策成本，按时回来；同一线程连按两次 Snooze 就该转待办。
3. **Done 都有去处。** Things 进 Logbook、GitHub 有 Done 3 个月、Linear 删除可撤销、Alma 留报表。打开列表不清零，逐条处理才减。
4. **角标只算“要你动”的。** Apple HIG：角标只表示未读；Alma 三处（托盘、Home、通知）共用一个 Needs you 数字。没人把“全部消息 + 全部任务”堆进一个数。
5. **任务层和通知层分开。** 任务有 assignee、due、status（Alma 的 What/Why/Done when/Assignee/Planned；Things 的项目/日期）；通知只是事件。只有 Alma 把 Needs you 和任务放在一个托盘，但仍是 Needs you / Active / Backlog 三段，任务页独立。

我们抄五条：三 lane、卡上动词、固定 Snooze、Done 可回看、通知不进行动列表。不抄：Linear 的 2000 条上限和 Triage Rules、GitHub 的 15 个筛选、Slack 的三套状态、Things 的 Someday/Anytime、Alma 的 Work 看板和 Responsibility、Apple 的地点提醒/附件/标签、Superhuman 的 Auto Archive。

### 上一版 5 条建议的处置

| 原建议（11-25） | 处置 | 理由 |
| --- | --- | --- |
| 1. 私聊退休（通知不再进“云吞”对话，房间隐藏） | **保留**，收紧为：只读归档，面板底部“旧的通知记录”可回看，M1 不删 | 旧信里的找回密码链接不能断；不用房间塞通知的目标不变 |
| 2. 入口只在聊天列表顶部一行 + 侧边栏 | **保留**，明确不放回底栏 | ADR-0095 已去掉底栏；主管理后台要在第一屏 |
| 3. Bot 提问留聊天，推荐不进面板 | **保留 M1**，等 ADR-0038 问题卡片落地后再进 `decide` | 回答要用上文，搬进面板会把上下文劈成两半 |
| 4. 应用角标 = 未读消息 + 待处理条目 | **改写**：角标 = 未读消息 + 待决定 + 今天到期/逾期的待办 | `know` 只是通知，进角标会把数字做大、把“要你动”稀释掉 |
| 5. 第一版不做 Snooze、搜索、筛选、“全部已读” | **部分改写**：Snooze 做固定四档；搜索、筛选、“全部已读”仍不做 | Snooze 决定“放哪”和角标怎么减，是面板的核心动词；搜索等量上来再加 |

## 决策

### 1. 三条 lane，一张表

**条目在核心里**（`core/inbox`，权限 0600），由核心给 `InboxViewModel`，两端壳只渲染、把操作发回核心。

一条条目有：ID、lane、类型、状态、收件人、时间、到期时间、来源（Bot、插件、核心、人）、可选的来源会话（`origin`）、指派对象（人 / Bot）、可选截止时间、进度和结果，以及按类型的载荷。

**三条 lane**：

| lane | 用户要做什么 | 典型类型 |
| --- | --- | --- |
| `decide`（要你决定） | 在一组按钮里选一个 | 确认卡片、Bot 提问、联系人请求、配对请求、worker 等批准 |
| `do`（要你做） | 完成、改期、交给 Bot | 自己记的待办、交给机器人的活、Bot 建议的任务、提醒某条消息、等对方回复 |
| `know`（要你知道） | 看一眼，已读 | 插件通知、应用来信、文件送达、告警、升级结果、worker 做完 |

`kind` 是带 `Other` 的枚举（`#[serde(other)]`，AGENTS.md 的兼容规则）：`Confirmation`、`Question`、`Request`、`Notice`、`Mail`、`Alert`、`Result`、`File`、`WorkerNeeds`、`WorkerDone`、`WorkerStuck`、`Task`、`Suggestion`、`Security`、`Other`。壳按 `lane` 和 `kind` 选行和卡片样式；`kind` 不决定能不能操作，操作只看 lane 和载荷里的按钮。

`do` 的**状态**只有：`todo`（还没开始）、`active`（Bot 在做）、`waiting`（等对方）、`stuck`（卡住要你处理）、`done`；另有 `dismissed`（用户删了）。`decide` 的状态是 `pending` → `handled` / `expired` / `snoozed`；`know` 是 `unread` → `read`。`handled` 是唯一的成功终态，带处理人和时间；`expired`、`dismissed` 是另外两个终点；`handled_elsewhere` 只用于发给所有管理员的条目被别人答了。

**ID 由核心分配**，带来源前缀（`c-` 卡片、`q-` 提问、`t-` 待办、`n-` 通知、`w-` worker），和 Matrix 事件 ID 无关。`revision` 是全局版本：任何一条新增、改状态、某人读了都加一。

**谁能看到**：收件人在创建时定下，之后不变，只有两种——一个人（Bot 的归属人、家人自己）或所有管理员。列表按请求者过滤，家人看不到主人的、别人的，主人也不替家人看。管理员条目对每个管理员都出现，回答后其余人的那份变成 `handled_elsewhere`。入口数字只数自己的。

**保留**：`decide` 按来源的期限（确认卡片 10 分钟；配对请求不过期；worker 等批准跟运行器的等待超时）；`do` 按截止时间，逾期仍留；`know` 未读 30 天。`decide` / `do` 处理过的留 24 小时可撤销；`know` 读了也留 24 小时再归档；之后都归档 30 天，审计另存 90 天。状态先落盘再改版本号；落盘失败时如实报错，不假装已处理。

**核心不做**：项目、看板、标签、里程碑、依赖、重复规则、多人分派。需要时另开 ADR。

### 2. 来源：现有的每一处都迁进来

| 来源 | `kind` | lane | 现在的去向 | 谁看得见 |
| --- | --- | --- | --- | --- |
| 插件工具、装/卸/升级、存文件、受控发送、链上限 | `Confirmation` | `decide` | 会话里的卡片行 + 列表 | 归属人；要管理员批的是所有管理员 |
| Bot 在对话里问的问题 | `Question` | `decide`（M1 留聊天） | 聊天里的文字 | 归属人（问在哪个房间就带哪个 `origin`） |
| 安装、升级、发送、存文件的结果 | `Result` | `know` | 卡片原地变成结果 | 发起的人 |
| 插件通知（下载完成、备份失败、证书到期、上游告警） | `Notice` | `know` | Wonton 私聊（`gateway/notify.rs`） | 归属人 |
| 应用发来的信（找回密码、验证码） | `Mail` | `know` | Wonton 私聊（`mailbox/delivery.rs`） | 信头里的家里人 |
| 入口对账失败和恢复、联邦名单失败、频率超限、磁盘不够升级 | `Alert` | `know` | Wonton 私聊（`entry_runtime.rs`、`federation_config`、`notify.rs`） | 主人 |
| 好友、家人加入和移除、Bot 联系请求 | `Request` | `decide` | 列表（联系请求）、聊天（批准卡片） | 管理员；联系请求是 Bot 的归属人 |
| 别的服务器发来的配对请求 | `Request` | `decide`（不过期） | Wonton 私聊（`peers/mod.rs`） | 主人 |
| 文件发到另一台设备 | `File` | `know`（带“打开”） | Wonton 私聊 | 收件人 |
| 家人加入、移除的告知 | `Notice` | `know` | Wonton 私聊 | 主人 |
| 新设备登录 / 密钥过期 | `Security` | `decide`（critical） | 无（新） | 主人 |
| 磁盘将满、备份盘掉线、模型 key 失效、额度耗尽 | `Alert` | `know`（warning / critical） | 无（新，部分在插件通知里） | 主人 |
| 插件更新可用 / 成功 / 失败、核心升级结果 | `Notice` / `Result` | `know` | 无（新） | 管理员 |
| 用户自己记的待办 | `Task` | `do` | 无（新） | 自己 |
| 交给机器人的活 | `Task` | `do`（Bot 做时 `active`） | 无（新） | 自己 |
| Bot 建议的任务 | `Suggestion` | `do`（Suggested，不计角标） | 无（新） | 归属人 |
| worker 事件（做完、卡住、要确认） | `WorkerDone` / `WorkerStuck` / `WorkerNeeds` | `know` / `know` / `decide` | 只在 router 的对话里，模型自己记得 | router 服务的那个人（owner） |
| 壳不认识的新类型 | `Other` | — | — | 收件人，不显示但计数 |

`know` 的级别是 `info` / `warning` / `critical`，决定推不推、多急（第 9 节），不由壳猜。插件通知的载荷沿用 `PluginNotice`（ADR-0047），来信沿用 `PluginMail`（ADR-0056），文案仍由插件写、核心只补来源。核心自己的告警文字由核心按 App 的语言写（ADR-0043）。

### 3. 和聊天的关系

- **通知不再进私聊。** `NoticeRoom` 的写入停掉：插件通知、来信、配对请求、失败告警、频率提示、文件链接、家人告知都只进面板。私聊房间保留（旧消息、旧信里的链接还要能点），从聊天列表隐藏，面板最下面有一行“旧的通知记录”打开它；房间没有消息时这一行不出现。ADR-0118 起房间都不加密，私聊的“不加密”不再是特例，旧记录和聊天同一个机密性边界。家人各自的私聊一样处理。
- **确认卡片留在来源会话里的那一行。** 它是条目的一个投影，不是第二份事实：在会话里答和从面板里答是同一个请求、同一份回写；一边答了，另一边和所有设备一起变。来源没核对出来的卡片没有这一行，只在面板里。
- **Bot 的对话提问留在聊天，推荐不进面板。** 理由：答案要用到前面几轮的话，选项之外还要能自由打字；搬进面板等于把上下文也搬过去，一次问答被劈成两半。M1 的提问是聊天里的文字，核心看不到，面板里没有它。ADR-0038 的问题卡片落地后，卡片形态的提问才成为 `Question` 条目（`decide`，带 `origin`，来源会话里同时有那一行），那时才进面板。
- **交给机器人的活留在来源会话。** 用户在面板点“交给 Bot”时，核心在来源会话（没有来源就选这个 Bot 与用户的私聊）发一条带 `app.wonton.inbox_task` 的消息，把任务 ID 带在结构化字段里；Bot 的回复仍走聊天，进度只写回条目，不每条都刷聊天。做完的摘要两边都有：聊天里是 Bot 的回复，条目里是结果链接；只有一条事实。
- **会话里不加“有新待处理”的常驻提示。** 卡片行本身就是提示。全局提示条照旧（ADR-0016 第 5 条），只在主人不在面板、也不在这张卡片的来源会话里时出现，点了进条目的来源会话，没有来源就进面板。

### 4. 用户自己加待办、交给 Bot

**用户加待办**：面板底部有“+”输入框，只收标题和可选日期；默认 `do`、assignee 是自己、状态 `todo`。不做项目、标签、优先级、备注（第二版再加）。完成进 `handled`，24 小时内可撤销；改期只改 `due`，不改状态。删除是 `dismissed`，归档 30 天。

**交给 Bot**：`do` 条目上有一个“交给 Bot”动作，选一个已在的 Bot（M1 只有主人的 Bot 和家人的 Bot），可带一句说明。核心做三件事：把 assignee 改成 Bot、状态改 `active`、在会话里发 `app.wonton.inbox_task`。Bot 之后用 `wonton__report_progress`（ADR-0117）报 `state` / `phase` / `progress` / `result`，核心按 `worker_id`（就是条目 ID）更新同一条，不新建条目、不刷聊天。`done` 时条目进 `handled` 并留结果链接；`failed` 时进 `stuck` 带原因。用户可以在条目上“停止”，核心发 `AgentAction::StopTurns`（ADR-0061 第 10 节）；停止失败标 `uncertain`，不假装停了。

这不是调度器：不排期、不重试、不并发控制、不自动挑 Bot。Bot 自己跑多久、要不要用 Cron，是 Bot 的事（第 5 节）。

**Bot 建议的任务**是 `Suggestion`：归属人点“接受”才进 `do` 的 Backlog，点“忽略”就 `dismissed` 并记原因；不计角标，不进 Needs you。避免 Bot 自说自话往待办里塞东西。

### 5. 和 Hermes 的 Cron/任务、worker 登记表是什么关系

**事实源只有一处，App 只读核心给的 ViewModel：**

| 事实 | 存在哪 | 谁写 | App 怎么拿 |
| --- | --- | --- | --- |
| 面板条目的状态、内容、谁处理了 | 核心 `core/inbox` | 核心自己 | `InboxViewModel`，长轮询 |
| 定时任务的表达式、触发、重试 | Hermes 的 Cron / 任务配置 | Hermes | 不直接读；由 Bot 经事件写进核心 |
| 活着的 worker：谁、在哪、状态 | router 的登记表（herdr/tern） | router | 不直接读；`WorkerNeeds/Done/Stuck` 写进核心，条目只带 `worker_id` |
| 推送令牌、通知分类 | 核心推送代理 | 核心 | 入口数字和推送一起算 |

规则：

- **Cron 不是条目。** 一个每天 7 点的晨报跑成功，不生成面板条目；只有“要你知道 / 要你决定”的结果才写 `WorkerDone` / `WorkerNeeds` / `WorkerStuck`。失败连续两次才写一条 `WorkerStuck`，同一条按 `worker_id` 更新，不每次失败刷一条。
- **worker 登记表不进核心。** 是谁、在哪个 pane、用什么运行器，只有 router 知道；核心存的是展示快照（`worker_id`、名字、任务、地点、最后事件），不是登记表。App 打开 worker 条目只显示这些，不查 router；要交互就“打开终端”，由 router 的深链接。
- **桥死了靠对账。** router 每条 worker 挂一个等待，命令返回就写条目；钩子优先。桥挂、宿主重启、登记表对不上时，对账补一条 `WorkerStuck`，不自动把 worker 当完成。
- **两边不一致时以核心条目为准显示。** router 说 worker 还在、条目已经 `stuck`：保持 `stuck` 并写明最后事件，等 router 的下一条事件纠正。核心不猜。
- **事件接口**：`POST /_wonton/api/inbox/workers`（主人令牌或经隧道的主人设备），请求体
  `{worker_id, event: done|stuck|needs, name, task, place, message?, expires_at_ms?}`。同意和拒绝由 router 用运行器自己的命令执行（`tern approve`、`agent send-keys`），结果回写条目。同一个 worker 的同一类事件只留最新一条：`needs` 答完进 handled，worker 随后 `done` 另起一条 `know`，两条带同一个 `worker_id`；连续 `stuck` 更新那一条。登记表对不上时补 `WorkerStuck`。
- **重复规则以后再说。** M1 不存 Cron 表达式；要做“每周备份”这类重复待办时，再加 `repeat` 字段并决定由核心还是 Hermes 物化下一次，不现在定。

### 6. 排序、页面、空态、手势

- **顺序**：要你决定（最早到期的在前，逾期置顶）→ 要你做（逾期、今天到期、以后）→ 新通知（新在前）→ 已处理（最近处理的在前，24 小时内）。
- **第一版不做**搜索、筛选、分组设置、全部已读。段头就是分组；类型多起来再加筛选条。
- **行**：来源图标/头像、来源名、标题、一行说明、时间；`decide` 直接露按钮（最多三个，其余收进“更多”），`do` 有完成框和“交给 Bot”，`know` 有“知道了”和可选的“打开”。长文字默认收起。
- **点开**是完整条目：全文、附件、来源会话、处理记录、审计入口（敏感卡片）；从条目可以回到来源会话。
- **手势**：右滑是主要的可撤销动作（`do` 完成、`know` 已读、`decide` 稍后），左滑是“更多”（改期、拒绝、删除）；长按多选，底部批量“已读 / 归档”。批准/拒绝这类不可逆动作不藏进滑动，卡上始终有按钮。
- `decide` 的 Snooze 只把条目从当前段挪走、到点再回来，不延长卡片自己的期限；到期仍然过期。界面上写“还有 8 分钟”这类剩余时间，不让用户以为 Snooze 能续命。
- **空态**：托盘勾图标 + “都处理完了”，不加副标题；已处理为空时那一段不显示。
- **读屏**读出 lane（“要你决定”“要你做”“新通知”）、来源、到期时间和按钮；滑动动作有等价按钮。
- 外观全部用现有令牌和组件（ADR-0026、ADR-0029），不新增颜色；手机竖屏 390×844 下按钮不换行，长标题两行截断。

### 7. 入口和角标

- **入口**：聊天列表顶部固定一行，图标用 Phosphor 的托盘，标题“待处理”，右边数字；零计数时标题用次要色，仍能点。侧边栏也有一行，位置在“新聊天”下面。不放回底栏（ADR-0095 已去掉底栏）。
- **角标 = 待处理的 `decide` + 今天到期/逾期的 `do`。** 应用角标（ADR-0101 第 3 节）从“未读消息”扩成“未读消息 + 这个数”，两者都清零才归零。
- `know` 不进角标；面板的“新通知”段头有未读数，`critical` 的 `know` 仍推送、横幅更醒目，但数字不加。`active` 的 Bot 任务不算“要你动”，不进角标；卡住转 `stuck` 才进。
- **打开面板不归零**，逐条处理才减；单独点“知道了”只减 `know` 的段头数字。
- 全部处理完时入口变灰、角标清零；系统角标由核心给绝对数，不每台设备各自加。

### 8. “云吞”私聊怎么处置

- 停止写入，从聊天列表隐藏，房间保留为只读归档；面板最下面“旧的通知记录”打开。旧信里的找回密码链接、文件链接继续能点。
- 家人各自的私聊同样处理；家人的旧记录只有家人自己看得到。
- 第一版不删房间。90 天后另开裁剪任务：确认没有活跃链接、导出审计后再删。
- 新 App 对上旧核心时，列表 404 就照实显示加载失败；旧 App 见到新 `kind` 落 `Other`，只显示角标不显示条目，不白屏。

### 9. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `inbox` 模块（`InboxItem`、`Lane`、`Kind`、`Status`、`InboxList`、`InboxViewModel`、路径、已读/完成/稍后/交办请求）。契约测试：未知类型落 `Other`、新增字段有默认值、管理员条目的 `handled_elsewhere`、按收件人过滤、`know` 的已读按人分开、`decide` 过期不能回答、`do` 完成后 24 小时内可撤销。
- **接口**：
  - `GET /_wonton/api/inbox?after=<revision>`，长轮询 25 秒，返回 `InboxList { decide, do, know, handled, revision, counts }`；
  - `GET /_wonton/api/inbox/{id}` 取一条；
  - `POST /_wonton/api/inbox/{id}/read`、`/done`、`/snooze`、`/dismiss`、`/reopen`；
  - `POST /_wonton/api/inbox` 建个人待办；`POST /_wonton/api/inbox/{id}/delegate` 交给 Bot；`POST /_wonton/api/inbox/{id}/progress` 给 Bot；
  - `POST /_wonton/api/inbox/workers` 给 router；
  - `decide` 的回答沿用各自的接口：确认卡片还是 `POST /_wonton/confirmations/{id}`，配对、联系请求还是它们自己的接口；条目的状态由结果的回写驱动，壳不直接改条目。
- **提供方**：核心。现有来源（确认表、通知入口、来信投递、配对请求、联系人请求、升级流程）都改成写条目，不再写私聊。
- **消费方**：两端的壳（页面、入口、通知动作）和客户端核心（长轮询、ViewModel、提示条）。
- **换提供方不改消费方**：条目和 `kind` 是核心给的，壳按 `lane` 渲染；以后加告警来源（例如核心自己探磁盘）不用改任何壳。
- **鉴权**：用户操作只认经隧道的已配对设备或主人令牌；Bot 的 `progress` 认 Bot 身份；worker 事件认主人令牌或 router 的隧道身份。插件网络里来的请求没有隧道身份，答不了卡片。
- **推送**：一个条目一条通知，不重复推；同一条目的进度更新只替换原通知，不每条弹一次；`decide` 中不带表单的卡片在通知上给“允许 / 拒绝”，带表单的打开条目；`know` 的 `info` 不推；`critical` 用 Time Sensitive。推送正文按 ADR-0118 去换行、截 120 字，附件只写文件名。
- **隐私**：条目在核心里是明文，核心用设备公钥加密后交给中继；锁屏内容分类开关和 App 锁照旧（ADR-0101 第 5 节、ADR-0102）。面板页在锁定期间不露内容。

### 10. 第一版不做

项目、看板、标签、里程碑、依赖、多人分派、重复规则、自动路由、搜索、筛选、全部已读、`Someday`/`Anytime` 这类时间层、`do` 的任务附件；通知仍只进面板、不进 Matrix 房间，别的 Matrix 客户端仍答不了卡片。

## 要主人拍板

1. **面板范围**：A 只做通知箱（`decide` + `know`，不存待办、不能交给 Bot）；B 收件箱 + 待办（三 lane、可自己加、可交给 Bot，不做项目）；C 完整任务台（项目、看板、重复、分派）。**推荐 B**：A 回答不了“todo 那种”，C 会把核心变成任务系统。
2. **自己加待办**：不能；能，只收标题和可选日期；能，带项目/标签/优先级。**推荐“只收标题和日期”**。
3. **交给 Bot**：不能；能，一次性委派，带一句说明；能，按类型自动路由。**推荐“一次性委派”**。
4. **角标**：只算 `decide`；算 `decide` + 今天到期/逾期的 `do`；算全部未处理（含 `know`、`active`）。**推荐第二种**；`know` 不算。
5. **“云吞”私聊**：保留为普通聊天；隐藏、只留只读归档；30 天后删除。**推荐“隐藏 + 只读归档”**，先不删。
6. **Bot 在聊天里提问**：留聊天（M1）；镜像到面板、回答仍在聊天；全部搬进面板。**推荐留聊天**，等 ADR-0038 问题卡片后再进面板。
7. **入口**：聊天列表顶部一行 + 侧边栏；只放侧边栏；回到底栏 tab。**推荐第一种**。
8. **和 Hermes Cron/任务、worker 登记表的关系**：面板只显示事件，调度全在 Hermes、登记表全在 router（推荐）；核心镜像 Hermes 的任务；核心自己存 Cron。**推荐第一种**。
9. **保留**：已处理 24 小时 + 归档 30 天（推荐）；7 天 + 90 天；永久保留。
10. **Snooze**：固定 1 小时 / 今天晚些 / 明天 / 下周（推荐）；自由日期；不做 Snooze。
11. **完成可撤销**：24 小时内可撤销（推荐）；不可撤销、二次确认；完成即删除。
12. **`know` 是否也进角标**：不进（推荐）；`warning` 及以上进；全部进。

## 没选的做法

- **继续用“云吞”私聊，只加几个过滤器。** 私聊是 Matrix 房间，过滤和已读要自己再写一层；清了消息找不回来，长信件仍挤聊天流。面板本来就得为卡片存在，扩它比再做一个视图省。
- **只做通知箱，不做待办。** 用户还是要打开聊天记“买牛奶”，面板回答不了“今天要做什么”；交给 Bot 也没有落点。
- **做成完整任务台。** 项目、看板、重复、责任域一套下来，核心变厚，第一版拖很久，还和 Hermes 的 Cron/任务打架。
- **每类一个页面**（通知页、来信页、请求页）。类型一多，用户要挨个翻；面板只分三段。
- **只做系统通知中心，不做 App 内页面。** 系统和分发器会把通知清掉，清掉就找不回来，ADR-0101 的失败路径也要卡片自己留住。
- **把 Bot 提问也搬进面板。** 见第 3 节。
- **把 `know` 也做成要点的确认。** 下载完成也要按一次按钮，处理量翻倍；`know` 只要求已读。
- **入口只放在设置或侧边栏。** 主管理后台要一眼看到，放在聊天列表顶部才够近。
- **条目照旧发一份进 Matrix 房间，让别的客户端也能看。** 一份事实变两份，读和答都要对账；别的 Matrix 客户端本来也答不了卡片。
- **第一版就做稍后提醒、搜索、筛选。** 条目类型和量都还没跑过；Snooze 先做，因为它决定“放哪”和“角标怎么减”。
- **管理员条目每人一份拷贝。** 一份条目、每人一份已读/回答状态才不会漂，回答后别人的那份直接变“已由某人处理”。

## 代价与风险

- **面板变成新的噪音源。** 插件通知、来信全进来以后，面板可能一天几十条。缓解：`info` 不推送、不占角标；插件每分钟 10 条、每天 200 条的上限照旧（ADR-0047）；`know` 30 天清掉；批量清。
- **角标压力。** 未读消息加待处理可能让角标一直很大；只有真的处理或读才减，而不是打开 App。
- **核心变成小任务管理器。** 边界是“三个 lane、一个 assignee、一个 due、五个状态”；谁要项目/标签/重复，另开 ADR，不往这张表里塞。
- **卡片的两个投影可能被看成两件事。** 靠同一 ID、同一回写和“处理一边两边一起变”的测试兜住。
- **worker 桥是新增的单点。** 它挂了就没有事件；靠登记表对账补 `WorkerStuck`，桥本身要做成能被 supervisor 拉起的长驻进程。
- **私聊退休会断旧链接。** 旧信里的找回密码地址不受影响（在消息正文里），但“去私聊找那封信”的习惯要换；房间保留就是为了这个。
- **家人多、管理员多时**，管理员条目对每个管理员都推一条，同一件事会在几台设备上各弹一次；这是“谁先答算谁的”的代价（ADR-0088）。
- **App 和核心分开升级时**：旧 App 见到新类型只显示角标不显示条目；新 App 对上旧核心时列表 404，照实显示加载失败。
- **没有服务端搜索。** 第一版条目少，靠段头和来源会话翻；量上来再加。

## 待验证

- 390×844 下入口行、带两个按钮的决定卡、长标题两行；大字号不截断。
- 家人 A 的条目不出现在家人 B 的面板；管理员条目在 A 答后 B 显示“已由 A 处理”且推送撤下。
- 通知上答没有表单的卡片，答案执行、条目进已处理、另一台设备的通知消失。
- 自己建的待办能完成、改期、交给 Bot；交给 Bot 后进度只更新条目，聊天里只有任务消息和最终回复；停止失败标 `uncertain`。
- worker 卡住时写一条 `WorkerNeeds`，在通知上允许后条目进已处理、worker 继续跑；worker 随后退出，`WorkerDone` 只留最新一条。
- worker 桥被杀、宿主重启后，登记表对账能给失联的 worker 补 `WorkerStuck`，不把 `active` 自动当 `done`。
- 面板最下面“旧的通知记录”能打开旧私聊，隐藏后不再出现在聊天列表。
- 来信 30 天、已处理 24 小时、归档 30 天的清理；核心重启后条目还在、长轮询接着走。
- 未知 `kind` 落 `Other`、旧壳不白屏；新增字段有默认值。
- 端到端：装插件、收一封应用来信、触发一条 `critical` 通知、另一台服务器发配对请求，四件事都只进面板、私聊为空；再让一个 worker 卡住，条目出现、在通知上允许后它继续跑，随后完成只剩一条“做完了”。

## 查证来源

- 调研页（九家逐项对比、完整来源清单）：`~/wonton-dev/tasks/inbox-design.html`。
- Linear Inbox：https://linear.app/docs/inbox （Priority / Other、Snooze、2000 条上限、已读/未读、删除）。
- Linear Triage：https://linear.app/docs/triage （Accept / Decline / Duplicate / Snooze、Triage Rules、Responsibility）。
- Linear My Issues：https://linear.app/docs/my-issues （指派/创建/订阅/活动、focus order）。
- GitHub 通知：https://docs.github.com/en/subscriptions-and-notifications/concepts/about-notifications
  （订阅、reason、Done / Saved、3 个月保留）；收件箱操作页 https://docs.github.com/en/subscriptions-and-notifications/how-tos/viewing-and-triaging-notifications/managing-notifications-from-your-inbox
  （Done / Save / Unsubscribe / 筛选 / 批量）。
- Slack Activity：https://slack.com/help/articles/19693583638803-Get-your-work-done-from-the-Activity-view
  （Unreads/DMs/Mentions/Threads/Reminders/VIP、清除、已清除）；Later → To-dos：https://slack.com/help/articles/55757634348435-Later-is-now-To-dos
  （To-dos 替代 Later、完成、过滤/排序）；保存稍后：https://slack.com/help/articles/13453851074067-Save-it-for-Later 。
- Things 日期列表：https://culturedcode.com/things/support/articles/4001304/ （Today/Upcoming/Anytime/Someday、Inbox 暂存、Logbook）；
  手势：https://culturedcode.com/things/support/articles/2803582/ （右滑 When、左滑选择、拖拽改期、多选）。
- Todoist Inbox：https://www.todoist.com/help/todoist/get-started/use-the-inbox-in-todoist-HwHvYErS ；
  滑动动作：https://www.todoist.com/help/todoist/features/how-to-change-your-swipe-actions-D5DQOQz6 ；
  日期：https://www.todoist.com/help/todoist/features/schedule-a-date-and-time-for-your-todoist-tasks-q7VobO 。
- 滴答清单任务管理：https://xwzappest.gitbooks.io/guide-dd/content/android_app/3_manage_tasks.html （收集箱、左滑改期/移清单、右滑归档/优先级、长按多选、计划模式）；
  清单与智能清单：https://help.dida365.com/articles/6950654234933067776 。
- Apple 提醒事项智能列表：https://support.apple.com/guide/iphone/use-smart-lists-iphe882772ed/ios ；
  完成：https://support.apple.com/guide/iphone/complete-items-iph3fb74d597/ios 。
- Superhuman Inbox Zero：https://superhuman.com/learn/email/inbox-zero-method （Clear / Defer / Make it a to-do、E/H/J、Split Inbox、Auto Archive）；
  Remind Me：https://help.superhuman.com/hc/en-us/articles/46005666142733-Remind-Me 。
- Alma Home：https://docs.alma.team/workspace/home （常驻对话、Needs you 在 thread 里、决定与 Tasks 同一份写入）；
  Alma Tasks：https://docs.alma.team/workspace/tasks （What/Why/Done when/Assignee/Planned/Under、Needs you/Active/Backlog/Suggested、Snooze 四档、Accept/Dismiss）。
- Apple HIG `Notifications`：https://developer.apple.com/design/human-interface-guidelines/notifications （最多四个动作、角标只表示未读、
  同一件事不要重复通知）。
- herdr CLI：https://herdr.dev/docs/cli-reference （`agent wait --until done,blocked`、`pane report-agent --state blocked|done`、`agent send-keys`、`pane read`）。
- tern CLI：https://tern.build/docs/cli （`tern wait --for done,needs`、`tern status -json`、`tern approve allow|always|deny`）。

## 关联

ADR-0005（厚核心）、ADR-0015（小组件）、ADR-0016（确认卡片和列表）、ADR-0003/0101（推送和通知上的动作）、ADR-0038（问题卡片）、
ADR-0118（推送带正文、房间不加密）、ADR-0117（Bot 长任务进度）、ADR-0082（运行状态）、
ADR-0043（语言）、ADR-0045/0088（家人、管理员）、ADR-0047（插件通知）、ADR-0050（配对请求）、ADR-0056（内部邮箱）、
ADR-0095（没有底栏）、ADR-0102（App 锁）、ADR-0026/0029（外观）。
