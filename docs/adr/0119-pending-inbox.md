# ADR-0119 待处理面板：要用户选的事和要用户知道的通知，统一进核心里的一页

- 状态：提议（2026-11-25）。方向由主人定（11-25：工作助理和生活助理都该有一个主管理后台，让用户清楚知道要处理什么，
  而不是只能翻对话；现在专门建一个“云吞”对话塞权限请求，不合理）。要主人拍板的取舍见“要主人拍板”
- 取代：[ADR-0016](0016-mcp-bus-and-confirmations.md) 第 2 步里的“等你确认”列表（`GET /_wonton/api/confirmations` 的列表、
  聊天列表里的入口和它的角标）。卡片本身、来源会话、回答接口都不变，只是列表和入口换成面板
- 修订：[ADR-0047](0047-plugin-notices.md)（插件通知不再发进 Wonton 私聊）、[ADR-0056](0056-internal-mailbox.md)（应用来信同上）、
  [ADR-0045](0045-family-members.md)（文件链接、家人加入的通知同上）、[ADR-0050](0050-cross-server-chat.md)（配对请求同上）、
  [ADR-0101](0101-notifications.md) 第 3 节（应用角标的数里加上待处理条目）
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（推送链路）、[ADR-0118](0118-im-three-layers-push-with-body.md)（推送带正文、房间不再端到端加密）、
  [ADR-0038](0038-multi-step-cards.md)（问题卡片，M1 不做）、
  [ADR-0088](0088-family-accounts.md)（管理员卡片）、[ADR-0102](0102-app-lock.md)（App 锁）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)（外观）、
  [ADR-0005](0005-native-clients-thick-core.md)（厚核心，壳只渲染）
- 实施拆分：协议 → 核心 → 两端壳 → 拆私聊 → 派活事件（router 的 worker 桥），每步的范围和验证见任务结果文件 `~/wonton-dev/tasks/adr-inbox.result.md`。

## 用户看到什么

打开云吞，聊天列表最上面固定一行“待处理”，右边的数字是有几件事等你。点进去是一列：机器人想装个应用、家人想加好友，
卡片上直接有按钮，当场就能点；派出去干的活回来了、卡住要你批准，也是这一列里的卡片；硬盘快满了、下载完了、应用发来一封找回密码的信，看一眼点“知道了”，带地址的另有“打开”。
处理过的挪到最下面，写着谁在什么时候处理的，过一天自动消失。没有事情时这一行变灰，页面里写着“都处理完了”。聊天列表里
只剩和机器人、和人的对话，原来那个“云吞”对话不再用来塞通知，旧消息还能从页面最下面翻回去。

## 背景

主人 2026-11-25 的原话要点：助理类产品都该有一个类似“主管理后台”的地方，让用户清楚自己要处理哪些东西；有的要用户做选择，
有的只要用户知道（点“已读 / 知道了”），本质上都是推给用户的通知，只是类型不同。这类往往重要、全局，不该只在对话流里出现。

现在的两套东西都不完整：

- **通知往对话里塞。** ADR-0016 曾把确认卡片发进“Wonton”私聊；卡片搬到会话里以后，私聊继续接插件通知
  （`crates/wonton-core/src/gateway/notify.rs:93`）、应用来信（`crates/wonton-core/src/mailbox/delivery.rs:289-320`，每人一个私聊）、
  文件链接、配对请求（`crates/wonton-core/src/peers/mod.rs:645-655`）、入口对账和联邦名单的失败（`crates/wonton-core/src/entry_runtime.rs:200`、
  `crates/wonton-core/src/matrix/federation_config/mod.rs:124-131`）、频率超限提示（`gateway/notify.rs:491`）。私聊在聊天列表里就是一间
  普通房间：清了以后找不回来，没读的混在消息里，长通知（一封信）挤占聊天流。
