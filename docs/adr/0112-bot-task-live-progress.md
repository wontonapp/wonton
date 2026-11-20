# ADR-0112 Bot 长任务的锁屏进度：iOS 实时活动（灵动岛）、Android 进度通知

- 状态：提议（2026-11-20）。由主人拍板后才能开工，要拍板的取舍见“要主人拍板”
- 扩展：[ADR-0101](0101-notifications.md) 第 4 节（原来只给“等主人确认”的任务上灵动岛/实时活动）
- 依赖：ADR-0003 的加密推送链路和中继；iOS 的推送部分要 Apple 付费开发者账号（未就绪）
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（加密推送、中继）、[ADR-0015](0015-small-components-standard-interfaces.md)（标准接口、不为某个 Agent 开特例）、
  [ADR-0016](0016-mcp-bus-and-confirmations.md)（MCP 总线、确认卡片）、[ADR-0041](0041-app-links-and-shortcuts.md)（`wonton://` 链接）、
  [ADR-0043](0043-app-languages.md)（壳的文字）、[ADR-0082](0082-agent-run-status.md)（Agent 运行状态）、[ADR-0101](0101-notifications.md)（通知、锁屏内容、实时活动方向）、
  [ADR-0102](0102-app-lock.md)（App 锁、锁屏隐私）、[ADR-0108](0108-hermes-default-kernel.md)（只维护 Hermes）

## 背景

issue #9：Bot 跑长任务时（下载模型、整理照片、生成图片、跑代码），不用开着 App，也要能在锁屏和灵动岛上看到进度、当前步骤，
结束时给出提示，点进去回到那个聊天。iOS 用 ActivityKit 的实时活动（Live Activity）；Android 用进度样式的常驻通知。

已经有的地基：

- 一轮任务的状态消息格式（[ADR-0082](0082-agent-run-status.md)）：Bot 在请求所在的位置发一条 `m.notice`，字段是 MSC4564 的
  `working`、`waiting`、`done`、`failed`，加我们的 `app.wonton.end`（`cancelled`、`uncertain`）和 `step`、`steps`、`progress`。
  客户端核心把它显示成时间线里的一行，`Waiting` 有核心卡片时能点开卡片。
- 等确认的常驻通知：Android 的 `OngoingTasks.kt` 已在核心标出“连续等了一分钟”的 Bot 卡片后画一条进行中通知，
  API 36 上请求系统提升成 Live Update（ADR-0101 第 4 节）。
- 推送：Tuwunel 只把“有新消息”交给核心推送代理，代理按 RFC 8291 加密后交给中继，中继转发 APNs/FCM。中继和苹果都只看得到密文。
- 加密房间：状态消息在房间里是密文，核心推送代理既没有房间密钥，也不是 Matrix 客户端，看到推送时只有房间、事件 ID。

三条硬约束决定了设计：

1. **进度只有 Bot 自己知道**（下载到几成、在跑哪一步），核心拿到的只能是 Bot 的自述。和 ADR-0082 一样，它是自述，不是核心核对过的事实。
2. **实时活动的更新由系统直接解码 `content-state`**，不经通知扩展，也不经推送代理；所以走 APNs 的那一份是明文（ADR-0003 的加密链路在这里不成立）。
   Apple 自己的隐私指引也是“只用不敏感的概要，细节点进 App 看”。
3. iOS 推送要付费的开发者账号（ADR-0003 的范围调整、ADR-0101 的实施顺序）。账号到位前，iOS 只有 App 在运行时本地的实时活动；
   Android 不受影响。

范围：**一个人自己在聊天里发起的、真的在跑的任务**。Bot 的定时任务（晨报、备份）没有人当场发起，不上锁屏；只发普通通知或什么都不发。
跑不满 30 秒的回合不上锁屏，免得一次“谢谢”也占一条。

## 决策

### 1. 进度从 MCP 总线的 `wonton__report_progress` 来，核心聚合

Bot 经 MCP 总线报进度（[ADR-0016](0016-mcp-bus-and-confirmations.md)），核心自己聚合。核心是总线的主人，能认调用方、能节流、
能按着急程度决定推不推，不需要读懂房间里的密文。

