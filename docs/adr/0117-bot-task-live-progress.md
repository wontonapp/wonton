# ADR-0117 Bot 长任务的锁屏进度：iOS 实时活动（灵动岛）、Android 进度通知

- 状态：提议（2026-11-20）。由主人拍板后才能开工，要拍板的取舍见“要主人拍板”
- 扩展：[ADR-0101](0101-notifications.md) 第 4 节（原来只给“等主人确认”的任务上灵动岛/实时活动）
- 边界：[ADR-0113](0113-media-transport.md) 的传输、播放会话是核心自己持有的长任务，进度和锁屏走它自己的那套，不并进本篇；
  本篇只管聊天里 Bot 的回合。收件人按人的规则引用 [ADR-0114](0114-plugin-agent-platform.md) 第 4 项，深链引用第 6 项
- 依赖：ADR-0003 的加密推送链路和中继；iOS 的推送部分要 Apple 付费开发者账号（未就绪）

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
- 调用方必须是带 Bot 身份的插件（总线的令牌已经认出 `BotIdentity`）。报的来源会话取自 `Caller.origin`，它是**自述**：
  核心不核对成员和加入状态，核对过的只有确认卡片的 `checked_origin`/`both_joined`（`crates/wonton-core/src/mcp/audience.rs`）。
  报错房间只影响深链落到哪个聊天、以及 `run` 缺省时的轮次键，不泄露别的房间（房间 ID 本来就在明文里）。
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
  （`not_a_turn`、`too_short`、`no_device`、`activities_disabled`、`too_many`），Bot 照实转告，不假装上了锁屏。
- `run` 缺省时按（Bot，房间）当同一轮，但这时必须靠 `Caller.chain` 证明这是本 Bot 的活跃真人轮次（见第 2 节）；
  同一轮再来的调用推进它，`run` 换了就是新一轮，把旧的一轮按 `uncertain` 结束。
- 工具**不改** ADR-0082 的消息格式：状态消息还是聊天里的那一行，两边可以同时存在。对不上时锁屏以工具为准，聊天以消息为准。
- 核心不核对 `run` 那个事件真的存在（它不读房间），也不核对来源房间的成员：和 0082 一样，进度和来源都是自述。
- 核心把这条调用记进那条 Bot 的调用链和审计（ADR-0061），出问题能追到哪个 Bot 在自报。

### 2. 什么任务配得上锁屏：人来发起、跑够 30 秒、每台设备最多 2 条

- **先过“是人来发起的一轮”**：这轮得挂在真实的人身上。判据二选一：
  - 调用带 `run`（Matrix 事件 ID），核心不核对它存在，只当“这轮有出处”；
  - `Caller.chain` 是本 Bot 的活跃轮次，且这条链代表一个本机的人：`live_turns()` 里找得到这条链，
    `Chains::chain_user(chain)` 归一是 `Local`，且 `person_ai_option` 认得它（主人，以及已加入/已邀请的成员都算；
    远端代表认不出）。说话人本身是 Bot 账号也一样：Bot 把活转进来时，链跟着叫回走，或者按点名者的归属人开链，
    链代表的是真人（ADR-0061 第 10 节；取法和 `audience.rs` 的 `bot_speaker_ai` 相同）。
  两样都没有（定时任务、链代表认不出）回 `none`/`not_a_turn`，只发普通通知。定时任务没有 `run`，也没有活跃轮次
  （ADR-0050、ADR-0083：报不了轮次的按只读回落），落在这一条上。这就是背景里“定时任务不上锁屏”的落地规则，
  也是第 12 节验收的那条测试。`waiting` 不例外：没进这道门的等待只发 ADR-0101 的普通确认通知。
- **发起人（只用来挑设备）**：`Local(mxid)` 是本机主人时映射成主人（设备表里的 `None`），家人映射成用户名。
  链在就用链的 `chain_user`，优先于 `Caller.sender`（Bot 传话时 `sender` 是 Bot 账号，链上的 `chain_user` 才是真人）；
  没有链、`sender` 本身是本机 Bot 账号时（带了 `run`）用它的归属人。
  `None`（没报说话人）按 Bot 的归属人算，共享 Bot 没有单个归属人，按主人；`Unknown`（远端好友、认不出）不上锁屏。
  设备用 `PushService` 现有的 `devices_of(person: Option<&str>)` 取（`None` 是主人，其余是用户名），不直接把 MXID 传给它。