- **“等你确认”列表只装卡片。** 它是 ADR-0016 第 6 条加的，列等回答和结束 24 小时内的卡片，另有 Bot 联系请求
  （`crates/wonton-protocol/src/confirmation.rs:234-250`）。核心里有表、有版本号、长轮询、推送、按人过滤
  （`crates/wonton-core/src/mcp/confirm.rs:480`、`crates/wonton-core/src/gateway/confirmations.rs:34`）；两端有页面
  （`apps/ios/Wonton/ConfirmationsListView.swift:4`、`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/ConfirmationsScreen.kt:78`）、
  聊天列表顶部的入口和角标（`apps/ios/Wonton/ChatListView.swift:98`、`ChatScreen.kt:238`）和全局提示条
  （`apps/ios/Wonton/MainView.swift:206`）。这些都能接着用，缺的是把它扩成“所有推给用户的事”，和把对话从通知里摘出来。

已经有的地基，不用新造：

- 条目在核心里、由核心给 ViewModel、壳只渲染（AGENTS.md“厚核心薄壳”）。
- 按收件人过滤：卡片发给 Bot 的归属人，或所有管理员，谁先答算谁的（ADR-0088）；列表按请求者只列自己的
  （`crates/wonton-core/src/mcp/confirm.rs:480-486`）。
- 结束的卡片留 24 小时（`mcp/confirm.rs:38`）；拒绝、过期、已经被谁答了（`answered_by`）都已经有状态。
- 推送按类别、没有表单的卡片能在通知上直接答（ADR-0101 第 1 节）；通知的分类开关和 App 锁（ADR-0101 第 5 节、ADR-0102）。
- 聊天列表的入口、角标、零计数时的次要色、空态、提示条不重复显示的规则（ADR-0016 第 2 步、第 6 条）。

## 决策

### 1. 一个条目，两类，核心里一张表

**条目在核心里**（`core/inbox`，权限 0600），由核心给 `InboxViewModel`，两端壳只渲染、把操作发回核心。
一条条目有：ID、类型、状态、收件人、时间、到期时间、来源（Bot、插件、核心）、可选的来源会话（`origin`），以及按类型的载荷。

**两类**：

| 类型 | 用户要做什么 | 载荷 |
| --- | --- | --- |
| `choose`（要选择） | 在一组按钮里选一个 | 标题、说明行、按钮；确认卡片本身就是一种 `choose` |
| `know`（要知道） | 看一眼，点“知道了”（已读） | 标题、正文、可选的附件或链接、级别 |

类型是带 `Other` 的枚举（`#[serde(other)]`，AGENTS.md 的兼容规则）：壳不认识的新类型落到 `Other`，不显示；核心仍把它算进入口的数字，
旧壳上表现为数字比看得到的条目多。
条目按来源细分 `kind`（`Confirmation`、`Question`、`Notice`、`Mail`、`Alert`、`Result`、`Request`、`File`、`WorkerNeeds`、`WorkerDone`、`WorkerStuck`、`Other`），壳按 `kind` 选行和卡片的样式；
`kind` 不决定能不能操作，操作只看 `choose`/`know` 和载荷里的按钮。

**状态**：

- `pending`：`choose` 等回答，`know` 未读。
- `handled`：已处理。`choose` 带结果（同意、拒绝、执行成功、失败、结果不确定），`know` 是已读。带处理人和时间。
- `expired`：到期没答。卡片沿用 10 分钟，其他 `choose` 跟来源的期限；`know` 不过期。
- `handled_elsewhere`：发给所有管理员的条目被另一个管理员答了。显示“已由某人处理”，不能再点，仍是结果状态。

**已读只属于个人**：`know` 条目发给多个人时各记各的已读；`handled` 是唯一的（谁先答算谁的）。

**谁能看到**：收件人在创建时定下，之后不变，只有两种——一个人（Bot 的归属人、家人的自己那份）或所有管理员。
列表按请求者过滤，家人看不到主人的、别人的，主人也不替家人看。管理员条目对每个管理员都出现，回答后其余人的那份变成
`handled_elsewhere`。入口的数字只数自己的。