- 新的核心工具 `wonton__report_progress`，`read_only: false`，**不要主人确认**（只是自报状态，不动任何东西）。
- 调用方必须是带 Bot 身份的插件（总线的令牌已经认出 `BotIdentity`），报的来源会话必须是核对得上的房间（`Caller.origin`）。
  工具插件、没有 Bot 的进程调用时回 `permission_denied`。
- 参数：

  ```json
  {
    "run": "$请求消息的事件 ID，可省",
    "state": "working | waiting | done | failed",
    "end": "cancelled | uncertain，只在 failed 时给",
    "phase": "thinking | reading | editing | deleting | moving | searching | command | web | waiting | other",
    "steps": 7,
    "progress": 0.42,
    "result": "结束时的一句话，只进聊天"
  }
  ```

  - `state` 和 `end` 的取值、含义和 ADR-0082 完全一样，不另立一套。
  - `phase` 是步骤的**类别**，照 ADR-0082 第 3 节工具类别那张表；不传全文（工具标题可能带命令参数、路径）。
  - `progress` 是可选的 0～1；下载这类知道总量的给，其余不给（不确定进度）。
  - `result` 是给聊天用的总结。锁屏不放自由文字（见第 7 节），所以它不进实时活动。
- 回应：`{"activity": "none" | "armed" | "running" | "ended", "reason": "..."}`。`none` 的 `reason` 说清为什么
  （`too_short`、`no_device`、`activities_disabled`、`too_many`），Bot 照实转告，不假装上了锁屏。
- `run` 缺省时按（Bot，房间）当同一轮；同一轮再来的调用推进它，`run` 换了就是新一轮，把旧的一轮按 `uncertain` 结束。
- 工具**不改** ADR-0082 的消息格式：状态消息还是聊天里的那一行，两边可以同时存在。对不上时锁屏以工具为准，聊天以消息为准。
- 核心不核对 `run` 那个事件真的存在（它不读房间），只核对 Bot 身份和来源房间：和 0082 一样，进度是自述。
- 核心把这条调用记进那条 Bot 的调用链和审计（ADR-0061），出问题能追到哪个 Bot 在自报。

### 2. 什么任务配得上锁屏：人来发起、跑够 30 秒、每台设备最多 2 条

- **发起人是本机的人**（`Caller.sender` 是本机的 MXID，或已报的轮次）：定时任务、别的 Bot 交给它干的活不上锁屏。
- **第一轮 `working` 先只“挂上”**（`armed`），到下面任一条件才真正开始：
  - 连续工作满 30 秒；
  - 报了 `progress`；
  - 进了 `waiting`。
  30 秒内就 `done`/`failed` 的，`activity` 一直是 `none`，只发普通通知。
- 目标设备只有发起人名下的已配对设备。
- **每台设备最多 2 条**并行的实时活动。超了按相关度取舍：`waiting` 优先，其次最早的；被挤掉的按 `uncertain` 结束。
- 主人随手划掉一条（ActivityKit 的 `activityState == .dismissed`）以后，这一轮不再为这台设备重开。
- 被 `waiting` 的卡片过期的、主人答完的，立刻结束。

### 3. 聚合、节流和超时

节流在核心，不在壳；规则写死，不按设备分。

- 只在内容真的变了才推：`state` 变、`phase` 变、百分比跨过 5 的整数倍、或者距上次已满 10 秒（`steps` 自己变不算）。
- 常规更新两次之间至少 5 秒；`waiting`、结束这类状态变化可以压到 2 秒。
- 比节流快的上报只留最后一条（合并，不排队）。
- 常规更新 `apns-priority: 5`（不占 Apple 每小时的额度）；**只有进 `waiting`、结束、被挤掉**用 `10`。v1 不开
  `NSSupportsLiveActivitiesFrequentUpdates`，按需再说。
- `stale-date` 永远是“上次更新 + 120 秒”。这条运行 2 分钟没有新上报、或者 Bot 离线（ADR-0044 的 `offline`），
  把锁屏改成“不确定”，不转圈；10 分钟没动静或跑满 8 小时（ActivityKit 的上限）就结束，状态写 `uncertain`。
- 结束的实时活动在锁屏再留 15 分钟（`dismissal-date`），够主人回来看结果；`done` 也留，不立刻撤。
- Android 同一套节流；系统只要求通知提示音一秒最多一次，我们远低于它。

### 4. 数据形状：核心给类别和数，壳写文字

