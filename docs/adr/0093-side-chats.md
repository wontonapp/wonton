# ADR-0093 旁聊：主对话里的 Matrix 话题，收成卡片，结束时主对话里一行结果

- 状态：提议。产品方向由主人定（2026-11-12）：像 Meta Muse 一样一个 AI 一个主对话，旁聊是新的上下文、同一个人格；在主对话里收成卡片；
  旁聊有结果时主对话里一行，AI 下一轮知道；“新建聊天 → 已有的 AI”开新旁聊，不另建私聊。
- 部分取代 [ADR-0058](0058-agent-side-sessions.md)：“决策”开头“核心和协议不改”一句、话题名字的取法、“主对话里一行提示”的格式。

## 背景

和 AI 的私聊只有一条时间线。主人想在不打断主对话的情况下另开一段（Muse 的 side chat）。两类 Agent 在服务器这一侧已经能分会话：

- Hermes 的 Matrix 适配器一个话题（`m.thread`）一个会话，人设和记忆在同一个 profile 下共用；默认不处理 `m.notice`（`process_notices` 为假）。
- ACP Agent 经桥按话题根分会话（ADR-0058），桥只处理 `m.text`；每轮结束在主对话里发一行纯文字提示，不带话题根，App 点了跳不进去。

缺的在 App：主时间线把话题回复平铺显示，发不出话题消息；“新建聊天”遇到已有私聊直接返回。Hermes 也不会让主对话知道旁聊的结果。

## 决策

### 1. Matrix 上的表示

旁聊就是 AI 私聊房间里的一个 Matrix 话题。任何话题都按旁聊显示，不管是 App、Element 还是 Agent（ADR-0059 的任务话题）开的。
下面三个约定写进 wonton-docs 的插件规范，Agent 一侧照它实现，App 一侧照它解读。

**根消息。** App 开的旁聊，根是主人发的一条 `m.notice`：

```json
{ "msgtype": "m.notice", "body": "整理一下手机里的照片",
  "app.wonton.side_chat": { "v": 1, "title": "整理一下手机里的照片" } }
```

- 用 `m.notice` 是因为 Hermes 和桥都不回应它，根不会在主会话里引出一轮。主人第一句话发在话题里。
- **标题**只有一条规则，App、桥、Hermes 一层都用它：根消息有 `app.wonton.side_chat.title` 就用它；没有（别的客户端开的话题）就取根消息
  `body` 第一行的前 20 个字符。App 开旁聊时 `title` 取主人第一句话第一行的前 20 个字符，`body` 等于 `title`。

**结果行。** 旁聊的一轮结束、或者旁聊结束时，Agent 一侧在主对话（不挂 `m.thread`）里发：

```json
{ "msgtype": "m.notice", "body": "旁聊「整理一下手机里的照片」：已完成",
  "m.relates_to": { "m.in_reply_to": { "event_id": "$root" } },
  "app.wonton.side_chat_result": { "v": 1, "root": "$root", "title": "整理一下手机里的照片", "state": "done" } }
```

- `state`：`done`（这一轮答完了）、`failed`（这一轮失败或被打断）、`ended`（旁聊关了：桥是空闲 30 分钟或 `/wonton end`，Hermes 是这个话题的会话被重置）。
  旁聊结束后主人再发消息，Agent 一侧照常接着做，下一轮结束再发结果行。
- 话题根放在结构化字段 `root` 里，App 按它跳，不解析 `body`。`m.in_reply_to` 指向根，让 Element 这类客户端也能点回去。
- `body` 只给不认识这个字段的客户端看，由发送方按主人的语言写（ADR-0043）。`m.notice` 不推送、不算未读。

**声明支持。** 实现了本篇全部约定（话题分会话、发结果行、下一轮让主会话知道）的 Agent，在它的每个私聊里写状态事件
`app.wonton.bot.side_chats`，`state_key` 是 Bot 自己的 Matrix ID，内容 `{ "v": 1 }`。信任规则同 ADR-0054：只认 `state_key` 和发送者相同、
发送者是房间成员，且是核心认得的 Bot（ADR-0082）。

### 2. AI 怎么知道旁聊的结果

结果行是给人看的；发它的 Agent 自己会忽略自己的 `m.notice`。所以“主会话下一轮知道”由发结果行的同一层负责，不经 Matrix：

- **桥**：不变，ADR-0058 的工作板和每轮开头的“其他会话的动态”。
- **Hermes**：主会话（会话键里没有 `thread_id` 的那个）每一轮开始前，把上一轮之后有新结果的旁聊列进这一轮的上下文：标题、状态、
  AI 在那个旁聊里最后一次回复的第一段（最多 200 个字符）。格式照桥的动态写。