**保留**：等回答的按来源的期限；处理过的留 24 小时（同卡片）；没人读的 `know` 30 天后丢弃；`choose` 过期后留 24 小时。
等回答的卡片重启后靠期限显示成过期（ADR-0016），其余条目在盘上，重启后照原样列出。
状态先落盘再改版本号；落盘失败时如实报错，不假装已处理。

**接口**（`wonton-protocol` 的 `inbox` 模块）：

- `GET /_wonton/api/inbox?after=<revision>`，长轮询 25 秒，返回 `InboxList { pending, unread, handled, revision }`；
  `pending` 是等回答的 `choose`，`unread` 是没读的 `know`，`handled` 是 24 小时内处理过的（两类都在里面）；
- `GET /_wonton/api/inbox/{id}` 取一条，点开通知时用；
- `POST /_wonton/inbox/{id}/read`，`know` 标记已读；
- `choose` 的回答沿用各自的接口：确认卡片还是 `POST /_wonton/confirmations/{id}`，配对、联系请求还是它们自己的接口；
  条目的状态由结果的回写驱动，壳不直接改条目。

ID 由核心分配，带来源前缀（`c-` 卡片、`n-` 其他），和 Matrix 事件 ID 无关。`revision` 是全局版本：任何一条新增、改状态、
某人读了都加一。旧路径 `GET /_wonton/api/confirmations` 和 `confirmation_path` 由 `/_wonton/api/inbox` 取代，回答路径不变。

### 2. 来源：现有的每一处都迁进来

| 来源 | `kind` | 类 | 现在的去向 | 谁看得见 |
| --- | --- | --- | --- | --- |
| 插件工具、装/卸/升级、存文件、替 Bot 发消息、链上限、家人加入批准、好友批准 | `Confirmation` | `choose` | 会话里的卡片行 + 列表 | 归属人；要管理员批的是所有管理员 |
| Bot 在对话里问的问题 | `Question` | `choose` | 聊天里的文字 | 归属人（问在哪个房间就带哪个 `origin`） |
| 安装、升级的结果 | `Result` | `know` | 卡片原地变成结果；核心自己的升级和插件升级结果只进私聊 | 发起的人 |
| 插件通知（下载完成、备份失败、证书到期、上游告警） | `Notice` | `know` | Wonton 私聊（`gateway/notify.rs`） | 归属人 |
| 应用发来的信（找回密码、验证码） | `Mail` | `know` | Wonton 私聊（`mailbox/delivery.rs`） | 信头里的家里人 |
| 入口对账失败和恢复、联邦名单失败、频率超限、磁盘不够升级 | `Alert` | `know` | Wonton 私聊（`entry_runtime.rs`、`federation_config`、`notify.rs`） | 主人 |
| 好友、家人加入和移除、Bot 联系请求 | `Request` | `choose` | 列表（联系请求）、聊天（批准卡片） | 管理员；联系请求是 Bot 的归属人 |
| 别的服务器发来的配对请求 | `Request` | `choose`（不过期） | Wonton 私聊（`peers/mod.rs`） | 主人 |
| 文件发到另一台设备 | `File` | `know`（带“打开”） | Wonton 私聊 | 收件人 |
| 家人加入、移除的告知 | `Notice` | `know` | Wonton 私聊 | 主人 |
| worker 事件（做完、卡住、要确认） | `WorkerDone` / `WorkerStuck` / `WorkerNeeds` | `know` / `know` / `choose` | 只在 router 的对话里，模型自己记得 | router 服务的那个人（owner） |
| 壳不认识的新类型 | `Other` | — | — | 收件人，不显示 |

`know` 的级别是 `info` / `warning` / `critical`，决定推不推、多急（第 4 节），不由壳猜。
插件通知的载荷沿用 `PluginNotice`（ADR-0047），来信沿用 `PluginMail`（ADR-0056），文案仍由插件写、核心只补来源。
核心自己的告警文字由核心按 App 的语言写（ADR-0043）。

**worker 事件。** 主人现在的做法是“协调会话 + 一群 worker”（herdr、tern 这类终端运行器）。worker 做完、卡住（等批准或等回答）、
自己退出时，由中间层写一条条目，router 和主人都不用盯着终端：

