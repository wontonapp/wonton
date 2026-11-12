# ADR-0101 通知：回复与按钮、解密正文、未读角标、实时活动

- 状态：已接受（2026-11-13）。方向由主人定（11-13：先做 Android 四项，iOS 的推送链路在 Apple 付费开发者账号就绪后做，
  账号就绪前 iOS 只有"App 在前台时"和本地通知；锁屏默认显示消息内容，设置里按类别关；只有不带表单的确认卡片才在通知上
  直接给"允许/拒绝"，带表单的点通知进 App；灵动岛和实时活动只给"等主人确认"的长任务）。细化
  [ADR-0003](0003-matrix-and-encrypted-push.md)，不推翻它。
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（加密推送、推送代理、event_id_only）、
  [ADR-0016](0016-mcp-bus-and-confirmations.md)（确认卡片、卡片推送）、[ADR-0082](0082-agent-run-status.md)（Agent 运行状态）、
  [ADR-0093](0093-side-chats.md)（旁聊，推送带 `thread_root`）、[ADR-0097](0097-share-in.md)（iOS App Group 数据目录，PR #402）、
  [ADR-0102](0102-app-lock.md)（App 锁，锁着不露内容）、[ADR-0043](0043-app-languages.md)、
  [ADR-0015](0015-small-components-standard-interfaces.md)

## 背景

现状（实测代码，main `7e77084`）：

- **推送链路两端都只做"弹一条通知"**。Android 的 `PushNotifications.kt` 有合并（:24、:61-62）、按类别分渠道（:33-35，渠道 id
  在 `NotificationSettingsScreen.kt:51-55`）、锁屏隐藏（:82 `setPublicVersion`）、卡片到期自动撤下（:91-94、:99-106），
  但没有任何 action 或 `RemoteInput`。iOS 一条推送链路都没有：`NotificationSettingsView.swift:12-18` 只跳系统设置，:21 的文案是
  "这个版本的云吞还收不到推送"；`apps/ios/` 搜不到 `UNUserNotificationCenter`，project.yml 也没有推送扩展 target。
- **确认卡片已经会推送**：核心推送代理把 `DeviceConfirmationPush`（卡片 ID、标题、到期时间、来源会话
  `crates/wonton-protocol/src/confirmation_origin.rs:67-74`）加密直推给接收人的已配对设备
  （`crates/wonton-core/src/push/mod.rs:228-229`，ADR-0016 第 2 步）。回答走 `POST /_wonton/confirmations/{id}`
  （`crates/wonton-protocol/src/confirmation.rs:21-23`），核心侧只认隧道的已配对设备（`TunnelPeer` + `not_their_card`，
  `crates/wonton-core/src/confirmations.rs:135-153`）。**推送里没有按钮、没有选项**；卡片模型本身有 `choices: Vec<CardChoice>`（:42）
  和表单字段（`secret_inputs` :51-57、`settings` :54）；推送结构里还没有 choices，由下面决策 1 补上。
- **未读数已经有了**：`RoomInfo::unread_count` 来自 Matrix 的 `room.num_unread_messages()`
  （`crates/wonton-matrix/src/rooms.rs:594`，字段声明 :113），进 `RoomRow.unread_count`
  （`crates/wonton-protocol/src/chat.rs:323`）。已读由打开房间时发回执清（`crates/wonton-matrix/src/chat_room.rs:535-547`，
  调用方 `crates/wonton-client/src/chat/ops.rs:16`）。没有总计，也没有任何地方把它交给系统角标。
- **Agent 进度已经有了**：`AgentRunState` 的 `Working`/`Waiting`/`Done`/`Failed`/`Uncertain`
  （`crates/wonton-protocol/src/agent_run.rs:12-22`，MSC4564 四值 + 客户端合成的 `Uncertain`，ADR-0082 第 4 节），
  从 Bot 的状态消息解读。用于聊天里的状态行，没有接实时活动。
- **跨进程的底子已经有一半**：`wonton-matrix/src/session.rs:454-456` 已开 `CrossProcessLockConfig::multi_process`，租约 500 ms
  （matrix-sdk-common-0.19.1 的 `cross_process_lock.rs:286`，`LEASE_DURATION_MS = 500`）。持有者名在
  `crates/wonton-matrix/src/store_holder.rs:73-79` 按**进程号**取（`wonton-{pid}-{n}`），同存储上两个进程会各自拿到不同的名字，
  新 Client 要等旧租约过期；它注释里写明名字相同时"另一个写回存储的明文、导入的房间密钥都被当成自己写的……解不开的消息就一直
  解不开（#804）"。