- **第一轮 `working` 先只“挂上”**（`armed`），到下面任一条件才真正开始：
  - 连续工作满 30 秒；
  - 报了 `progress`；
  - 进了 `waiting`。
  30 秒内就 `done`/`failed` 的，`activity` 一直是 `none`，只发普通通知。
- 目标设备只有发起人这个“人”名下的已配对设备，规则同 [ADR-0114](0114-plugin-agent-platform.md) 第 4 项的按人收件：主人是 `None`，家人是用户名。
- Google 把“聊天消息”列为不适合 Live Update，长任务是另一种形状：人当场发起、有开始和结束、不是一条条新消息刷屏，
  所以只有这里申请提升，普通消息仍然是普通通知（和 ADR-0101 第 4 节同一句）。
- **每台设备最多 2 条**并行的实时活动，取舍规则写死：
  1. 没满 2 条：进；
  2. 满了且还有 `armed`/`working` 的：挤掉最早的非 `waiting`，被挤的按 `uncertain` 结束；
  3. 满了且两条都在 `waiting`：新的这轮回 `none`/`too_many`，不挤正在等主人的；下一次上报再试。
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
`subscribe_live_runs()`：整份列表，每项是下面的结构。这份列表吸收 ADR-0101 第 4 节的 `runs_waiting()`：以前那个“只给 `Waiting`、
最多 3 条”的视图不再单独存在，等确认的项就是本列表 `state == Waiting` 的部分。两者共用一条上限（每设备 2 条）和
第 2 节的同一套取舍：`Waiting` 优先保留、两条都在等时新来的回 `too_many`；没有“3 条确认 + 2 条进度”各算各的。
只收第 2 节门槛放行的轮次，定时任务的等待不走这里。

```rust
pub struct LiveRunsViewModel { pub runs: Vec<LiveRun> }   // 最多按设备 2 条取，服务端已经挑过

pub struct LiveRun {
    pub server_id: String,        // 多服务器：点开、推送都按这台
    pub run_id: String,
    pub room_id: String,
    pub thread_root: Option<String>,
    pub bot_name: String,         // 只在本地画；进 APNs 载荷的没有
    pub state: AgentRunState,     // 复用 ADR-0082：Working/Waiting/Done/Failed/Uncertain/Other
    pub end: Option<AgentRunEnd>, // 复用 ADR-0082；`Uncertain` 归一成 `state`，留下的只有 `Cancelled`
    pub phase: RunPhase,          // 未知落 Other
    pub percent: Option<u8>,      // 0～100，只在有确定进度时给；明文里叫 pct
    pub steps: u32,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub stale_at_ms: u64,
    pub card_id: Option<String>,  // state 是 Waiting 且是核心卡片时
    pub can_answer: bool,         // 只有标准同意/拒绝按钮时给 true；见第 8 节
}
```

- 壳只按枚举和数画：文字全在各端本地化资源里（ADR-0043），核心不给展示句子。
- `Uncertain` 由核心合成（Bot 离线、2 分钟没更新、被挤掉），和客户端核心在聊天里的合成规则一致（ADR-0082 第 4 节）；
  Bot 自报 `end: uncertain` 也归一到它。`Cancelled` 只留在 `end` 上，`state` 仍是 `Failed`。
- 百分比超出 0～100 按没有进度画；`phase` 不认识落 `Other`，壳照旧画一条不确定进度，不丢整条。
- `can_answer` 由核心算：卡片没有表单字段，且选项恰好是核心补的 `approve`/`reject` 这一对。自定义选项的标签不进推送，
  所以这种卡片在锁屏上只给“打开”（第 8 节）。

### 5. 传输：连着就在本地更新，断了才推送

- App 的进程活着、隧道连着时，客户端核心已经拿得到上面的列表，壳直接更新实时活动/通知，
  **核心不再为这台设备发同样的更新**（Apple 也要求别把本地更新和推送叠着做）。核心按设备当前的隧道连接判断。