- `WorkerNeeds`（`choose`）：worker 在等批准或等一个回答。显示 worker、任务、在哪（宿主、工作树、pane 或 session）和运行器给的选项；
  tern 是 `approve allow|always|deny`，直接回；herdr 的批准是按键，条目给“打开”和“回车”“Esc”两个键。
- `WorkerDone`（`know`）：做完或退出了，带结果地址（工作树、分支、PR、会话），另有“打开”。
- `WorkerStuck`（`know`，`warning`）：超时、失联、连续出错，或者运行器的列表和登记表对不上。`WorkerNeeds` 的期限跟运行器自己的等待超时，超时后转 `WorkerStuck`。

登记表（谁、做什么、在哪、状态）在 router 那一层，不进核心；条目带 `worker_id`，同一个 worker 的同一类事件只留最新的一条。

**事件怎么来。** 两个运行器都没有推送接口，只有阻塞等待和状态查询：tern 有 `tern wait <session> --for done,needs`、`tern status -json`、
`tern approve`，herdr 有 `herdr agent wait --until done,blocked`、`agent read`、`pane report-agent --state blocked|done`（官方集成的钩子）。
所以中间层持一个 worker 桥：每个 worker 挂一条等待，命令返回就写条目再挂上；有钩子的以钩子为准。router 不轮询，模型不用记住派过谁。

接口：`POST /_wonton/api/inbox/workers`（主人令牌或经隧道的主人设备），请求体
`{worker_id, event: done|stuck|needs, name, task, place, message?, expires_at_ms?}`。同意和拒绝由 router 用运行器自己的命令执行
（`tern approve`、`agent send-keys`），结果回写条目；桥死了或宿主重启，登记表对账时补 `WorkerStuck`。

### 3. 和聊天的关系

- **通知不再进私聊。** `NoticeRoom` 的写入停掉：插件通知、来信、配对请求、失败告警、频率提示、文件链接、家人告知都只进面板。
  私聊房间保留（旧消息、旧信里的链接还要能点），从聊天列表隐藏，面板最下面有一行“旧的通知记录”打开它；房间没有消息时这一行不出现。
  ADR-0118 起房间都不加密，私聊的“不加密”不再是特例，旧记录和聊天同一个机密性边界。家人各自的私聊一样处理。
- **确认卡片留在来源会话里的那一行。** 它是条目的一个投影，不是第二份事实：在会话里答和从面板里答是同一个请求、同一份回写；
  一边答了，另一边和所有设备一起变。来源没核对出来的卡片没有这一行，只在面板里。
- **会话里不加“有新待处理”的常驻提示。** 卡片行本身就是提示。全局提示条照旧（ADR-0016 第 5 条），只在主人不在面板、
  也不在这张卡片的来源会话里时出现，点了进条目的来源会话，没有来源就进面板。
- **Bot 的对话提问留在聊天，推荐不进面板。** 理由：答案要用到前面几轮的话，选项之外还要能自由打字；搬进面板等于把上下文也搬过去，
  一次问答被劈成两半。M1 的提问是聊天里的文字，核心看不到，面板里没有它。ADR-0038 的问题卡片落地后，卡片形态的提问才成为
  `Question` 条目（`choose`，带 `origin`，来源会话里同时有那一行），那时才进面板。

### 4. 和通知的关系

每条推送都带标题和正文，和消息推送同一条路（ADR-0118）：条目在核心里是明文，核心用设备公钥加密后交给中继，手机本机解开就显示；
不依赖房间密文，也不起通知扩展解 Megolm。正文按 ADR-0118 的规则去换行、截到 120 个字符，放不下时按 来源名 → 标题 → 正文 丢字段，
附件只写文件名。带表单、带密钥或正文更长的点开条目看全。

推送和面板共用一份条目：**一个条目一条通知**，不重复推，也不为同一件事再发一条。