服务端新增 `GET /_wonton/api/runs`（已配对设备，同其他 ViewModel 的鉴权）和客户端核心的
`subscribe_live_runs()`：整份列表，每项是

```rust
pub struct LiveRunsViewModel { pub runs: Vec<LiveRun> }   // 最多按设备 2 条取，服务端已经挑过

pub struct LiveRun {
    pub server_id: String,        // 多服务器：点开、推送都按这台
    pub run_id: String,
    pub room_id: String,
    pub thread_root: Option<String>,
    pub bot_name: String,         // 只在本地画；进 APNs 载荷的没有
    pub state: AgentRunState,     // 复用 ADR-0082：Working/Waiting/Done/Failed/Uncertain/Other
    pub phase: RunPhase,          // 未知落 Other
    pub percent: Option<u8>,      // 0～100，只在有确定进度时给
    pub steps: u32,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub stale_at_ms: u64,
    pub card_id: Option<String>,  // state 是 Waiting 且是核心卡片时
    pub choices: Vec<CardChoice>, // 卡片没有表单时最多 3 个，用来画锁屏上的按钮
}
```

- 壳只按枚举和数画：文字全在各端本地化资源里（ADR-0043），核心不给展示句子。
- `Uncertain` 由核心合成（Bot 离线、2 分钟没更新），和客户端核心在聊天里的合成规则一致（ADR-0082 第 4 节）。
- 百分比超出 0～100 按没有进度画；`phase` 不认识落 `Other`，壳照旧画一条不确定进度，不丢整条。

### 5. 传输：连着就在本地更新，断了才推送

- App 的进程活着、隧道连着时，客户端核心已经拿得到上面的列表，壳直接更新实时活动/通知，
  **核心不再为这台设备发同样的更新**（Apple 也要求别把本地更新和推送叠着做）。核心按设备当前的隧道连接判断。
- 隧道断了（锁屏、进程被挂起、被杀）才走推送：
  - **iOS**：ActivityKit 推送，明文，`apns-push-type: liveactivity`，`apns-topic: <bundle>.push-type.liveactivity`。
  - **Android**：现有的加密推送再加一种数据载荷 `run_progress`，设备上 `WontonPushService` 解开后更新对应通知；
    Android 本来就是本机解密，加密不受这次改动影响。
- 本地路径上，iOS 壳对 `LiveRun` 用自己的 1 秒合并；`waiting`、结束立刻画。Android 同样。
- iOS 只在场景是 `active` 时用 `Activity.request` 本地开始（后台开始要 `LiveActivityIntent`，这里不用）；
  App 不在前台就由核心的 push-to-start 开，17.0/17.1 没有 push-to-start，后台就不开。
- 推送路径的壳不猜：载荷里有什么画什么，`timestamp` 旧的丢掉。

### 6. 设备登记和中继

**iOS 的令牌**都由 App 经隧道登记给核心，存活在 `core/push/live.json`：

- 首次配对、App 启动时给每台已配对的服务器登记同一个 **push-to-start** 令牌（`Activity.pushToStartTokenUpdates`，iOS 17.2 起），
  用来在 App 没开时把活动开起来；一台设备只有一个，换新时删旧的。
- 每条活动起来以后登记它自己的更新令牌（`activity.pushTokenUpdates`），绑定（设备，`run_id`，`activity_id`）。
- 设备经核心登记：`PUT /_wonton/push/live`（只有隧道里的已配对设备，body `{kind: "start"|"update", token, run_id?, activity_id?}`）、
  `DELETE /_wonton/push/live`；壳用新 `ClientCore` 方法走它，不加 `Action` 变体（同 ADR-0101/0102 的先例）：
  `set_live_activity_token(server_id, token, kind, run_id?, activity_id?)`、`clear_live_activity_token(...)`、
  `live_activity_changed(server_id, run_id, activity_id?, state)`（`started`/`ended`/`dismissed`，划掉的别重开）。
- 设备报告 `ActivityAuthorizationInfo.areActivitiesEnabled == false` 时核心不再为它发起（`activities_disabled`）；
  系统开关变了（`activityEnablementUpdates`）壳就再报一次。