- 隧道断了（锁屏、进程被挂起、被杀）才走推送：
  - **iOS**：ActivityKit 推送，明文，`apns-push-type: liveactivity`，`apns-topic: <bundle>.push-type.liveactivity`。
  - **Android**：现有的加密推送再加一种数据载荷 `run_progress`，设备上 `WontonPushService` 解开后更新对应通知；
    Android 本来就是本机解密，加密不受这次改动影响。
- 本地路径上，iOS 壳对 `LiveRun` 用自己的 1 秒合并；`waiting`、结束立刻画。Android 同样。
- iOS 只在场景是 `active` 时本地开始（后台开始要 `LiveActivityIntent`，这里不用）；App 不在前台就由核心的 push-to-start 开，
  17.0/17.1 没有 push-to-start，后台就不开。
  - `request` 带 `style:` 的重载是 iOS 18 起：18 以上走 `request(attributes:content:pushType:style: .standard)`，
    17.0/17.1 走 `request(attributes:content:pushType:)`；`pushToStartTokenUpdates`（17.2 起）、
    `isDynamicIslandLimitedInWidth`（27 起）都套 `#available`。
- 本地开始和 push-to-start 之间有竞态（核心以为这台设备没有活动，壳刚好本地起了一条），同一轮会出两条活动。去重两头都做：
  壳本地一开起来就先报 `live_activity_changed(...started, activity_id)`，核心把它记进 (设备, run)，没有 `ended`/`dismissed` 之前不再发 start；
  壳收到 start 推送时先查自己有没有这个 (server, run) 的活动，有就结束新来的那条、留下先有的，再按活动 ID 重新报一次 `started`。
  更新令牌跟着活下来的那条走。
- 推送路径的壳不猜：载荷里有什么画什么，`timestamp` 旧的丢掉。

### 6. 设备登记和中继

**注册表（第 4 节）是唯一的 run 清单**：活动、令牌都按它里面的 (设备, run) 绑定；ADR-0101 的 `runs_waiting()` 并进来以后，
等确认的卡片也从这里取，不再另存一份等待视图。`Waiting` 和进度共用一个每设备上限、一套取舍（第 2 节），
不因为“在等主人”另开额度。

**iOS 的令牌**都由 App 经隧道登记给核心，存活在 `core/push/live.json`：

- 总开关开着时，首次配对、App 启动时给每台已配对的服务器登记同一个 **push-to-start** 令牌（`Activity.pushToStartTokenUpdates`，iOS 17.2 起），
  用来在 App 没开时把活动开起来；一台设备只有一个，换新时删旧的。开关后来打开时补登记，关掉时删。
- 每条活动起来以后登记它自己的更新令牌（`activity.pushTokenUpdates`），绑定（设备，`run_id`，`activity_id`）。
- 设备经核心登记：`PUT /_wonton/push/live`（只有隧道里的已配对设备），body 三种：
  `{kind: "start", token, run_id}`、`{kind: "update", token, run_id, activity_id}`、`{kind: "enabled", enabled}`；
  `DELETE /_wonton/push/live` 带 body `{run_id?}`：带 `run_id` 只清这一轮（`ended`/`dismissed` 后），
  不带 `run_id` 清这台设备的全部令牌（退出服务器、关掉系统开关）。
  壳用新 `ClientCore` 方法走它，不加 `Action` 变体（同 ADR-0101/0102 的先例）：
  `set_live_activity_token(server_id, token, kind, run_id?, activity_id?)`、`clear_live_activity_token(server_id, run_id?)`、
  `live_activity_changed(server_id, run_id, activity_id?, state)`（`started`/`ended`/`dismissed`，划掉的别重开）。
- 有两层开关，都由核心判定：
  - **系统的**：设备报告 `ActivityAuthorizationInfo.areActivitiesEnabled`（iOS）或通知权限与
    `NotificationManager.canPostPromotedNotifications()`（Android）算不算可用，经 `kind: "enabled"` 报上来；
    系统开关变了（`activityEnablementUpdates` 等）壳就再报一次。不可用时核心不再为它发起（`activities_disabled`）。
  - **我们的总开关**：`live_activity_enabled(server_id, enabled)`，存核心的本机文件（同 `appearance.json`/`push-method.json`，
    ADR-0102 第 2 节），默认开；设置页新增 `SettingsEntry::TaskProgress { enabled }`。关掉时核心结束已有活动、删掉令牌；
    再打开由壳重新登记。