| 条目 | 系统通知 | 通知上能做什么 |
| --- | --- | --- |
| `Confirmation`（没有密钥、设置表单） | 确认类别，高 | “允许”“拒绝”，同 ADR-0101 第 1 节 |
| `Confirmation`（带表单）、`Question` | 确认类别，高 | 只“打开”；表单和选项在条目里 |
| `Request`（Bot 联系人、配对请求） | 确认类别，高 | “同意”“拒绝”；配对请求和已配对的服务器重名时只“打开” |
| `Notice` `warning`/`critical`、`Alert` | 插件通知类别，按级别；`critical` 用 Time Sensitive | “知道了” |
| `Notice` `info` | 不推，只进面板 | — |
| `Mail`、`File` | 插件通知类别 | “打开” |
| `WorkerNeeds` | 确认类别，高 | `tern approve` 的三个选项；herdr 给“打开”“回车”“Esc” |
| `WorkerDone` | 插件通知类别，普通 | “打开” |
| `WorkerStuck` | 插件通知类别，`warning` | “打开” |

- **处理后各端同步消失**：回答或已读都会让 `revision` 变化，挂着的长轮询立刻回来；每台设备把这条通知撤下，条目挪进已处理。
  在通知上答也算处理，和 App 里答一样。撤不掉的（App 被杀、分发器丢了）等下一次同步或到期，由核心的状态兜底。
- **入口的数字和应用角标由核心算**：入口数字 = 待处理的 `choose` + 未读的 `know`。应用角标（ADR-0101 第 3 节）从“未读消息”
  扩成“未读消息 + 待处理条目”，两者都清零才归零。打开面板不归零，逐条处理才减。
- **隐私**：推送带正文，锁屏内容分类开关照旧（确认 / 通知 / 消息）管着；小米、OPPO、vivo 和 HMS 退回通知栏消息的渠道按 ADR-0118 只写“新消息”。
  面板页面在 App 锁锁定期间不露内容（ADR-0102）。来信和文件只在收件人自己的面板里；机密性边界和聊天相同（ADR-0118：服务器 root、托管宿主、对方服务器读得到）。

### 5. 入口、页面和空态

- **入口**：聊天列表顶部固定一行，图标用 Phosphor 的托盘，标题“待处理”，右边数字；零计数时标题用次要色，仍能点。
  侧边栏也有一行，位置在“新聊天”下面。iPad 内容栏打开这一页，左列的会话列表不动（ADR-0103）。
- **页面**：标题“待处理”。从上到下：要处理（`pending` 的 `choose`，最早到期的在前）、新通知（未读的 `know`，新的在前）、
  已处理（24 小时内，最近处理的在前）。“旧的通知记录”在最下面。第一版不做搜索、筛选、分组设置和“全部已读”。
- **行**：图标（来源的 Bot 头像或插件图标，核心的用核心图标）、来源名、标题、一行说明、时间；`choose` 直接露出按钮，
  点开是完整条目。长文字默认收起，点开再看。
- **空态**：托盘里的勾图标和一句“都处理完了”，不加副标题；已处理为空时那一段不显示。
- 外观全部用现有令牌和组件（ADR-0026、ADR-0029），不新增颜色；手机竖屏 390×844 下按钮不换行，长标题两行截断。
  读屏读出类型（“要你选”“新通知”）、来源和状态。

### 6. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `inbox` 模块（条目、类型、状态、`InboxList`、`InboxViewModel`、路径和已读请求）。契约测试：
  未知类型落 `Other`、新增字段有默认值、管理员条目的 `handled_elsewhere`、按收件人过滤、`know` 的已读按人分开。
- **提供方**：核心。现有来源（确认表、通知入口、来信投递、配对请求、联系人请求、升级流程）都改成写条目，不再写私聊。
- **消费方**：两端的壳（页面、入口、通知动作）和客户端核心（长轮询、ViewModel、提示条）。
- **换提供方不改消费方**：条目和 `kind` 是核心给的，壳按 `choose`/`know` 渲染；以后加告警来源（例如核心自己探磁盘）
  不用改任何壳。