- **iOS 的共享容器随 PR #402 落地**：数据目录搬进 App Group（[ADR-0097](0097-share-in.md)，`apps/ios/Shared/AppGroup.swift`）。
  本 ADR 排在它后面，复用 `AppGroup.dataDirectory()`，不再自己定路径。

已定的两条地基（沿用，不改）：

- 推送里**没有正文**，只有房间、事件、发送者（[ADR-0003](0003-matrix-and-encrypted-push.md)）。解密只在本机做，中继只看到密文。
- 推送是**唤醒信号**，内容由核心在本机取回并解密（`resolve_push`，`crates/wonton-client/src/client/push_route.rs:42`，注释
  :30-33；取不到时返回 `NotReady`/`TransportUnavailable`，:220、:223）。通知上能显示什么、能点什么都由核心给的
  `PushNotification` 决定，壳只渲染并调用平台能力（AGENTS.md"厚核心薄壳"）。

## 决策

### 1. 通知上的动作：回复和按钮

**能答什么、不能答什么。** 只给两类动作：

- 消息通知：`RemoteInput`（Android）/ `UNTextInputNotificationAction`（iOS）的"回复"。
- 确认卡片通知：**没有表单字段的卡片**（没有 `secret_inputs` 也没有 `settings`，`confirmation.rs:51-57`）给"允许""拒绝"两个按钮
  （`UNNotificationAction` / `NotificationCompat.Action`）。要填密钥（[ADR-0064](0064-input-secrets.md)）或设置（ADR-0077）的卡片不给按钮，点通知进 App：
  表单没法塞进推送。

**回答怎么执行。** 不新开旁路，一律回到核心现有的回答/发送路径：

| 平台 | 入口 | 进程 |
| --- | --- | --- |
| Android | `NotificationCompat.Action` 的 `PendingIntent.getBroadcast` → `BroadcastReceiver`（同一个 `WontonPushService` 所在进程） | 推送服务唤起的进程里就有核心，直接调 |
| iOS | `UNUserNotificationCenterDelegate.userNotificationCenter(_:didReceive:withCompletionHandler:)` | **系统在后台把 App 拉起来**（见"查证来源"） |

- 核心加两个 `ClientCore` 方法，不加 `Action` 变体（`Action` 的 uniffi 元数据已贴着上限，加变体编不过：
  `crates/wonton-client/src/client/tabs.rs:100-101`）：
  - `answer_push_confirmation(server_id, card_id, answer, choice_id)`：包一层现有的 `answer_confirmation`
    （`crates/wonton-client/src/chat/confirmation.rs:577`）；
  - `reply_to_push(server_id, room_id, thread_root, reply_to_event, text)`：走现有的待发消息（ADR-0094），
    故意不带附件（通知里只能打字）。
- 两个方法都要能在**冷启动**的进程里调：核心一创建就先等隧道和会话恢复（`resolve_push` 已经是这个行为），拿不到就返回
  `NotReady`/`TransportUnavailable`，**不排队假装成功**。
- iOS 的通知类别只能在 App 里注册，**装好后至少启动过一次**才有点按钮；没启动过就只有普通通知。
- 在等待被回答的卡片上，推送要多带按钮：`DeviceConfirmationPush` 加 `choices: [{id, label}]`（取前 3 个，APNs 载荷上限 4 KB）。
  卡片本身没有自定义按钮时，核心补一对 `approve`/`reject`，和 App 里画的是同一组。

**答不上时。** 失败必须看得见，三种都做：

1. 动作处理中先给回答本身一个进度：Android 在同一个通知上换成"正在发送…"，iOS 用 `UNNotificationContent` 的 subtitle 或一条
   本地通知，最多等 30 秒（后台动作的可用时间是秒级到 30 秒量级，要真机量）。
2. 失败就留一条本地通知"没答上：<原因>，点开重试"，原来的卡片通知不撤，卡片在核心那边照常等到期。
3. 失败也写进 ViewModel（`AnswerProgress::Failed(ChatProblem)` 已有，"没送到或者核心出错：可以再点"，
   `crates/wonton-client/src/chat/confirmation.rs:37-43`、:61），下次打开 App 能看到同一句。