- 中继（gorush 之上那层鉴权）加一个 `POST /v1/live-activity`：核心发
  `{token, priority, aps}`，中继补 `apns-push-type`、`apns-topic`（固定成 App 的 bundle 加后缀）、`apns-priority` 后交 gorush
  （gorush 已原生支持 `push_type`、`topic`、`content-state`、`stale-date`、`relevance-score`、`attributes-type`、`attributes`）。
  鉴权沿用服务器现有的中继凭据。APNs 回 410 `Unregistered`（令牌作废）或 400 `BadDeviceToken`（令牌不属于这个环境和 topic）时把令牌删掉并告诉核心。
- `push-to-start` 只在“这轮在这台设备上没有活动、连接也断着”时用：同一轮同一台设备同时只发一条，失败就等下一次更新重试。
  令牌在活动起来后立刻换成更新令牌。
- iOS 18 起的开始载荷带 `input-push-token: 1`，确保系统给更新令牌；17.2/17.3 由系统随开始一并给出。

### 7. 隐私：明文里只有枚举、数和四个不透明 ID

`content-state`（每次更新都全量给）：

```json
{ "state": "working", "end": null, "phase": "searching", "pct": 42, "steps": 7, "canAnswer": false, "card": null }
```

开始载荷的 `attributes`（之后不变）：

```json
{ "server": "<配对码里的 endpoint ID>", "room": "!abc:host", "run": "$事件 ID" }
```

- 不放：消息正文、文件名、房间名、Bot 名字、命令、模型名、结果总结、选项文字。
- 四个 ID 都不透明：`room`、`run` 是随机的 Matrix 标识，`server` 是公开的 endpoint ID，`card` 是随机卡片 ID。
  放明文只说明“有个任务在跑”，看不出内容。
- 指向卡片时只放 `card` 和 `canAnswer`；按钮上的“允许/拒绝”是壳写死的词，进本地化资源。
  确认通知自己的推送（ADR-0101 §5）照旧带选项标签，那两个词的来源是核心，不是用户数据；实时活动这边为了本地化和 4 KB 只放布尔。
- 提示（`alert`）只走 iOS 推送，只在进 `waiting` 和结束时带，平常更新没有 `alert` 这个键。用 `aps.alert` 的 `loc-key` 加
  `loc-args`，`loc-args` 只放数字（步数、百分比），由系统按手机语言解析；键按状态选：`live_run.alert.waiting`、
  `live_run.alert.done`、`live_run.alert.failed`、`live_run.alert.cancelled`、`live_run.alert.uncertain`，五条都进本地化资源、
  四语言齐全（第 10 节）。Android 走加密的 `run_progress`，文字由壳直接写，没有 loc-key。
- 过期时间只有一份：APNs 的 `stale-date`（第 3 节），系统的 `isStale` 由它驱动，`content-state` 不再放 `staleAt`。
  本地路径没有推送，壳按 `LiveRun.stale_at_ms` 画同一行提示。
- 锁屏内容单独一个开关（新增 `NotificationCategory::Task`，默认开）；关掉只写“有任务在跑”。
- App 锁（ADR-0102）开着且锁定时：实时活动照常显示百分比和条，但不显示阶段文字，标题写成“云吞”；数字不算敏感内容（ADR-0101 第 5 节的同一条）。
  锁定标记写进 App Group，实时活动的视图和通知扩展读同一个文件。
- 机主在系统里关掉实时活动、或者 Always-On 只留概要时，如实变少，不补假字。

### 8. 点进去打开哪里、等确认时的动作

- 点实时活动（任何一处）：`wonton://room/<房间 ID>`，壳先盖锁定页，解锁后进那个聊天（ADR-0102 已规定深链要穿过锁屏）。
  链接语法归 [ADR-0114](0114-plugin-agent-platform.md) 第 6 项管，这里只复用 `room` 那一支。
  v1 只保证当前这台服务器；别的服务器的房间先停在切换那一步，多服务器的 `?server=` 后续补。
