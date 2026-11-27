# ADR-0121 聊天时间按组显示、未读用右上角胶囊回到最早一条

- 状态：已接受（2026-11-25 主人拍板）。6 个待定问题全照推荐：停在底部即标已读、@ 优先跳、锚点顶到标题栏下闪 1 秒、99+ 且翻不到时提示、30 分钟阈值、进房停在最新加右上角胶囊（原话见 `~/wonton-dev/omp-msgs/1379247142731778-20261125-231858-3298463.md`，截图与要求见同目录 `...-223514-2900553.md`）。
- 修订：[ADR-0109](0109-chat-like-telegram.md) 第 2 节的组间隔、第 3 节的时间、第 9 节的时间和勾的令牌；[ADR-0071](0071-timeline-position.md) 的“进房间停在哪”和“按钮的样子”。
- 关联：[ADR-0118](0118-im-three-layers-push-with-body.md)（本地层、房间不加密）、[ADR-0083](0083-bot-contacts-and-group-chat.md)（@ 的写法）、
  [ADR-0093](0093-side-chats.md)（旁聊的独立时间线）、[ADR-0107](0107-ios-liquid-glass.md)（玻璃浮层）、[ADR-0026](0026-design-system.md)（令牌）、
  [ADR-0043](0043-app-languages.md)（文案）
- 调研与来源：`~/wonton-dev/tasks/chat-unread-jump.html` 汇总了微信、Telegram、iMessage、WhatsApp、Slack、Element X 六家的做法和链接；下面是取舍。

## 用户看到什么

时间不再贴在每条气泡里，而是一组消息只在组首上方写一行；隔 30 分钟以上才另起一行，跨天用日期胶囊。进有未读的群停在最新，
右上角浮一枚玻璃胶囊「↑ 26 条新消息」；点它一下跳到最早那条未读，那条背景闪一下。往上翻过最早未读，胶囊收起，右下角换成
「回到最新 ↓」。没加载到最早未读就先把历史翻出来再跳，翻不到如实说。有人 @ 我时胶囊先写「有人@你 · 26 条新消息」，点它先跳 @ 那条。

## 背景

主人 2026-11-25 在真机上看了微信，定两件事：

1. **时间改成微信的写法**：一组连续消息只在开头显示一次，隔 30 分钟以上再显示；跨天显示日期。ADR-0109 现在写的是 Telegram 的“每条气泡右下角一个时间”，要改。
2. **未读要有微信那样的一下回到最早未读**：进群停在最新（底部），右上角浮胶囊「↑ 26 条新消息」，点了跳回最早那条未读；这是基准，其余家只作对照。

两件事都和现有决策冲突。ADR-0109 第 3 节把时间放在每条气泡里；ADR-0071 的“进房间停在哪”是停在已读位置（第一条未读），不是最新。
本 ADR 修订这两处，保留 ADR-0109 的气泡、勾、跳转栈和 ADR-0071 的“回到最新”按钮，只改时间与落点。

六家的做法（细节和出处见调研页）：

| 产品 | 做法 | 取舍 |
| --- | --- | --- |
| 微信 | 组首时间、5 分钟、跨天日期；右上胶囊「N 条新消息」跳最早未读；会话列表「[有人@我]」；免打扰群「有人@你」跳 @ 那条 | 基准 |
| Telegram | 每条气泡内时间、未读分隔条、进房停在未读分隔条、向下按钮 | 落点不同 |
| iMessage | 偶尔的时间分隔、右滑看单条；停在最新 + 工具栏「回看」跳第一条未读 | 入口在工具栏 |
| WhatsApp | 气泡内时间、未读分隔盒、进房即在未读、向下双箭头 | 落点在未读 |
| Slack | 红「New」线、可配置落点、⌘J 跳最近未读、顶部「N new messages」 | 配置太多 |
| Element X | 气泡内时间、绿色 New 线、进房在最新、Jump to Unread 按钮 | 概念近、细节未稳 |

Matrix 的事实源是现成的：`m.fully_read` 是房间 account data 里的已读标记，`m.read`/`m.read.private` 是回执；`/read_markers` 一次能同时写。
`m.mentions.user_ids` 里有本账号就是 @ 我。matrix-sdk 0.19.1 已经比较公开和私有回执、提供 `num_unread_mentions`，也提供 `fully_read_event_id()`。
SDK 的 `__read_marker` 只能当分隔线的定位参照：它只在已读位置后面还有消息时插入，读完就删；拿它当“有没有未读”的判据会把读完的房间当成未读（ADR-0071 已写明）。