### 3. Hermes 的这一层放在 Hermes 插件里

放在 Hermes 插件仓库（ADR-0079 §6），做成 Hermes 上游的 Python 插件（`ctx.register_hook`），随插件挂进上游原版镜像，不改上游源码：

- `post_llm_call`：会话属于某个话题时，发结果行（`done`）；失败、被打断时发 `failed`。会话和房间、话题根的对应从 `agent:start` 事件里记下。
- 话题的会话被重置（`on_session_reset`，或网关的 `session:end`）时发 `ended`。
- `pre_llm_call`：主会话按第 2 节注入。
- 连上后在每个私聊写第 1 节的声明。
- 发消息和写状态事件经 Hermes 进程里已登录、能加密的 Matrix 客户端，不另起账号。

核心（服务器和客户端）不知道哪个 Agent 怎么实现，只认第 1 节的约定。按 [ADR-0015](0015-small-components-standard-interfaces.md)，
提供方是桥和 Hermes 插件，消费方是客户端核心，合同是 Matrix 事件。

### 4. 客户端核心（`wonton-client`、`wonton-matrix`）

- **主时间线**用 `TimelineFocus::Live { hide_threaded_events: true }`：话题回复不再平铺。有回复的普通消息带一张卡片；带 `app.wonton.side_chat`
  的根只显示成卡片。卡片的回复数取 SDK 的 `ThreadSummary`。
- **结果行**变成时间线里单独的一种条目。同一个旁聊在主时间线里只留最新一条结果行，更早的不显示。
- **旁聊时间线**用 `TimelineFocus::Thread { root_event_id }`，和主时间线共用同一套条目转换、发送队列、位置（ADR-0071）、草稿和回复（ADR-0073）；
  这些都按 `(room_id, thread_root)` 分开记。话题里发文字和图片都带 `m.thread` 关系，SDK 的话题发送不够时在 `wonton-matrix` 里补图片。
  根是带 `app.wonton.side_chat` 的 `m.notice` 时不作为条目，标题和状态放在订阅的头部；根是普通消息时作为第一条显示。
- **开旁聊**：先发根消息，等服务器回事件 ID，再把第一句话放进这个话题的发送队列（本地回显照常），返回根的事件 ID。根没发出去时返回错误，
  不留下空旁聊。
- **未读**：旁聊里读到哪用 Matrix 的话题已读回执（`ReceiptThread::Thread(root)`），主时间线改发 `ReceiptThread::Main`。
  每个旁聊的未读数由核心按话题回执算；聊天列表里这个房间的未读数是主对话加所有旁聊。
- **卡片的状态**由核心合成：

  | 状态 | 什么时候 |
  | --- | --- |
  | `Open` | 还没有结果行 |
  | `Done`、`Failed`、`Ended` | 最新一条结果行的 `state` |
  | `ResultMissing` | 房间里有支持声明；Bot 在旁聊里的最后一条消息（不算 ADR-0082 的状态消息）之后 2 分钟没有对应的结果行，而且这个旁聊里没有还在 `working`、`waiting` 的 ADR-0082 状态 |
  | `Untracked` | 房间里没有支持声明：不会有结果行 |
  | `Other` | 结果行的 `state` 不认识 |

- **新建聊天**：选的人已有私聊、且私聊里有支持声明时，开新旁聊；没有私聊时照旧建私聊。已有私聊但没有支持声明时打开主对话，壳提示一句
  “这个 AI 不支持旁聊”。不为“新的对话”再建一个私聊房间。
- **聊天列表**一个 AI 一行。最后一条预览是主时间线的最后一条；那是根或结果行时，预览给旁聊的标题和状态。
- 确认卡片的来源（`ConfirmationOrigin.thread_id`）和推送都带话题根：点了都进那个旁聊。

### 5. 协议（`wonton-protocol`）

`Action` 是 UniFFI 枚举，`uniffi_core` 0.32.2 给每个类型的元数据一个 32 KiB 的缓冲区，变体名、字段、类型和**文档注释**都算在内，
超了编译失败。#287（链接预览，加 `DismissLinkPreview { room_id }`）合并后 `Action` 已经顶到上限，那个变体的文档只能写一行；
之前也有一次为此把动作改成 `ClientCore` 的直调方法（`2d48363`）。所以本篇不在 `Action` 上净增任何东西，而是在同一个 PR 里
先把按时间线的操作整组挪进嵌套枚举，嵌套枚举有自己的 32 KiB：