- `waiting` 时：`relevance-score` 用 100（平时 50）并带提示，灵动岛显示这一条；卡片还在，列表里点了能到卡片。
- `can_answer` 为 true 时（卡片没有表单字段，选项就是核心补的 `approve`/`reject` 这一对），锁屏和通知上给“允许”“拒绝”：
  - iOS：`LiveActivityIntent` 的按钮，系统在后台拉起 App 进程，走 `answer_push_confirmation(server_id, card_id, approve|reject, None)` 同一条回答路径；
  - Android：`NotificationCompat.Action` → 现有的 `PushActionReceiver`；
  - 有自定义选项、要填密钥或设置的卡片只给“打开”，不在锁屏上猜选项的含义（`choice_id` 不往明文里放）。

### 9. 平台约束和降级

- **iOS**：实时活动本身不要付费账号，`NSSupportsLiveActivities` 就能本地开；**推送**（开始、更新、结束）要付费账号和推送能力。
  账号到位前 iOS 只在 App 运行时本地更新，锁屏后停在最后一份内容；文案不承诺“关掉 App 也能看”。
- iOS 17.0～17.1 能本地开始、能被推送更新，但不能被推送**开始**；17.2 起才有 push-to-start。18 起是 `input-push-token`，
  27 起横屏也显示紧凑视图（要按 `isDynamicIslandLimitedInWidth` 收窄）。
- 没有灵动岛的机型是锁屏实时活动；iOS 16.1 以下没有。
- Android：`NotificationCompat.ProgressStyle` 只在 API 36 起有进度条外观，更早的版本退回普通 `setProgress`；
  用户把 Live Update 关了就不再请求提升：实现先查 `NotificationManager.canPostPromotedNotifications()`，关着就不申请，
  设置页给 `Settings.ACTION_MANAGE_APP_PROMOTED_NOTIFICATIONS` 的入口；OEM 还会有自己的额外条件。
- Android 的通知权限被关：进度通知和上面的活动都画不出来，设置页如实指去系统设置；iOS 的实时活动不受通知权限影响。
  Android 没装分发器、又在保持连接时，进程被系统杀掉后通知停在最后一格，打开 App 再继续。
- 全局开关就是第 6 节的 `SettingsEntry::TaskProgress`（按设备）；关掉后不再登记令牌，已有的活动收尾。
- 媒体任务（[ADR-0113](0113-media-transport.md) 的上传、下载、播放会话）不走 `wonton__report_progress`：它们由核心自己持有，
  进度走 `media.task.progress`，锁屏由 0113 第 4 节的 `MediaSessionService`/`MPNowPlayingInfoCenter` 出；
  只有播放会话自己的“等你确认”才借用本节的卡片规则。

### 10. 壳的文字和画法

两端各一份，四语言齐全，按 `scripts/ui_text/groups.py` 分组：`live_run.*`，含 `live_run.alert.*` 的五个提示键
（App 与 widget 扩展都要有，后者给系统解析推送的 `loc-key`）。

| `phase` | 文案（中文示例） |
| --- | --- |
| thinking | 在想办法 |
| reading / editing / deleting / moving | 在读 / 在改 / 在删 / 在整理 |
| searching / command / web | 在找 / 在跑命令 / 在上网 |
| waiting | 等你确认 |
| other、未知 | 在干活 |

`state`：`working` 画转圈或进度条；`waiting` 画醒目的“等你确认”和按钮；`done` 画对勾和用时；`failed` 画错误；
`failed` 且 `end` 是 `cancelled` 写“打断了”、`state` 是 `uncertain` 写“结果不确定”，两种都不画红。

iOS 的呈现按平台指引：锁屏一条，灵动岛紧凑左侧 App 标记、右侧百分比或转圈，最小视图只放百分比，展开视图放阶段、步数、进度条、“打开”。
`isStale` 时一行“进度可能过期”。Android 一条 `setOngoing` + `setOnlyAlertOnce`，标题是 Bot 名（本地画），正文是阶段，
`setWhen(started_at)` + `setUsesChronometer`，API 36 起用 `setShortCriticalText` 放百分比，`setDeleteIntent` 记下被划掉。