现有代码里，核心已经算了 `TimelineMessage::starts_time_group`（15 分钟），但 `crates/wonton-protocol/src/timeline.rs` 的注释把它标成“已被 run 取代”，两端都没画。
`MessageRun` 的组间隔是 5 分钟、按发送者断。`read_position()` 只取本人最新回执，没取 `m.fully_read`；`mark_as_read` 两个都写。
`TimelineContent::ReadMarker` 已经渲染成“以下是新消息”，两端都有；它由 SDK 的虚拟条目产生。

## 决策

### 1. 时间分组

- **只在组首上方居中写一行时间**，不进气泡。`starts_time_group` 重新启用；时间仍是本机时区格式化，核心只给分组和 `timestamp_ms`。
- 新组条件（任一）：跨本地日历日；与上一条**消息**相隔 ≥ 30 分钟；上一条不是气泡（日期分隔、说明行、`ReadMarker`、旁聊卡片、`TimelineStart`）。
- **发送者变化只断气泡的圆角组，不另起时间组**：微信也只看时间和跨天。换了人但没超 30 分钟，接着同一行时间往下写。
- 跨天处是日期胶囊，**当天第一条的时间并进日期行**（“11 月 25 日 星期二 19:25”），那一条不再另写时间；当天其余组只写“19:25”。
- 一周内写“星期二 19:25”，更早写“11 月 20 日 19:25”；当年不写年份，跨年才写。秒不显示。
- 时间行是独立读屏节点（“时间 19:25”），正文的占位不再包含时间；日期胶囊念“日期 11 月 25 日 星期二”。
- 本地时区变了、或向前翻页后有更早的消息插进来，重算受影响的边界；别的时区用户看到的日期不同是正常的。
- AgentRun 状态行不是气泡：它上面不写时间，但它隔开前后，下一句另起一组。

气泡组（`MessageRun`）的间隔同步从 5 分钟改成 30 分钟：时间边界一致（日期、非气泡、≥ 30 分钟），另加发送者变化断气泡组。这样时间行落在时间组首，圆角组和它同界或更细，不会错位。

### 2. 未读的事实源

- 读位置取**更晚的那个**：房间 account data 的 `m.fully_read`，和本人最新的 `m.read` 或 `m.read.private` 回执。按时间线顺序比；位置不在时间线里时按 `origin_server_ts` 比。
- “有没有未读”= 读位置之后还有可见消息。`num_unread_messages` 只用来在区间没加载时先给条数，不作为唯一判据（免打扰房间可能是 0）。
- 分隔线用会话锚点，不用 SDK 的 `__read_marker` 当唯一来源；SDK 那条在时仍可复用它的事件 ID 定位。
- 写已读时**两个都写**：`m.read` 和 `m.fully_read`，在同一次用户动作里发（能合成一次 `/read_markers` 就合成）：前者给 ADR-0109 的已读勾和别人的回执，后者给多设备同步的已读位置。将来加“隐藏已读”开关时改发 `m.read.private`，读取逻辑不变。
- 没有 `m.fully_read` 也从没发过回执（新账号、刚加入）：按 `num_unread_messages` 从最新往回数；数不到就锚最早一条已加载的消息，锚点之后仍算未读。
- 读位置指向时间线外的事件（表情回应、话题回复，Element 也有这个 bug）：用它的 `origin_server_ts` 找第一条更晚的可见消息；取不到时间戳就按未读数往回数。

### 3. 进房位置和会话锚点