**回复的乐观显示。** 先在本机把这条回复画进时间线（ADR-0094 的待发状态），发送结果回填；失败标红可重试，不静默丢。
同一条消息被两台设备都回了时，按 Matrix 事件去重，时间线只留两条不同 ID 的回复。

### 2. 通知里的解密正文（iOS NSE）

Android 今天就是对的：推送在服务进程里被核心解密（`WontonPushService.onMessage` → `resolve_push`），拿不到时
`PushContent::Unavailable` 带原因，壳写"取不到内容"（`PushNotifications.kt:114-117`，`R.string.push_unavailable`）。
要补的只是把原因按类别分细。

iOS 新建 **Notification Service Extension**，在其中解 Megolm 密文。这是 ADR-0003 待验证项
（"iOS 通知扩展的内存限制下解密的可行性（优先用系统自带的加密能力，不在扩展里跑完整核心）"，ADR-0003:46）的答案，不推翻它：

- Web Push（RFC 8291）那层确实不用核心，系统 `CryptoKit` 的 P-256 + AES-GCM 就能解，和 ADR-0003 写的一致；
- 但**正文是 Megolm**，系统没有对应能力，解密只能在 Rust 核心一侧做。所以"不跑完整核心"落地成：NSE 链接同一个核心库，
  只走推送解密这一条（`NotificationClient` + `EncryptionSyncService`），不起 `ClientCore`、不起插件页/文件那些子系统。

具体照 Element X iOS 的现成做法：

- **范围收到最小**：`setupEncryption: false`、不发现 sliding sync、只恢复被推的那个房间、请求超时 15 秒、
  `.systemIsMemoryConstrained()` 打开（Element X 同款）。
- **内存**：NSE 的上限约 24 MB 是社区实测数据（Apple 未公开数字），所以：NSE 里不下载图片、不做缩略图、不用 `UIImage`；
  Rust 侧要专门的编译配置（`opt-level = "z"`、不开 backtrace 符号链）；NSE 目标里加一块打印 `phys_footprint` 的探针
  （Element X 的 `ExtensionLogger.logMemory`），真机测到峰值并留至少 30% 余量才放行。
- **跨进程存储**：两个进程同时开同一个 SQLite 存储。用已有的 `multi_process` 锁（session.rs:454-456），但**持有者名必须两进程
  一致**——改成按存储固定的名字（iOS 上用常量 `ios`），不能再让 `store_holder.rs:73-79` 按进程号取名。租约 500 ms，App 被挂起后
  扩展最多等一个租约就能接手；等一个租约重试一次后仍拿不到就放弃解密、走"取不到"的显示（`try_lock_store_once` 的语义撑着：
  `vendor/matrix-sdk-ui/src/encryption_sync_service.rs:138`、:159，拿不到锁不跑加密同步循环）。
- **解密**：`NotificationClient` 用 `NotificationProcessSetup::MultipleProcesses`
  （`vendor/matrix-sdk-ui/src/notification_client.rs:72`），和 `resolve_push` 一样经隧道取事件；缺房间密钥时由一个
  `EncryptionSyncService` 拉 to-device 事件（模块注释就是为 NSE 写的），并设解密期限。
- **隧道所有权**：一台手机只能有一条隧道（ADR-0072）。App 被挂起时 NSE 要能从共享钥匙串拿设备密钥自己建一条，用完释放。
  这里要**在两种情形下都验过**才能上线：App 进程还在（被挂起但没杀）和 App 已被杀。做法沿用 ADR-0072 的"谁持有隧道谁写连接信息"，
  NSE 借走后必须写回，App 回前台发现连接信息换了就重连。
- **共享的东西**：会话凭据放共享钥匙串访问组，数据目录用 #402 的 `AppGroup`（`group.<bundle ID>` 容器下的 `wonton/`）。
- **设备重启后还没解锁**时组容器写不进去，申请不了会话（Element X 专门处理了这个：`BootDetectionManager`）。这种情况发一条
  "有 1 条以上新消息，解锁后点开查看"的普通通知，不假装是解密成功的正文。
- **取不到时显示什么**（三种原因分开写，不许出现空正文）：还没收到房间密钥（"内容暂时取不到，可能刚加入"）、
  连不上家里（"连不上家里的服务器"）、设备锁着（"解锁后可看"）。详情收进折叠区。