- 中继（gorush 之上那层鉴权）加一个 `POST /v1/live-activity`：核心发
  `{token, priority, aps}`，中继补 `apns-push-type`、`apns-topic`（固定成 App 的 bundle 加后缀）、`apns-priority` 后交 gorush
  （gorush 已原生支持 `push_type`、`topic`、`content-state`、`stale-date`、`relevance-score`、`attributes-type`、`attributes`）。
  鉴权沿用服务器现有的中继凭据。APNs 回 410/BadDeviceToken 时把令牌删掉并告诉核心。
- `push-to-start` 只在“这轮在这台设备上没有活动、连接也断着”时用：一轮最多一次，失败就等下一次更新重试。
  令牌在活动起来后立刻换成更新令牌。
- iOS 18 起的开始载荷带 `input-push-token: 1`，确保系统给更新令牌；17.2/17.3 由系统随开始一并给出。

### 7. 隐私：明文里只有枚举、数和三个不透明 ID

`content-state`（每次更新都全量给）：

```json
{ "state": "working", "phase": "searching", "pct": 42, "steps": 7, "staleAt": 1763700120, "card": null }
```

开始载荷的 `attributes`（之后不变）：

```json
{ "server": "<配对码里的 endpoint ID>", "room": "!abc:host", "run": "$事件 ID" }
```

- 不放：消息正文、文件名、房间名、Bot 名字、命令、模型名、结果总结。
- `room`、`run` 是随机的 Matrix 标识，`server` 是公开的 endpoint ID；这三样放明文只说明“有个任务在跑”，看不出内容。
- 指向卡片时只放卡片 ID（`card`），选项文字也不进明文；按钮上的“允许/拒绝”是壳写死的词。
- 提示（`alert`）用 `loc-key` 加数字参数，由 App 按系统语言写。进入 `waiting` 和结束时才带提示，平常更新不响。
- 锁屏内容单独一个开关（新增 `NotificationCategory::Task`，默认开）；关掉只写“有任务在跑”。
- App 锁（ADR-0102）开着且锁定时：实时活动照常显示百分比和条，但不显示阶段文字，标题写成“云吞”；数字不算敏感内容（ADR-0101 第 5 节的同一条）。
  锁定标记写进 App Group，实时活动的视图和通知扩展读同一个文件。
- 机主在系统里关掉实时活动、或者 Always-On 只留概要时，如实变少，不补假字。

### 8. 点进去打开哪里、等确认时的动作

- 点实时活动（任何一处）：`wonton://room/<房间 ID>`，壳先盖锁定页，解锁后进那个聊天（ADR-0102 已规定深链要穿过锁屏）。
  v1 只保证当前这台服务器；别的服务器的房间先停在切换那一步，多服务器的 `?server=` 后续补。
- `waiting` 时：`relevance-score` 用 100（平时 50）并带提示，灵动岛显示这一条；卡片还在，列表里点了能到卡片。
- 卡片没有表单字段（和 ADR-0101 第 1 节同一个条件）时，锁屏和通知上给“允许”“拒绝”：
  - iOS：`LiveActivityIntent` 的按钮，系统在后台拉起 App 进程，走 `answer_push_confirmation` 同一条回答路径；
  - Android：`NotificationCompat.Action` → 现有的 `PushActionReceiver`；
  - 要填密钥或设置的卡片只给“打开”，不在锁屏上填。

### 9. 平台约束和降级

- **iOS**：实时活动本身不要付费账号，`NSSupportsLiveActivities` 就能本地开；**推送**（开始、更新、结束）要付费账号和推送能力。
  账号到位前 iOS 只在 App 运行时本地更新，锁屏后停在最后一份内容；文案不承诺“关掉 App 也能看”。
- iOS 17.0～17.1 能本地开始、能被推送更新，但不能被推送**开始**；17.2 起才有 push-to-start。18 起是 `input-push-token`，
  27 起横屏也显示紧凑视图（要按 `isDynamicIslandLimitedInWidth` 收窄）。
- 没有灵动岛的机型是锁屏实时活动；iOS 16.1 以下没有。
- Android：`NotificationCompat.ProgressStyle` 只在 API 36 起有进度条外观，更早的版本退回普通 `setProgress`；
  成为 Android 16 的 Live Update 要清单里的 `POST_PROMOTED_NOTIFICATIONS`（普通权限）、`ongoing`、有标题、渠道不是 `IMPORTANCE_MIN`，
  用户把 Live Update 关了就不再请求提升；OEM 还会有自己的额外条件。