### 11. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `runs` 模块（上面的结构体和 `RunPhase`，测试盖未知值落 `Other`、`LiveRun` 的字段往返（含 `end`）、
  `can_answer` 只对标准按钮为 true）；`SettingsEntry::TaskProgress`；`wonton__report_progress` 的参数和错误类别；
  中继的 `POST /v1/live-activity`、`PUT/DELETE /_wonton/push/live` 的 body 与 APNs 载荷（固定 golden 载荷测试，
  断言 `content-state` 里没有选项文字、`cancelled` 落在 `end`；进 `waiting` 和四种结束都带 `alert` 的 `loc-key`、
  `loc-args` 只有数字，平常更新没有 `alert`）。
- **提供方**：家里的核心（聚合、节流、推送、VM）+ 中继（APNs）+ 两端壳（ActivityKit、通知）。
- **消费方**：两端壳。换中继不改核心和壳（只有 HTTP 契约）；换客户端只要实现“拉 `/_wonton/api/runs` 或解 `run_progress`
  → 画列表 → 深链”，核心不认平台。
- 能证明换提供方的契约测试：假 APNs 当中继的下游，断言核心发给中继的请求、中继转出的 APNs 头、载荷和 410 处理都不随后端变化。

### 12. 分步落地

每一步单独合并、单独验收；做不动就停在前面，不留半截。

1. **核心**：`run_progress` 模块（注册表、聚合、节流、`Uncertain` 合成、8 小时/10 分钟收尾）、`wonton__report_progress`、
   `GET /_wonton/api/runs`、`subscribe_live_runs()`。
   验收：行为测试——工具序列到 VM 的迁移、无 `run` 也没有活跃真人轮次的定时上报回 `not_a_turn` 不上锁屏、
   Bot 传话（说话人是 Bot 账号、链代表真人）能上锁屏并按链上的那个人挑设备（主人也认）、链代表是远端时不上、30 秒门槛、
   5 点/10 秒节流与合并、两条都在 `waiting` 时第三次回 `too_many`、只有非 `waiting` 时才挤、离线变不确定、超时结束。
2. **Android 画**：从 VM 画进度通知（36 上 `ProgressStyle`、以下 `setProgress`）、提升请求（先查 `canPostPromotedNotifications`）、
   深链、公开版和隐藏版、设置开关。
   验收：投递假 VM 的截图测试；模拟器上对着开发栈真跑一遍（装、跑长任务、锁屏看、点回聊天）。
3. **Android 推送**：加密载荷 `run_progress`、`resolve_push` 分开消息和进度、410 处理。
   验收：核心契约测试；模拟器装分发器，杀掉 App 后看进度还能动。
4. **iOS 本地**：widget 扩展里的 `ActivityConfiguration`、本地开始/更新/结束、深链、App Group 的锁定标记、授权探测、
   18/17.2/27 的 `#available` 分支。
   验收：固定假 VM 的预览（锁屏、紧凑、最小、展开）；模拟器走查；关掉实时活动后不再开。
5. **iOS 推送和中继**：中继接口、令牌登记、开始/更新/结束的载荷、priority、`stale-date`、`dismissal-date`。
   验收：本机对 APNs 沙箱 curl；模拟器用 `simctl push` 打活动载荷；真机在账号到位后走一遍（开始、更新、结束、410）。
6. **等确认的动作**：活动/通知上标准同意/拒绝的按钮，接 `answer_push_confirmation`；把现有 `OngoingTasks` 的确认通知并进同一份注册表。
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
- 本地开始和 start 推送的竞态去重后仍可能多出一条，壳结束它、主人可能闪一下。
- 自定义选项的卡片在锁屏上只能“打开”，选项再多也不猜；这是为了不把插件写的标签放进明文。
- `GET /_wonton/api/runs` 和 `PUT /_wonton/push/live` 是又一组常驻接口；前者只在有运行时才有内容，没有时不唤醒。
- 明文里的 `room`/`run` 让中继能对上一台设备连续看了哪些任务（次数和时长），对不上具体内容。

## 要主人拍板

1. **明文概要（推荐）还是自加密让中继也看不到？** 推荐明文：只放枚举、百分比和四个不透明 ID，换来实现简单、系统原生呈现全能用；
   自加密要多一套钥匙和编解码，锁屏提示也用不上本地化。