- **进房停在最新一条**，贴着输入框。ADR-0071 的 `anchor` 字段保留，但它的值改成最新一条，用于“落定之前不报 Bottom”；不再锚已读位置。
- 订阅建好的第一批里，核心同时冻结**本会话锚点**：读位置之后的第一条可见条目（气泡或 Agent 状态行），以及它的条数和 @ 列表。锚点只在锚点那条被撤回/被裁掉时顺延到下一条；标记已读不动它，别的设备读了也不动它。
- 锚点条带 `unread_anchor`（气泡在 [`TimelineMessage`](0121-chat-time-groups-and-unread-jump.md)，状态行在 `TimelineContent::AgentRun`）：壳在它上方画“以下是新消息”，并隐藏 SDK 自己的 `ReadMarker`，避免两条、也免得胶囊收掉后线跟着没。锚点之后没有消息了就一起收。
- 本地历史不全、锚点还指不到本地任何消息时，`anchor_item_id` 为空：胶囊照常显示、条数先用 `num_unread_messages`，壳不发 `UnreadAnchorSeen`、不画分隔线；点跳由服务器分页解析出锚点后再补。
- 锚点之后没有消息 = 没有未读，不显示胶囊；读位置比最新还新（全读完）也一样。
- 会话锚点只活在这次进房；杀掉 App 再进，已读位置已经在底部，不会再有胶囊。这就是微信的行为，ADR 里写明，不假装能跨重启找回。

### 4. 胶囊

- 位置：导航栏安全区下 12、右 12，压在列表上；Liquid Glass（ADR-0107），高 28，左右内边距 10，圆角全，上箭头 12 + 文案 13 中粗。命中区至少 44。
- 文案：普通“26 条新消息”；有 @“有人@你 · 26 条新消息”；超过 99 显示“99+ 条新消息”，读屏念全量。
- 只由核心的 `unread_jump` 驱动：`count > 0` 才给；壳不自己数未读、不自己比回执。
- 点击：有 @ 且还有没看过的 @ 时发 `JumpToNextMention`；否则发 `JumpToFirstUnread`。加载中忽略连点。
- 消失：锚点是 `Some` 且任意一部分进入视野（壳报 `UnreadAnchorSeen`）、跳转成功、锚点被删且没有下一条、离开房间。
  回滚不重新出现；但收掉后又有新消息时，按新的未读块重新算锚点再出现，不做成“未读开关”。
- 读屏：整颗是一个按钮，标签“回到最早未读，26 条新消息”；@ 时“有人@你，26 条新消息”。跳转成功后焦点移到锚点那条。
- 减少动态效果：跨三屏以上的跳转直接落位；高亮仍闪一次 1 秒。
- 胶囊和右下角“回到最新”互斥：锚点可见或贴底时只有其中一个。

### 5. 跳到最早未读

`TimelineAction::JumpToFirstUnread { target }`，核心执行：

1. 取会话锚点；没有就按第 2 节重新解析。
2. 锚点已加载：下一批发 `TimelineFocus { item_id: 锚点, highlight: true }`，同时收胶囊。
3. 锚点没加载：`UnreadJump.state = Loading`，向前分页，每页 30 条，最多 30 页；找到读位置那条的下一条就停。
4. 翻到房间开头还没有：`state = Unavailable { reason: NotFound }`；离线或请求失败：`Unavailable { reason: Offline }`；服务器拒绝、解析失败：`Unavailable { reason: Other }`。胶囊都留着，壳按 reason 写提示（“找不到最早未读，可能已被删除”“连不上服务器，暂时回不到最早未读”，Other 用通用句）；再点可重试。
5. 落地：锚点滚到导航栏下沿，分隔线在它上面；气泡背景用 `ratio.jumpHighlight` 闪 1 秒淡出；读屏焦点移到锚点。
6. 跳之前的条目进 `back_to` 栈，第一下“回到最新”回栈顶，再回底部（ADR-0109 第 4 节不变）。
7. 动画/手势期间再来消息不改变锚点；锚点被删则顺延，跳转中锚点消失就落到下一条。

分页途中不改读位置，也不标已读。翻页失败的页面留在缓存里，重试从缓存继续，不从头再来。

`JumpToNextMention` 用同样的分页规则，找的是第一条 `mentions_me` 的消息：`first_mention_item_id` 已知就直接跳，未知就往前分页直到找到；找到后记进“已看过 @”，再找下一条。全都看过、或者一条 @ 都找不到时落回 `JumpToFirstUnread`。

### 6. 已读时机和多设备

- 只在列表贴底（阈值 24 pt）、最新一条可见、且布局稳定后发 `MarkRead`；滚在别处、只看历史、点胶囊都不发。发送消息也算贴底。
- 300 毫秒防抖；同一个位置不重发；`m.read` 和 `m.fully_read` 在同一个动作里发。
- 离线：本地先推进读位置和分隔线，进队列；联网补发。补发失败进 `ChatViewModel::problem`，不静默当成功。
- 别的设备推进了 `m.fully_read`/回执：房间未读角和“回到最新”的数字跟着重算；本次会话的锚点、分隔线、胶囊不动。
- 服务器接受回执后未读数才清零；界面不自己先把房间角标抹成 0。