- 新 `TimelineTarget { room_id: String, thread_root: Option<String> }`，`thread_root` 为空是主时间线。
- 新 `TimelineAction`，由 `Action` 里这 11 个变体原样搬过去、`room_id` 换成 `target: TimelineTarget`：`SendText`、`SendImage`、
  `ForwardMessage`（目标仍是 `to_room_id` 的主时间线）、`RedactMessage`、`RetrySend`、`CancelSend`、`PaginateBackwards`、
  `SetTimelineScroll`、`MarkRoomRead`（改名 `MarkRead`）、`SetDraft`、`DismissLinkPreview`（旁聊的输入框也有链接预览）。
  搬过去的变体的文档可以写全，#287 只能留一行的那条补回来。
- `Action` 删掉这 11 个，加一个 `Timeline { action: TimelineAction }`，不带文档注释，说明写在 `TimelineAction` 上（结构体变体：
  `Action` 用 serde 的内部标签 `type`，newtype 变体里再套一个带 `type` 的枚举会撞键）。
- 实测（2026-11-12，#287 的分支 `feat/link-preview-client` `d805ccb`，`cargo check -p wonton-protocol --features uniffi`）：
  原样只放得下一个单字母的空变体；照上面搬完后 `Action` 空出约 2.9 KB，`TimelineAction` 编译通过。
- `ClientCore::subscribe_timeline` 和 `draft` 的参数从 `room_id` 换成 `TimelineTarget`；新增 `ClientCore::start_side_chat(room_id, body)`，
  返回根的事件 ID（第 4 节“开旁聊”）。它要等服务器回 ID，是请求–应答，和 `subscribe_timeline` 一样做成方法，不占 `Action`。
- 以后 `Action` 再要加一组相关操作，同样先搬成一个带嵌套枚举的变体，不逐个加；`Action` 空出来的地方留给这类搬家，不拿来加单个变体。

ViewModel：

- `SideChatCard { root_event_id, title, reply_count: u64, unread_count: u64, state: SideChatState }`，`SideChatState` 是第 4 节的表。
- `TimelineMessage.side_chat: Option<SideChatCard>`（有回复的普通消息）；`TimelineContent::SideChat { card }`（带标记的根）；
  `TimelineContent::SideChatResult { root_event_id, title, state }`（结果行）；`TimelineUpdate.side_chat: Option<SideChatCard>`（旁聊订阅的头部，主时间线为空）。
- `RoomRow.side_chats: bool`（房间里有支持声明）；`MessageSummary::SideChat { title, state }`；
  `ChatContact.side_chat_room_id: Option<String>`（选这个人时在哪个房间开新旁聊；为空就 `StartDirectChat`）；`PushNotification.thread_root: Option<String>`。
- 三个 Matrix 事件内容（根、结果行、支持声明）的 serde 类型也放在 `wonton-protocol`，桥和客户端核心共用。契约测试：一组 JSON 样例放进
  这个 crate，桥发出的事件和客户端核心读的都过这组样例；Hermes 插件的测试用 wonton-docs 里同样的样例。

### 6. 壳

只画 ViewModel、把操作发回核心（ADR-0005）。样式走 ADR-0026、ADR-0029。

- 主对话：卡片画成一行“旁聊 · 3 条”加标题和未读，结果行画成一行小字“旁聊「标题」：已完成”，点了都进旁聊。
- 支持旁聊的房间，聊天页顶栏一个开旁聊的图标按钮；进去是空的旁聊页，发第一句时调 `start_side_chat`。
- 旁聊页复用聊天页和输入框，订阅换成 `TimelineTarget`，顶上是标题和状态；`Untracked` 时一句轻提示“结果不会带回主对话”。
- 文字按 `SideChatState`、数目和错误类别写在 String Catalog 和 `strings.xml`，四种语言齐全，“条”用复数资源（ADR-0043）。术语见 `docs/glossary.md`。

### 7. 出错时

- **AI 不支持旁聊**：不出开旁聊的按钮；“新建聊天”选它时打开主对话并提示一句（第 4 节）。别的客户端开的话题照样显示成卡片，状态 `Untracked`。
- **开旁聊失败**：`start_side_chat` 按类别返回（传输不可用、没就绪、输入非法、服务器出错），旁聊页留着主人写的字，配重试，详情默认收起。
  根发出去了、第一句没发出去，就是旁聊里一条发送失败的消息，照常重发或取消。