**NSE 的时间预算**：`didReceive` 不超过 30 秒（Apple 原文 "no more than 30 seconds"），超时前 `serviceExtensionTimeWillExpire()`
给最后一次机会交回兜底内容；交回的就是上面那三种说明，不是原文。

**和 #402 的关系**：#402 决定"扩展不链接核心"（分享扩展只搬文件），理由是不确定内存、以及两个进程抢设备密钥/存储未验证。
NSE 和分享扩展不同：分享扩展是前台、可等；NSE 只有 30 秒且必须解密。所以这两个未验证项是 NSE 的**准入条件**，不是附注。

### 3. 未读角标

- **核心算，壳只写系统角标**。核心加 `unread_total()`（所有房间 `unread_count` 之和，邀请按 1 计）和一个订阅；
  `RoomRow.unread_count` 还是唯一事实源，不新造计数。
- 清零只发生在**真的读过**：房间被打开、回执发出去（`chat_room.rs:535-547`），角标跟着收；App 只是被打开不归零。总数为 0 时角标写 0。
- iOS：NSE 里写 `content.badge`（Element X 同期做法是每条 +1，但我们是核心给的绝对数，避免多设备各自加法漂移），
  App 回前台和清零时用 `setBadgeCount`。
- Android：`NotificationChannel.setShowBadge`（已默认开）；分组摘要通知上用 `setNumber(unread_total)`。
  启动器差异如实写进设置页：多数启动器只画圆点、或者按"有几条通知"算，`setNumber` 的绝对数不在所有启动器上生效（待真机验证到
  具体机型）。
- 局限：另一台设备读掉之后，本机 iOS 角标要等下一次推送或回前台才纠正；Android 圆点在通知被清掉后自行消失。

### 4. 灵动岛 / 实时活动（等主人确认的 Agent 任务）

- 数据源已经够：`AgentRunState` 的 `Waiting`（等主人确认，`agent_run.rs:14-22`），加上会话名和开始时间。
- 核心加 `runs_waiting()` 视图，只给 `Waiting` 的，最多 3 条。
- **什么时候开**：
  - iOS `Activity.request(attributes:content:pushType: .token)`，在 App 运行并看到某轮进入 `Waiting` 时开；主人答完
    （`Done`/`Failed`）结束（`end`）；Live Activity 最长活跃 8 小时，超过系统自动结束（Apple 文档）。
  - 推送更新：把 `activity.pushTokenUpdates` 的 token 存下来，经隧道登记给核心，核心再经中继发 ActivityKit 推送。
    预算：`apns-priority: 10` 计入每小时预算，`5` 不计（Apple 原文），所以默认用 5，只有"主人在等的卡片有新动静"用 10；
    需要更密的更新加 `NSSupportsLiveActivitiesFrequentUpdates` 并尊重 `frequentPushesEnabled`。
  - Android：`Waiting` 任务的常驻通知用 `Notification.ProgressStyle`（API 36）显示段/点；Android 16 起用
    `setRequestPromotedOngoing(true)` + ongoing + 清单里的 `POST_PROMOTED_NOTIFICATIONS`（普通权限）申请成为 Live Update。
    Google 把"聊天消息"列为**不适合** Live Update，所以只给等主人确认的任务，普通进度仍是普通通知。
- 灵动岛只在带灵动岛的机型上有；老机型就是锁屏实时活动，再老（iOS 16.1 以下）没有。

### 5. 隐私

- 锁屏内容单独一个开关，按类别（消息 / 确认 / 插件通知），默认**显示**内容；关掉的类别只写"新消息"。
  Apple 侧用 `UNNotificationCategory.hiddenPreviewsBodyPlaceholder`，Android 侧用已有的 `setPublicVersion` 这一套
  （`PushNotifications.kt:82`）。
- 和系统"显示预览"叠加：系统"永不"优先于我们的开关，开关不能绕过它。
- 角标数字本身不算敏感内容，锁屏开着。
- 回复的内容会经过隧道，不出家门；确认卡片的选项在推送明文里（`approve`/`reject` 的标签是核心写死的词，不是用户数据）。
- App 锁锁定期间到达的通知不露内容，见 [ADR-0102](0102-app-lock.md)。