### 7. @我

- 只认 `m.mentions.user_ids` 里有本账号（回复自动带上的也算）；不从正文按显示名猜（ADR-0083）。
- 未读区间里有这样的消息：胶囊进入“提及态”，优先跳最早一条未看过的 @；看过一条就记进会话的“已看过 @”集合，跳到下一条；没有剩余 @ 了恢复普通态，跳最早未读。
- 房间列表：未读里有 @ 时徽标换成“@”，读屏“N 条未读，其中 X 条提到了你”；没有未读仍按数字。@ 徽标同样不靠颜色单独表意。
- 免打扰群的胶囊和普通群一样有提及态；不另做微信那种“有人@你”第二颗按钮，位置已经够用。
- 锚点前的 @ 不算（已经读过）；锚点后的 @ 被撤回就从集合里去掉。

### 8. 本地层和离线

- 按 ADR-0118：本地 SQLite 先出历史，读位置、`m.fully_read`、已读队列都在本地。进房先按本地锚点画；服务器同步下来更晚的消息就接在后面。
- 离线时跳转只翻本地缓存；锚点在本地缓存里就直接跳，不在就 `Unavailable { reason: Offline }`（“连不上服务器，回不到最早未读”），联网后用户再点一次即可。
- 离线标记已读是本地写，不假装服务器已经收到；房间角标以服务器回执为准，未同步时详情里显示“已读状态还没同步”。
- 本地历史不全（刚装、刚加入、缓存被清）时，锚点可能指不到任何本地消息：停在最新，胶囊按 `num_unread_messages` 显示，点跳再走服务器分页；一条都拉不到就按第 5 节失败处理。

### 9. 核心和壳的分工

核心（Rust，ViewModel）负责：

- `TimelineMessage::mentions_me: bool`（新字段）：这条的 `m.mentions` 包含我。
- `TimelineMessage::unread_anchor: bool`（新字段）：这条是最早未读，壳在它上方画分隔线，并隐藏 SDK 的 `ReadMarker`。
- `TimelineUpdate::unread_jump: Option<UnreadJump>`（新字段），每个批次给当前值：
  `UnreadJump { anchor_item_id: Option<String>, count: u32, mentions: u32, first_mention_item_id: Option<String>, state: UnreadJumpState }`。
  `anchor_item_id` 为空表示锚点还没解析出来（第 3 节）：壳只画胶囊、不发 `UnreadAnchorSeen`，点跳交给核心分页。
  `UnreadJumpState`：`Idle` / `Loading` / `Unavailable { reason: UnreadJumpUnavailable }` / `Other`。
  `UnreadJumpUnavailable`：`NotFound` / `Offline` / `Other`，壳按它选提示文案。
- 分隔线条目、锚点解析、条数与 @ 计数、分页、聚焦、已读、同步、离线队列。

壳负责：

- 按 `starts_time_group` 画时间行，按 `ReadMarker` 画分隔线，按 `unread_jump` 画胶囊。
- 报告 `TimelineScroll::Bottom/Away`；在锚点条目进入视野时发 `UnreadAnchorSeen`。
- 滚动、落位、1 秒高亮、读屏焦点、减少动态效果、文案本地化（时间用 `timestamp_ms` 自己格式化）。
- 不比较发送者、时间、未读、回执；不自己数条数。

新字段全部带默认值、只追加在枚举末尾，旧壳不认识的取值落 `Other`。`check-shell-parity.py` 的白名单只放真正只在一端用的项（本方案两端都用）。

### 10. 视觉规格

| 项 | 值 |
| --- | --- |
| 时间行 | 12/16，`textSecondary`；组首上方 8、下方 2；居中 |
| 日期+时间行 | 沿用日期胶囊的 13 中粗、高 22；文字“11 月 25 日 星期二 19:25” |
| 分隔线 | 已有 `ReadMarker` 样式：两侧细线 + `primary` 文字“以下是新消息”，13 |
| 胶囊 | 高 28、圆角 14、上箭头 12、字 13 中粗、`textPrimary` 压在玻璃上；暗色不额外加深 |
| 高亮 | `ratio.jumpHighlight`（`textPrimary` 12%，ADR-0130），1 秒淡出 |
| 回到最新 | 沿用 ADR-0071 的 44 圆钮和 Material FAB；数字徽标 99+ |
| 无障碍 | 胶囊 44 命中；时间行、分隔线、@ 徽标都有单独标签；不靠颜色单独表意 |