- **结果行一直没来**：卡片显示 `ResultMissing`（“没收到结果”），不当成完成。之后结果行到了就按它显示。
- 结果行或根的 `v` 不是 `1`、字段不全：当普通的 `m.notice` 显示，不猜。

### 8. 旧版本 App 和别的客户端

- 旧版 App 没有话题界面：旁聊的消息照旧平铺在主对话里，根和结果行是两行提示文字，看得懂，不会显示错。旁聊发出的确认卡片照旧显示在主对话里（`ConfirmationOrigin` 的约定）。
- Element 这类客户端用自己的话题界面，结果行是一条回复根的提示，点得回去。
- 新版 App 遇到不认识的结果状态落到 `Other`，显示 `body`。

## 考虑过的做法

Hermes 的结果放在哪：

- **服务器核心**：核心不在主人和 AI 的加密私聊里，读不到话题内容；要写结果就得拿一个账号进每个私聊，还得知道哪些 Agent 需要它补，
  正是 ADR-0015 不许的按 Agent 特殊处理；也没有接口把结果送进 Hermes 的下一轮。桥已经做了 ACP 那一半，核心再做一份是重复。
- **App 发结果行**：以主人身份发的消息会被 AI 当成新的一句来回答；App 不知道旁聊的结果，要总结就得自己调模型；几台设备会各发一条，设备离线就没有。
- **插件里另起一个 Matrix 账号的小程序**：要进私聊才能解密，私聊就不再是一对一；也够不着 Hermes 的下一轮。
- **改 Hermes 上游或改走 ACP**：上游已有插件钩子，不用改源码；不走 ACP 的理由见 ADR-0079 §6。

其他：

- **“新建聊天”再建一个私聊房间**（应急做法）：一个 AI 在列表里多行，不是 Muse 的形态，主人否了。
- **聊天列表里每个旁聊一行**：主人选了卡片。
- **结果行编辑成同一条**：编辑不改位置，新结果留在上面，主人看不到；改成发新的一条、核心只显示最新的。
- **结果行挂 `m.reference`**：ruma 的 `m.room.message` 不认这个关系，SDK 会当成未知关系；`m.in_reply_to` 是标准做法，别的客户端也认。
- **在 `SendText`、`SendImage` 上加 `thread_root`、或加一个只装旁聊操作的变体**：#287 之后 `Action` 一个字节都加不了；
  草稿、翻页、位置、已读、重发、链接预览也都要分话题，按 `TimelineTarget` 一次说清，旁聊的条目 ID 也和主时间线分开。
- **旁聊的操作全做成 `ClientCore` 直调方法**：能绕开上限，但壳就有两套发操作的路；只有要回值的 `start_side_chat` 做成方法。
- **App 内的旁聊列表**：卡片已在主对话里；旧旁聊多到找不着时再定。

## 代价与待验证项

- `Action` 搬家是破坏性接口改动：两个壳所有发消息、翻页、草稿、链接预览的调用点都要改（机械）；和同时在改这些变体的 PR 会冲突。
  接口 PR 基于含 #287 的 `main`。
- 每轮一条结果行，主对话里的行数随旁聊轮数增长；同一旁聊只显示最新一条，`ResultMissing` 的 2 分钟按实测调。
- 待验证：
  - Tuwunel v1.9.4 存不存话题已读回执（MSC3771），`/sync` 里给不给话题的未读；不给就由核心在本机按回执算。
  - matrix-sdk-ui 0.19.1 的 `hide_threaded_events` 和 `Thread` 焦点在加密房间里、翻历史时的表现；`ThreadSummary` 的回复数在没翻到的旧根上准不准。
  - Hermes 插件拿得到网关里已登录的 Matrix 客户端，能发带自定义字段的消息和状态事件；`post_llm_call` 能对上会话和话题根；
    话题会话的重置有没有钩子触发。拿不到就改本篇，不另找路子。
  - Hermes 对话题的处理只读过代码和文档：用 Element 往 Hermes 私聊的话题里发一句，确认它开新会话、回答发在同一个话题里。

## 关联

ADR-0005（厚核心）、ADR-0015（标准接口）、ADR-0026、ADR-0029（设计系统）、ADR-0035（ACP 桥）、ADR-0043（多语言）、ADR-0054（Bot 写状态事件）、
ADR-0058（话题会话，本篇部分取代）、ADR-0059（任务话题）、ADR-0071（时间线位置）、ADR-0073（回复）、ADR-0079（Hermes 单独仓库）、ADR-0082（Agent 进度）。