- Android 的通知权限被关：进度通知和上面的活动都画不出来，设置页如实指去系统设置；iOS 的实时活动不受通知权限影响。
  Android 没装分发器、又在保持连接时，进程被系统杀掉后通知停在最后一格，打开 App 再继续。
- 全局开关在设置里，按设备；关掉后不再登记令牌，已有的活动收尾。

### 10. 壳的文字和画法

两端各一份，四语言齐全，按 `scripts/ui_text/groups.py` 分组：`live_run.*`。

| `phase` | 文案（中文示例） |
| --- | --- |
| thinking | 在想办法 |
| reading / editing / deleting / moving | 在读 / 在改 / 在删 / 在整理 |
| searching / command / web | 在找 / 在跑命令 / 在上网 |
| waiting | 等你确认 |
| other、未知 | 在干活 |

`state`：`working` 画转圈或进度条；`waiting` 画醒目的“等你确认”和按钮；`done` 画对勾和用时；`failed` 画错误，`cancelled` 写“打断了”，
`uncertain` 写“结果不确定”，不画红。

iOS 的呈现按平台指引：锁屏一条，灵动岛紧凑左侧 App 标记、右侧百分比或转圈，最小视图只放百分比，展开视图放阶段、步数、进度条、“打开”。
`isStale` 时一行“进度可能过期”。Android 一条 `setOngoing` + `setOnlyAlertOnce`，标题是 Bot 名（本地画），正文是阶段，
`setWhen(started_at)` + `setUsesChronometer`，API 36 起用 `setShortCriticalText` 放百分比，`setDeleteIntent` 记下被划掉。

### 11. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `runs` 模块（上面的结构体和 `RunPhase`，测试盖未知值落 `Other`、`LiveRun` 的字段往返）；
  `wonton__report_progress` 的参数和错误类别；中继的 `POST /v1/live-activity` 与 APNs 载荷（固定 golden 载荷测试）。
- **提供方**：家里的核心（聚合、节流、推送、VM）+ 中继（APNs）+ 两端壳（ActivityKit、通知）。
- **消费方**：两端壳。换中继不改核心和壳（只有 HTTP 契约）；换客户端只要实现“拉 `/_wonton/api/runs` 或解 `run_progress`
  → 画列表 → 深链”，核心不认平台。
- 能证明换提供方的契约测试：假 APNs 当中继的下游，断言核心发给中继的请求、中继转出的 APNs 头、载荷和 410 处理都不随后端变化。

### 12. 分步落地

每一步单独合并、单独验收；做不动就停在前面，不留半截。

1. **核心**：`run_progress` 模块（注册表、聚合、节流、`Uncertain` 合成、8 小时/10 分钟收尾）、`wonton__report_progress`、
   `GET /_wonton/api/runs`、`subscribe_live_runs()`。
   验收：行为测试——工具序列到 VM 的迁移、30 秒门槛、5 点/10 秒节流与合并、两个上限和挤掉、离线变不确定、超时结束、
   定时任务不上锁屏、别的 Bot 的调用不能推进。
2. **Android 画**：从 VM 画进度通知（36 上 `ProgressStyle`、以下 `setProgress`）、提升请求、深链、公开版和隐藏版、设置开关。
   验收：投递假 VM 的截图测试；模拟器上对着开发栈真跑一遍（装、跑长任务、锁屏看、点回聊天）。
3. **Android 推送**：加密载荷 `run_progress`、`resolve_push` 分开消息和进度、410 处理。
   验收：核心契约测试；模拟器装分发器，杀掉 App 后看进度还能动。
4. **iOS 本地**：widget 扩展里的 `ActivityConfiguration`、本地开始/更新/结束、深链、App Group 的锁定标记、授权探测。
   验收：固定假 VM 的预览（锁屏、紧凑、最小、展开）；模拟器走查；关掉实时活动后不再开。
5. **iOS 推送和中继**：中继接口、令牌登记、开始/更新/结束的载荷、priority、`stale-date`、`dismissal-date`。
   验收：本机对 APNs 沙箱 curl；模拟器用 `simctl push` 打活动载荷；真机在账号到位后走一遍（开始、更新、结束、410）。