### 6. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `push` 载荷扩展（`choices`、回复目标）和 `ClientCore` 的四个新方法；契约测试写在
  `crates/wonton-protocol/src/push.rs` 的测试里（按钮往返、按类别关内容时字段仍完整、超 3 个选项只取前 3）。
- **提供方**：iOS 的 NSE 和 App 的通知 delegate；Android 的 `WontonPushService` 和动作 `BroadcastReceiver`。
- **消费方**：核心——动作最终都落到 `answer_confirmation` / 待发消息。
- **换提供方不改消费方**：换成第三方客户端时，它只要实现"解开推送 → 调 `resolve_push` → 渲染 `PushNotification`（含 `choices`）→
  回答走 `answer_path`"就够，核心不认平台。

## 实施顺序

1. **Android 四项**今天都能落地：通知动作（回复 + 确认卡片按钮）、解密正文的原因细分、角标、等主人确认任务的常驻通知与
   Live Update，做完在真机上验（迟到率、失败率、启动器角标表现）。
2. **iOS 的推送链路**（APNs、NSE、ActivityKit 推送更新）以 Apple 付费开发者账号为前提：能力表里 "Push Notifications" 只有付费的
   账号有勾（见"查证来源"）。账号就绪前，iOS 只有"App 在前台时"的本地通知这一条路。
3. NSE 是这一批里唯一有技术风险的一项，准入条件：真机量到 NSE 内存峰值（留 30% 余量）、两进程抢存储和隧道的两种情形都验过。

## 业内参考

| 做法 | 出处 | 我们抄什么、不抄什么 |
| --- | --- | --- |
| NSE 里跑完整 Matrix SDK 解密，内存收窄 | Element X iOS：`NSE/Sources/NotificationServiceExtension.swift`、`NSE/Sources/NSEUserSession.swift`、`ClientFactory.swift` 的 `makeNSEClient`（`.systemIsMemoryConstrained()`、只恢复一个房间、不建交叉签名、`NotificationProcessSetup::MultipleProcesses`） | 抄全部收窄手法和跨进程锁；不抄它的 CallKit 分支（ADR-0096 的通话是另一条线） |
| 通知动作 + 快速回复 + 接受/拒绝邀请按钮 | Element X Android：`QuickReplyActionFactory.kt`、`RejectInvitationActionFactory.kt`、`AcceptInvitationActionFactory.kt`；App 进程里解密（`libraries/matrix/impl/…/RustMatrixClient.kt:217` 用 `SingleProcess`） | 抄"每个按钮一个 ActionFactory + 抽取回复文本"的结构；我们的按钮文案和回答接口由核心定 |
| 推送唤醒后由 App 进程解密 | 同上；Signal iOS `SignalNSE/NotificationService.swift`（进程级单例环境、串行 `fetchQueue`、超时说明） | 抄"进程级只初始化一次""串行处理"；Signal 用自研 SQLite 和自有密码学，我们的加解密在核心 |
| NSE 不解码图片、拿不到就跳过 | Telegram iOS（`TelegramMessenger/Telegram-iOS`）的 `Telegram/NotificationService/Sources/NotificationService.swift`（`fetchResource` 返回 nil，不做媒体下载） | 照做：NSE 不下载附件、不做缩略图 |
| 角标用累加 | Element X iOS `NSE/Sources/NotificationHandler.swift` 的 `incrementBadgeCount`，并在撤回时减一 | 我们改成核心给绝对数，撤回和别处已读都能直接纠正（它做不到） |
| 卡片/事件添加入口（分享）不跑核心 | 本项目 [ADR-0097](0097-share-in.md) / PR #402 | NSE 必须跑核心，见第 2 节的准入条件 |

## 没选的做法

- **通知里带正文，中继直接给（像商业 IM）**：改 ADR-0003 的结论，端到端加密的房间就只能走明文预览，且中继能看到全部内容。
  没选，但可以作为以后"按房间开关预览"的扩展。
- **回复/按钮由 NSE 直接发 Matrix 请求**：NSE 里反正要起核心，再写一遍签名、重试、E2EE 加密是第二套事实源。没选。
  例外是确认卡片：它根本不经 Matrix（卡片只在核心里），只能打核心接口。
- **iOS 点按钮改成"打开 App 再答"**（`UNNotificationActionOptions.foreground`）：动作可靠，但会打断主人、跳走当前界面，
  和"在通知里答"的诉求相反。留着做兜底（`foreground` 只在冷启动起不来时用）。