2. **哪些任务上锁屏？** 推荐“人当场发起、跑过 30 秒、每设备最多 2 条”。备选是维持 ADR-0101 的“只在等你确认时”，
   或者由 Bot 自己声明“这是长任务”（安静，但会漏报，Bot 判断错就没有）。
3. **默认开还是默认关？** 推荐默认开、给总开关和类别开关：这是主人自己发起的活，预览一眼就知道在跑什么；怕烦的关一类就行。

## 影响

- **ADR-0003**：通过后要在它里面补一节例外：实时活动的概要是明文，只放枚举、数和随机 ID，文字在壳里；普通消息的推送加密不变。
  本节先记着，本 PR 不改它的正文。
- **ADR-0101**：本 PR 已经在状态行和第 4 节写上到本篇的链接和“扩充中”，范围以本篇为准；§4 的 `runs_waiting()` 由本篇的
  注册表吸收，它原来的“最多 3 条”改成和本篇同一套“每设备 2 条、`waiting` 优先、两条都在等时回 `too_many`”，
  范围也收窄成第 2 节的门槛（定时任务的等待只发普通确认通知，不上锁屏）；
  第 5 节要给 `Task` 类别加锁屏开关，第 1 节的按钮要加实时活动入口。后面两项等它通过后再补。
- **ADR-0113**：媒体任务不并进来，进度和锁屏各走各的；只有播放会话等确认时用本篇的卡片按钮规则。
- **ADR-0114**：收件人按第 4 项的“按人”规则，深链按第 6 项；本篇不重写它们。
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
- Android `NotificationManager.canPostPromotedNotifications()` 与设置入口 `Settings.ACTION_MANAGE_APP_PROMOTED_NOTIFICATIONS`：
  https://developer.android.com/reference/android/app/NotificationManager#canPostPromotedNotifications() 、
  https://developer.android.com/reference/android/provider/Settings#ACTION_MANAGE_APP_PROMOTED_NOTIFICATIONS 。
- Android“Create a progress-centric notification”与 `Notification.ProgressStyle`（API 36）、`NotificationCompat.ProgressStyle`
  （androidx.core 1.17.0，36 以下退回默认样式）：
  https://developer.android.com/develop/ui/compose/notifications/progress-centric 。
- Android 通知提示音一秒最多一次（API 27 起）：
  https://developer.android.com/develop/ui/compose/notifications/create-notification 。
- gorush 已支持 `push_type`、`topic`、`content-state`、`stale-date`、`dismissal-date`、`relevance-score`、`attributes-type`、`attributes`
  （`notify/notification.go`、`notify/notification_apns.go` 的 `GetIOSNotification`，仓库 appleboy/gorush，MIT）。
- 现有代码：`crates/wonton-core/src/push/`（加密推送、登记、`send_confirmation`、`devices_of` 的按人签名）、
  `crates/wonton-core/src/device.rs`（`Device.user`）、`crates/wonton-core/src/mcp/server.rs`（`Caller.sender`/`origin`）、
  `crates/wonton-client/src/chat/pending.rs`（`LONG_WAIT_MS` 和 `ongoing`）、
  `apps/compose/androidApp/src/main/kotlin/app/wonton/android/push/OngoingTasks.kt`、
  `crates/wonton-protocol/src/agent_run.rs`、`apps/ios/project.yml`（已有 widget 扩展、iOS 17.0、App Group）。

## 关联

[ADR-0003](0003-matrix-and-encrypted-push.md)、[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0016](0016-mcp-bus-and-confirmations.md)、
[ADR-0041](0041-app-links-and-shortcuts.md)、[ADR-0043](0043-app-languages.md)、[ADR-0044](0044-bot-profile-and-status-avatars.md)、
[ADR-0061](0061-agent-runtime-and-frontends.md)、[ADR-0082](0082-agent-run-status.md)、[ADR-0101](0101-notifications.md)、
[ADR-0102](0102-app-lock.md)、[ADR-0108](0108-hermes-default-kernel.md)、[ADR-0110](0110-one-hermes-container-per-person.md)、
[ADR-0113](0113-media-transport.md)、[ADR-0114](0114-plugin-agent-platform.md)。