6. **等确认的动作**：活动/通知上的允许、拒绝，接 `answer_push_confirmation`；把现有 `OngoingTasks` 的确认通知并进同一份注册表。
   验收：卡片、过期、答完后的撤下；两端各一遍。

## 考虑过的做法

- **核心自己也当 Matrix 客户端，读状态消息来聚合**：客户端核心已经有全部解析（`agent_run.rs`），但服务端核心要一个自己的账号、
  自己的设备和 Megolm 密钥，还要管房间成员关系和回执，等于把聊天客户端塞进核心；而且 App 被杀时正好是它不在的时候。
  没选。以后要“服务端看得懂聊天”时再评估。
- **客户端核心报进度给服务端**：App 被杀就断了，恰好是最需要的时候。没选。
- **把 `step` 原话（状态消息里那句话）放进明文**：它可能是工具标题，带路径、命令、参数；还得靠壳截断。没选，只放类别。
- **让 `content-state` 自加密（密文 blob + App Group 里的钥匙）**：中继也看不到，但要自己管钥匙、自己写编解码，
  `alert` 的 `loc-key` 和系统渲染都用不上，4 KB 更紧。留作以后；见“要主人拍板”。
- **全部用普通通知，不做实时活动**：锁屏上是十几条各自刷新的通知，看不出是一条任务到哪一步；APNs 明文也照样要过。
- **开始时直接上 priority 10**：一次“谢谢”也亮屏；30 秒门槛加 priority 5 更安静。
- **Android 单开一个前台服务**：已有 specialUse 的保持连接服务；推送时进程由推送拉起来画完就完，不需要第二个。

## 代价与风险

- 中继第一次看到明文（概要）。泄露面是“有任务、到几成、哪一类”，不含内容；但这是对 ADR-0003 的实质例外，得主人点头。
- 进度是 Bot 的自述：Bot 卡住不报时锁屏会停在旧值，靠 `stale-date` 和“不确定”兜底，不补猜。
- APNs 预算、OEM 的 Live Update 条件、iOS 17.2 以前的限制都是平台侧、只能按上面降级。
- 每人每设备两条，多了会挤；挤掉的按不确定结束，主人可能觉得忽然没了。
- `GET /_wonton/api/runs` 和 `PUT /_wonton/push/live` 是又一组常驻接口；前者只在有运行时才有内容，没有时不唤醒。
- 明文里的 `room`/`run` 让中继能对上一台设备连续看了哪些任务（次数和时长），对不上具体内容。
- 实时活动的视图在 widget 扩展里，读 App Group 的锁定标记；扩展内存和时序沿用 ADR-0101 通知扩展那一套的结论，要真机验。

## 要主人拍板

1. **明文概要（推荐）还是自加密让中继也看不到？** 推荐明文：只放枚举、百分比和三个随机 ID，换来实现简单、系统原生呈现全能用；
   自加密要多一套钥匙和编解码，锁屏提示也用不上本地化。
2. **哪些任务上锁屏？** 推荐“人当场发起、跑过 30 秒、每设备最多 2 条”。备选是维持 ADR-0101 的“只在等你确认时”，
   或者由 Bot 自己声明“这是长任务”（安静，但会漏报，Bot 判断错就没有）。
3. **默认开还是默认关？** 推荐默认开、给总开关和类别开关：这是主人自己发起的活，预览一眼就知道在跑什么；怕烦的关一类就行。

## 影响

- **ADR-0003**：通过后要在它里面补一节例外：实时活动的概要是明文，只放枚举、数和随机 ID，文字在壳里；普通消息的推送加密不变。
  本节先记着，本 PR 不改它的正文。
- **ADR-0101**：第 4 节的范围从“等确认”扩到长任务进度；第 5 节要给 `Task` 类别加锁屏开关；第 1 节的按钮要加实时活动入口。同样是它通过后再补。
- **ADR-0082**：状态取值和类别的**含义**复用，消息格式不动；锁屏的数据来自新工具，冲突时两边各按自己的源。
- **ADR-0016**：核心工具表加 `wonton__report_progress`（不要确认）。
- **ADR-0102**：锁定期间实时活动隐藏阶段文字；深链穿过锁定页的规则不变。
- **ADR-0061**：进度上报记进调用链和审计。
- **ADR-0043**：新增 `live_run.*` 文案组，四语言。
- **ADR-0015、ADR-0108**：工具在总线上，任何能说 MCP 的 Agent 都能用，核心不认 Hermes。
- **ADR-0044**：Bot 离线时锁屏显示不确定，沿用它的活动状态。
- **ADR-0041**：不动链接语法，只复用 `wonton://room/<id>`。