- **Android 用 ShortcutBadger 这类库拿数字角标**：又一个依赖、还得逐厂商适配；先用系统给的（圆点 + `setNumber`），不够再说。
- **实时活动覆盖所有还在跑的 Agent 或每条消息**：看到的东西多，但一轮对话就能刷满锁屏，ActivityKit 每小时预算也紧；
  Google 也把聊天列为不适合 Live Update。只给等主人确认的任务。
- **NSE 里塞完整核心（照搬 App 的 `ClientOptions`）**：内存必爆。收窄见第 2 节。
- **把解密放进 App 的 `content-available` 后台唤醒**：要 `background` 权限和 App 常驻，且首条通知没有正文（先弹出去再改不了），
  正是 NSE 要解决的。仅作为"静默推送补正文"的补充。

## 代价与待验证项

- **最大依赖是账号**：免费个人团队没有推送能力（Apple 能力表，见"查证来源"）；"App Groups" 免费列有勾，
  所以 PR #402 的共享容器不受影响，受限的只是推送这一项能力。
- APNs 要中继签发/保管密钥，中继也要拿 ActivityKit 的 push-to-update token（ADR-0003 的 gorush 之外还要一段）。
- NSE 内存 24 MB 封顶是社区数据，Apple 未公开；要真机量到峰值才有定论。
- 跨进程设备密钥（同一把里面两条隧道）没有实测；抢不到锁、拿不到隧道时的降级必须是"取不到内容"，不能拖垮 NSE。
- 通知里打字回复走的是 30 秒级后台窗口，长文本 + 慢网会失败，靠"没答上"提示兜底；量的指标是失败率。
- Android 的数字角标随启动器；Live Update 有 OEM 额外条件（Google 在文档里明说）。
- 关掉锁屏内容后，正文是否还能被系统"预览"临时显示，取决于系统设置，我们只能尽力。
- 待验证清单：NSE 峰值内存；两进程抢存储/隧道；后台 action 的可用时间；ActivityKit 在 iOS 17/18/26 上的行为差异；
  Android 16 Live Update 在国产启动器上的实际表现。

## 查证来源（2026-11-13）

- Apple 能力表：https://developer.apple.com/help/account/reference/supported-capabilities-ios （"Push notifications" 免费列空、
  "App Groups" 免费列有勾）。
- NSE：`didReceive` "no more than 30 seconds"、超时由 `serviceExtensionTimeWillExpire` 给最后一次机会（超时则显示原始内容）：
  Apple `UNNotificationServiceExtension` 文档；24 MB 内存上限（有复现记录）：
  https://blog.kulman.sk/dealing-with-memory-limits-in-app-extensions 。
- ActivityKit：`Activity.request(attributes:content:pushType:)`、`pushTokenUpdates`、`apns-priority` 预算（10 计、5 不计）、
  8 小时硬上限（超时系统自动结束）、`NSSupportsLiveActivitiesFrequentUpdates`/`frequentPushesEnabled`：Apple ActivityKit 文档。
- 通知动作的后台启动："the system launches your app in the background"（Apple "Handling notifications and notification-related
  actions"）；官方未给后台运行时长数字，正文按"秒级到 30 秒量级、要真机量"处理。APNs 普通通知载荷上限 4 KB：Apple
  "Generating a remote notification"。
- Android 16 Live Updates：`setRequestPromotedOngoing`、`POST_PROMOTED_NOTIFICATIONS`（normal 权限）、`Notification.ProgressStyle`
  （API 36）、"聊天消息"列为不适合：developer.android.com（经 developer.android.google.cn 同源镜像核对）。`setShowBadge`、
  `setNumber` 的绝对数语义官方有据；"多数启动器只画圆点"无官方出处，正文按待真机验证处理。
- Element X iOS / Android、Signal iOS、Telegram iOS 的文件：见"业内参考"表，仓库与路径按表内。
- matrix-sdk：`NotificationProcessSetup::MultipleProcesses`（`vendor/matrix-sdk-ui/src/notification_client.rs:72`）、
  `try_lock_store_once`（`vendor/matrix-sdk-ui/src/encryption_sync_service.rs:138`、:159）、租约 500 ms
  （matrix-sdk-common-0.19.1 `cross_process_lock.rs:286`）。