## 要主人拍板

1. **私聊退休**：通知不再进“云吞”对话，房间从聊天列表隐藏，旧记录留在面板底部。要不要连旧房间一起删（删了旧信里的链接会断）。
2. **入口位置**：聊天列表顶部一行 + 侧边栏一行；不放回底栏（ADR-0095 已去掉底栏）。
3. **Bot 提问**：留在聊天；ADR-0038 的问题卡片落地后再进面板。
4. **应用角标**：未读消息 + 待处理条目一起算；只算消息的话，面板的数字会比角标大。
5. **第一版不做**：稍后提醒（snooze）、搜索、按来源筛选、“全部已读”。

## 业内参考

| 做法 | 出处 | 抄什么、不抄什么 |
| --- | --- | --- |
| 一个 “Needs you” 列表，每行写清要什么；决定就在卡片上；同一个请求在来源对话里也出现，两边是同一份写入 | Alma `Home`（“What is waiting on you arrives in the thread as Alma's own turns… The decision is the same one you would make in Tasks”）、`Tasks` 的 Needs you / Active / Backlog 分组（docs.alma.team/workspace/home、/workspace/tasks） | 抄“一个列表 + 要什么一行 + 卡片上决定 + 与来源对话同一份”；不抄 Active/Backlog/Suggested 三套分组，我们的条目只有待处理和已处理 |
| 收件箱是唯一的通知中心，Done / 未读分开，Done 能回看 | Linear Inbox（linear.app/docs/inbox） | 抄 Priority 与其余分开（对应“要选择”和“要通知”）；不抄快捷键、搜索和 2000 条上限 |
| Done、已读/未读、按原因筛选、保留期限 | GitHub 通知（docs.github.com/…/concepts/about-notifications） | 抄 Done 与未读分开、处理过的可回看；不抄按仓库/原因的多级筛选 |
| 通知中心是 App 外的第二层：按应用分组、最多四个动作、Passive/Active/Time Sensitive 分级、角标只表示未读 | Apple HIG `Notifications`、`Managing notifications` | 抄按类型定紧急程度、动作只给能当场做完的、角标跟着未读走；不抄把任务说明塞进通知正文 |

## 没选的做法

- **继续用“云吞”私聊，只加几个过滤器。** 私聊是 Matrix 房间，过滤和已读要自己再写一层；清了消息找不回来，长信件仍挤聊天流。
  面板本来就得为卡片存在，扩它比再做一个视图省。
- **每类一个页面**（通知页、来信页、请求页）。类型一多，用户要挨个翻；面板只分“要处理”和“要知道”。
- **只做系统通知中心，不做 App 内页面。** 系统和分发器会把通知清掉，清掉就找不回来，ADR-0101 的失败路径也要卡片自己留住。
- **把 Bot 提问也搬进面板。** 见第 3 节。
- **把 `know` 也做成要点的确认。** 下载完成也要按一次按钮，处理量翻倍；`know` 只要求已读。
- **入口只放在设置或侧边栏。** 主管理后台要一眼看到，放在聊天列表顶部才够近。
- **条目照旧发一份进 Matrix 房间，让别的客户端也能看。** 一份事实变两份，读和答都要对账；别的 Matrix 客户端本来也答不了卡片。
- **第一版就做稍后提醒、搜索、筛选。** 条目类型和量都还没跑过，先只留两段分组。
- **管理员条目每人一份拷贝。** 一份条目、每人一份已读/回答状态才不会漂，回答后别人的那份直接变“已由某人处理”。

## 代价与风险

- **面板变成新的噪音源。** 插件通知、来信全进来以后，面板可能一天几十条。缓解：`info` 不推送；插件每分钟 10 条、每天 200 条
  的上限照旧（ADR-0047）；`know` 30 天清掉。