## 查证来源（2026-11-20）

- Apple“Starting and updating Live Activities with ActivityKit push notifications”：`apns-push-type: liveactivity`、topic 加
  `.push-type.liveactivity`、priority 5 不计预算 / 10 计入、`pushTokenUpdates`、push-to-start（17.2 起）、`input-push-token`（18 起）、
  `stale-date`、`relevance-score`、`dismissal-date`、结束后的默认保留时间：
  https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications 。
- Apple“Displaying live data with Live Activities”：静态加动态合计 4 KB 上限、8 小时活动上限、锁屏最多再 4 小时：
  https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities 。（原文 URL 为
  `/documentation/activitykit/displaying-live-data-with-live-activities`。）
- `ActivityContent(state:staleDate:relevanceScore:)`、`ActivityState`（含 `dismissed`）、`ActivityAuthorizationInfo`、
  `request(attributes:content:pushType:style:)`（18 起，`.standard`/`.transient`）、`LiveActivityIntent`（17 起，后台拉起 App 才能开活动）、
  `pushToStartTokenUpdates`（17.2 起）：同上站点各自的符号页。
- Apple HIG“Live Activities”：不露敏感信息、只在新内容时更新、别和本地更新叠着推送、结束时立刻结束并给 15～30 分钟的撤下时间、
  “Don't use a push notification alongside Live Activities for the same updates”：
  https://developer.apple.com/design/human-interface-guidelines/live-activities 。
- WWDC26“Live Activities essentials”：iOS 27 在横屏也显示紧凑/最小视图，按 `isDynamicIslandLimitedInWidth` 收窄：
  https://developer.apple.com/videos/play/wwdc2026/223 。
- Android“Live update notifications”：`POST_PROMOTED_NOTIFICATIONS`（普通权限）、`setRequestPromotedOngoing`、必须 ongoing /
  有标题 / 不用 RemoteViews / 渠道不是 MIN、用户能划掉、别重发被划掉的：
  https://developer.android.com/develop/ui/views/notifications/live-update 。
- Android“Create a progress-centric notification”与 `Notification.ProgressStyle`（API 36）、`NotificationCompat.ProgressStyle`
  （androidx.core 1.17.0，36 以下退回默认样式）：
  https://developer.android.com/develop/ui/compose/notifications/progress-centric 。
- Android 通知提示音一秒最多一次（API 27 起）：
  https://developer.android.com/develop/ui/compose/notifications/create-notification 。
- gorush 已支持 `push_type`、`topic`、`content-state`、`stale-date`、`dismissal-date`、`relevance-score`、`attributes-type`、`attributes`
  （`notify/notification.go`、`notify/notification_apns.go` 的 `GetIOSNotification`，仓库 appleboy/gorush，MIT）。
- 现有代码：`crates/wonton-core/src/push/`（加密推送、登记、`send_confirmation`）、`crates/wonton-client/src/chat/pending.rs`
  （`LONG_WAIT_MS` 和 `ongoing`）、`apps/compose/androidApp/src/main/kotlin/app/wonton/android/push/OngoingTasks.kt`、
  `crates/wonton-protocol/src/agent_run.rs`、`apps/ios/project.yml`（已有 widget 扩展、iOS 17.0、App Group）。

## 关联

[ADR-0003](0003-matrix-and-encrypted-push.md)、[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0016](0016-mcp-bus-and-confirmations.md)、
[ADR-0041](0041-app-links-and-shortcuts.md)、[ADR-0043](0043-app-languages.md)、[ADR-0044](0044-bot-profile-and-status-avatars.md)、
[ADR-0061](0061-agent-runtime-and-frontends.md)、[ADR-0082](0082-agent-run-status.md)、[ADR-0101](0101-notifications.md)、
[ADR-0102](0102-app-lock.md)、[ADR-0108](0108-hermes-default-kernel.md)、[ADR-0110](0110-one-hermes-container-per-person.md)。