浅色深色都用主题令牌，不新造颜色；胶囊是浮层，按 ADR-0107 用玻璃，不是内容气泡。

### 11. 和现有代码怎么接

- `starts_time_group` 的注释从“已被 run 取代”改成“组首时间”；`MessageRun` 文档的 5 分钟改 30 分钟，`read_mark.rs` 的 `TIME_GROUP_GAP_MS` 改 30 分钟，并加发送者/日期/非气泡边界。
- `ChatRoom::read_position()` 改成 `m.fully_read` 与回执取更晚；新增 `fully_read` 写入（`mark_as_read` 已经写，保持）。
- 未读锚点、分隔线、胶囊状态放在 `ChatTimeline`/`ChatViewModel` 的会话字段里，不写进 SDK；订阅释放就清。
- 会话锚点的分隔线用固定 ID 的 `ReadMarker` 条目插在锚点前；SDK 自己的 `EntryKind::ReadMarker` 在有会话锚点时过滤掉。
- 协议只加字段/变体：`UnreadJump`、`starts_time_group` 保持、`mentions_me`、`unread_mentions`（`RoomRow`）；不删旧字段。
- 不动 Hermes、不动 Tuwunel、不改矩阵服务端；`m.read.private` 只作为读取兼容，首版仍发 `m.read`。

### 12. 实施拆分和验证

1. **protocol / ViewModel 接口**：加 `UnreadJump`、`UnreadJumpState`（含 `Other`）、三个动作、`mentions_me`、`RoomRow::unread_mentions`；`starts_time_group` 语义写进文档。范围只有 `crates/wonton-protocol`。验证：serde/UniFFI 里旧 JSON 缺字段落默认、未知枚举落 `Other`、两端都能编译；不动 UI。
2. **核心**：算时间组和 30 分钟的 run；冻结锚点、计数/@、分隔线、分页跳转、已读（`m.read`+`m.fully_read`）、本地队列、多设备重算。范围 `crates/wonton-matrix`、`crates/wonton-client`、本地存储迁移。验证（行为测试，给定事件序列）：
   - t=0、20 分、31 分、跨天、换发送者、夹说明行：断言 `starts_time_group` 与 `MessageRun`。
   - 读位置在中间：断言锚点=下一条、条数、@ 数；标记已读后锚点不变；滚过锚点后收胶囊；另一设备推进回执不动锚点；锚点删除后顺延。
   - 锚点未知（本地历史不全）：`anchor_item_id` 为 `None`，胶囊仍显示；壳不发 `UnreadAnchorSeen`，点跳才解析出锚点。
   - 跳转：已加载→`focus.highlight`；未加载→分页次数与落点；翻到开头→`Unavailable { reason: NotFound }`；离线→`Unavailable { reason: Offline }`；读位置是表情/话题→时间戳兜底；@ 逐条跳完回普通态。
   - 已读：只在贴底+最新可见时发；防抖；离线入队；成功后未读数清零。
3. **iOS**：胶囊、时间行、日期+时间合并、分隔线、跳转动画/高亮、读屏、减少动态、滚动上报、房间列表 @ 徽标。验证：快照各状态；UI 测试用 fake room 断言点击后 `focus` 和高亮；390×844 浅/深、VoiceOver。
4. **Android**：同一套 Compose/Material，FAB 保留；TalkBack、返回栈。验证：同上的截图测试 + UI 测试；`gradlew test`。

每步单独 PR，前一步合了再下一步；壳没跟上的版本只会少画胶囊，不会坏旧行为。

### 13. 没选的方案

- **进房停在第一条未读**（Telegram/WhatsApp/Element 期望）：读未读更顺，但打开看不到最新，回一句话要先往下滚；主人选了微信。
- **Slack 式可配置落点**：两种都能有，但多一个要解释的设置，首版不做。
- **时间留在每条气泡里**：Telegram 的做法，主人已否；信息重复、气泡脚注拥挤。
- **5 分钟阈值**（微信官方）：家里聊天密度低，5 分钟会把一段对话切得很碎；主人拍 30 分钟。
- **拿 `__read_marker` 当未读判据**：SDK 读完就删，胶囊会跟着消失；已读回执也不该靠虚拟条目。
- **用列表下标当锚点**：编辑、本地回显、向前分页都会让下标漂；用条目/事件 ID。
- **只在本地记未读、不走 Matrix**：多设备不同步；服务器上的已读位置也保不住。
- **进房不标已读、手动清**（Slack 第三档）：和微信不一致，角标会一直挂着，家里人会以为没看。