- **角标压力。** 未读消息加待处理可能让角标一直很大；只有真的处理或读才减，而不是打开 App。
- **私聊退休会断旧链接。** 旧信里的找回密码地址不受影响（在消息正文里），但“去私聊找那封信”的习惯要换。
- **核心多存一份正文。** 来信正文和附件在 `core/inbox` 里多留一阵（最长 30 天），磁盘占用和 Mailpit 的 7 天叠加。
- **卡片的两个投影可能被看成两件事。** 靠同一 ID、同一回写和“处理一边两边一起变”的测试兜住。
- **家人多、管理员多时**，管理员条目对每个管理员都推一条，同一件事会在几台设备上各弹一次；这是“谁先答算谁的”的代价（ADR-0088）。
- **App 和核心分开升级时**：旧 App 见到新类型只显示角标不显示条目；新 App 对上旧核心时列表 404，照实显示加载失败。
- **worker 桥是新增的单点。** 它挂了就没有事件；靠登记表对账补 `WorkerStuck`，桥本身要做成能被 supervisor 拉起的长驻进程。

## 待验证
- 390×844 下入口行、带两个按钮的卡片、长标题两行；大字号不截断。
- 家人 A 的条目不出现在家人 B 的面板；管理员条目在 A 答后 B 显示“已由 A 处理”且通知撤下。
- 通知上答没有表单的卡片，答案执行、条目进已处理、另一台设备的通知消失。
- worker 卡住时写一条 `WorkerNeeds`，在通知上允许后条目进已处理、worker 继续跑；worker 随后退出，`WorkerDone` 只留最新一条。
- worker 桥被杀、宿主重启后，登记表对账能给失联的 worker 补 `WorkerStuck`。
- 面板最下面“旧的通知记录”能打开旧私聊，隐藏后不再出现在聊天列表。
- 来信 30 天、已处理 24 小时的清理；核心重启后条目还在、长轮询接着走。
- 端到端：装插件、收一封应用来信、触发一条 `critical` 通知、另一台服务器发配对请求，四件事都只进面板、私聊为空；再让一个 worker 卡住，条目出现、在通知上允许后它继续跑，随后完成只剩一条“做完了”。

## 查证来源
- herdr CLI：https://herdr.dev/docs/cli-reference （`agent wait --until done,blocked`、`pane report-agent --state blocked|done`、`agent send-keys`、`pane read`）。
- tern CLI：https://tern.build/docs/cli （`tern wait --for done,needs`、`tern status -json`、`tern approve allow|always|deny`）。
- Alma `Home`：https://docs.alma.team/workspace/home （“What is waiting on you arrives in the thread… The decision is the same one”）。
- Alma `Tasks`：https://docs.alma.team/workspace/tasks （Needs you / Active / Backlog、Snooze 一小时的选项、请求留在任务里）。
- Linear Inbox：https://linear.app/docs/inbox （Priority 与其余分开、已读/未读、Done、2000 条上限）。
- GitHub 通知：https://docs.github.com/en/subscriptions-and-notifications/concepts/about-notifications
  （Done、已读/未读、按原因筛选、3 个月保留）；inbox 的操作页 https://docs.github.com/en/subscriptions-and-notifications/how-tos/viewing-and-triaging-notifications/managing-notifications-from-your-inbox 。
- Apple HIG `Notifications`：https://developer.apple.com/design/human-interface-guidelines/notifications （最多四个动作、角标只表示未读、
  同一件事不要重复通知）；`Managing notifications`：https://developer.apple.com/design/human-interface-guidelines/managing-notifications
  （Passive/Active/Time Sensitive 的语义）。
- 代码位置按正文里的路径和行号，取自分支切出时的 main `33f3fb54`。

## 关联

ADR-0005（厚核心）、ADR-0015（小组件）、ADR-0016（确认卡片和列表）、ADR-0003/0101（推送和通知上的动作）、ADR-0038（问题卡片）、
ADR-0118（推送带正文、房间不加密）、
ADR-0043（语言）、ADR-0045/0088（家人、管理员）、ADR-0047（插件通知）、ADR-0050（配对请求）、ADR-0056（内部邮箱）、
ADR-0095（没有底栏）、ADR-0102（App 锁）、ADR-0026/0029（外观）。