## 理由

- **照微信做**：主人要的是微信的手感，几亿人用熟；我们的时间从 Telegram 改成微信，落点也照微信，不自己发明第三套。
- **事实源用 Matrix 标准**：`m.fully_read`/回执是协议里的东西，服务器、多设备、别的 Matrix 客户端都认；本地字段只缓存，不另造一套“已读”。
- **核心算、壳画**：分组、锚点、条数、@ 顺序、分页都在 Rust，iOS 和 Android 不会各算各的走样，也能用事件序列做行为测试。
- **锚点会话内冻结**：停在最新就必须先把“从哪儿开始算新”固定住；跟着回执走的话，一进房标已读，锚点就跑到最新，胶囊也就没意义了。
- **分页失败如实说**：不把“没找到”降级成“跳最新”，不假装看过；胶囊留着让用户重试。

## 代价

- **打开即已读**：没看就退出也清角标，只能靠会话里的胶囊找回；杀掉重进就没了。要“留着未读”得另做设置，首版不做。
- **数字可能差一两条**：房间角标是通知计数，胶囊数的是消息；状态事件、话题回复会让两者对不齐。
- **大房间慢**：最早未读在几千条以前时要分页好几轮，点下去到落地可能几秒；这期间胶囊在转圈。
- **每批多算一点**：分组、锚点、@ 都要扫一遍已加载区间；用缓存，只算变化附近。
- **`m.read` 是公开的**：家里服务器上别人能看到你读到哪（ADR-0109 的已读勾也依赖它）；要藏得另做私有回执开关。
- **离线补发前别的设备看不到已读**：本机先清，服务器角标等回执；可能短暂不一致。
- **胶囊会盖住右上角内容**：长图片、顶部的消息会被压住一角，点胶囊可跳走。

## 没实测和待验证

- 微信胶囊的消失、高亮、动画只有一张静态截图和 2017 年一篇开发者描述（`https://juejin.cn/post/6844903533007880199`），没现场核实；上线前装微信并排走一遍。
- 微信“以下是新消息”线没在官方文档找到文字，只有社区截图；文案先用它，并排对照后再改。
- iMessage 的时间间隔、Slack 的“Jump to present”细节、Element 的按钮位置来自二手资料或上游 commit，没装机。
- 390×844 上胶囊和浮动日期会不会撞、长按/双击要不要加、TalkBack/VoiceOver 的念法，都要真机走查。
- 大房间分页的真实耗时、内存、`m.fully_read` 指向时间线外事件的概率，要用合成房间压。
- `m.read.private` 目前只读不写；等“隐藏已读”设置时再验证写路径。

## 对照来源

- 微信：官方时间规则 `https://cs.help.wechat.com/hc/zh-cn/articles/11917596826383`；截图见任务目录；@ 行为 `https://juejin.cn/post/6844903533007880199`
- Telegram：`https://github.com/TelegramMessenger/Telegram-iOS/blob/master/docs/chat/corelist-chat-history-backend.md`、`https://telegram.im/blog/unread-replies-mentions-and-reactions/`
- iMessage：`https://support.apple.com/guide/iphone/keep-track-of-messages-iphe9b48b89e/ios`、`https://www.macworld.com/article/221911/...`
- WhatsApp：`https://www.reddit.com/r/whatsapp/comments/1jyghxc/`、`https://faq.whatsapp.com/521835819370205`
- Slack：`https://slack.com/help/articles/360043037853`、`https://slack.green/en/blog/slack-mark-as-read`、`https://slack.com/help/articles/201374536`
- Element X：`https://github.com/element-hq/element-x-android/issues/6931`、`https://github.com/element-hq/element-x-ios/commit/eae464e`
- Matrix：`https://github.com/matrix-org/matrix-spec/blob/main/content/client-server-api/modules/read_markers.md`、`https://matrix.org/blog/2022/09/29/matrix-v-1-4-release/`
