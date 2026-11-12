# ADR-0100 系统入口：小组件、系统搜索和控制按钮只读核心写的快照，点了经 `wonton://` 进 App

- 状态：已接受（2026-11-12）。范围（桌面小组件、系统搜索、控制按钮）、第一版三个小组件且不做配置、iOS 刷新不到时停在旧数据并写
  “上次更新于”、插件自带的组件留口子、Android 不做快捷设置 Tile，由主人定（2026-11-12）
- 关联：[ADR-0005](0005-native-clients-thick-core.md)（厚核心）、[ADR-0041](0041-app-links-and-shortcuts.md)（`wonton://` 链接和快捷方式）、
  [ADR-0042](0042-multiple-servers.md)（多服务器）、[ADR-0043](0043-app-languages.md)（语言）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)（设计系统）、
  [ADR-0015](0015-small-components-standard-interfaces.md)（组件只做一件事）、[ADR-0003](0003-matrix-and-encrypted-push.md)（推送）

## 背景

主人要在 App 外面也能看到、点到云吞：桌面小组件（iOS WidgetKit、Android Glance）、系统搜索（iOS Spotlight、Android 桌面搜索）、
iOS 的控制中心和锁屏按钮。三类入口画的时候核心都不一定在跑：iOS 的小组件和控制按钮跑在独立的扩展进程里；Android 的小组件由
桌面进程显示，App 进程可能只是被一条广播拉起来，核心没启动。

核对过的平台事实（2026-11-13 核对官方文档）：

| 事实 | 出处 |
| --- | --- |
| iOS 常看的小组件每天约 40–70 次刷新（约 15–60 分钟一次）；App 在前台时调 `reloadTimelines` 不计预算 | [Keeping a widget up to date](https://developer.apple.com/documentation/widgetkit/keeping-a-widget-up-to-date) |
| iOS 26.0 起可以用推送刷新小组件（`WidgetPushHandler`），同样计预算、由系统择机送达 | [WidgetPushHandler](https://developer.apple.com/documentation/widgetkit/widgetpushhandler)、[Updating widgets with WidgetKit push notifications](https://developer.apple.com/documentation/widgetkit/updating-widgets-with-widgetkit-push-notifications) |
| 小组件里 `Text(date, style: .relative)` 显示的相对时间由系统自己走 | [Displaying dynamic dates](https://developer.apple.com/documentation/widgetkit/displaying-dynamic-dates) |
| iOS 18.0 起有控制按钮（`ControlWidgetButton`），放在控制中心、锁屏和操作按钮上，可以把 App 打开到某一处 | [Creating controls](https://developer.apple.com/documentation/widgetkit/creating-controls-to-perform-actions-across-the-system)、[ControlWidgetButton](https://developer.apple.com/documentation/widgetkit/controlwidgetbutton) |
| Android 的小组件在 App 醒着时（前台、或正在处理推送和广播）调 `updateAll` 即时更新；`updatePeriodMillis` 最短 30 分钟，WorkManager 最短 15 分钟 | [Manage and update GlanceAppWidget](https://developer.android.com/develop/ui/compose/glance/glance-app-widget)、[Advanced widgets](https://developer.android.com/develop/ui/views/appwidgets/advanced) |
| Android 每个 Activity 的静态加动态快捷方式总数不超过 `getMaxShortcutCountPerActivity`，多数桌面一次显示 4 个；App 在后台时更新快捷方式受频率限制；用户固定到桌面的快捷方式 App 删不掉 | [Shortcuts](https://developer.android.com/develop/ui/views/launch/shortcuts)、[Manage shortcuts](https://developer.android.com/develop/ui/views/launch/shortcuts/managing-shortcuts) |

## 决策

### 1. 第一版三个小组件，固定默认 Agent，不做配置

| 小组件 | iOS | Android | 显示 | 点了 |
| --- | --- | --- | --- | --- |
| 最近聊天 | `systemMedium`；锁屏 `accessoryInline` 只显示总未读数 | 4×2，可拉高，按高度放几行 | 最近的会话：名字、未读数、最新一条的预览（核心给类别和原文，壳按类别写占位），底下一行“更新于” | 一行进那个会话（`wonton://room/<房间 ID>`），别处进聊天列表 |
| Bot 状态 | `systemSmall`；锁屏 `accessoryRectangular` | 2×1 | 默认 Agent 的名字、在做什么（`BotActivity`，没取到就写“状态未知”）、和它的私聊的未读数、更新时间 | `wonton://agent-chat` |
| 问 Bot | `systemSmall`；锁屏 `accessoryCircular` | 1×1 | 图标和“问 Bot” | `wonton://agent-chat` |

- 不做“选哪个 Bot、哪个会话”的配置：快照里只有一个默认 Agent，配置界面先付成本没有收益。
- 小组件里不直接发话：发话要核心（`Action::AskAgent`），扩展进程里没有核心。点了进私聊，在输入框里接着说。
- 没配对、登录失效、刚换服务器、App 装好后没打开过、快照读不了时，三个小组件都只写一句“打开云吞……”，点了进 App，由 App 接着说。

### 2. 快照：核心写，入口只读

类型在 `wonton-protocol::widget`（`WidgetSnapshot`），带版本号，契约测试锁住 JSON 字段名。内容：账号状态、写下的时刻、
总未读数、最近 8 个会话、默认 Agent、系统搜索用的联系人（有私聊的好友和 Bot）和插件（已装、有网页的），以及 App 的浅色、深色颜色令牌
（入口照着上色；iOS 扩展不链接核心，拿不到外观）。账号状态四种：

| 状态 | 什么时候 | 其余字段 |
| --- | --- | --- |
| `signed_in` | 当前服务器连得上、会话列表已到 | 照实填 |
| `not_paired` | 这台手机没有配对的服务器 | 空 |
| `signed_out` | 设备被吊销、登录失效（`Unauthorized`） | 空 |
| `switching` | 刚换到另一台服务器，那台的数据没到 | 空 |

- 写的是 `ClientCore` 里的一个任务，跟着当前服务器的作用域走，订阅主页、聊天、引导、插件的 ViewModel 和外观，不加 `Action`
  （`Action` 的 UniFFI 元数据已到上限）。壳不触发写入，只订阅“写下了一份”（`subscribe_widget_snapshot`）去刷新入口。
- 登录状态下两秒内最多写一次；变成另外三种状态立刻写，不等。
- 不确定时不写：恢复会话后会话列表还没到、同步没开始时，留着上一份，不把它写成“没有会话”。断线重连时同样留着上一份。
  插件列表还没取到时，插件沿用上一份。
- 写到数据目录下的 `widget/snapshot.json`：先写临时文件再改名，入口读不到半份。iOS 的数据目录在分享进云吞（PR #402）之后就在
  App Group 容器里，扩展读得到；Android 是 App 的私有目录，Glance 的代码在 App 进程里。核心不区分平台。
- 入口读的是 `read_widget_snapshot`（iOS 扩展用同一套字段自己解码）。文件不在是“还没写过”；坏了或版本不认识按读不了处理，原因进日志，
  界面写“打开云吞刷新”，不猜。
- 多服务器（ADR-0042）：只写当前在用的那台，不聚合。
- 隐私：预览原文和通知、聊天缓存一样留在 App 的沙盒里，不另加密。出沙盒的只有系统搜索的条目，只放名字（见第 4 节）。

### 3. 刷新，以及“上次更新于”

- Android：进程里一个跟着核心的协程，每写一份就对三个小组件 `updateAll`。核心在跑（前台、保持连接的服务、推送服务）就即时；
  `updatePeriodMillis` 设 0，不加 WorkManager 定时：核心不在时定时任务也拿不到新数据，只会耗电。底下写绝对时间
  （当天“更新于 14:05”，更早的带日期）：桌面上的小组件不会自己走时，绝对时间一直是对的。
- iOS：App 每写一份调 `WidgetCenter.reloadTimelines`。iOS 26 起再加推送刷新（`WidgetPushHandler`），和 M2 的系统推送一起接。
  iOS 17–25 上 App 被杀后没有立即刷新的路：小组件停在旧数据，写“上次更新于 X 前”（`Text(date, style: .relative)`，系统自己走），
  系统的预算刷新每次重读快照文件，App 再写过就跟上。
- 不把旧数据伪装成新的：不清零未读数，不显示“刚刚”。

### 4. 系统搜索

索引的条目全部来自快照，壳不再推导。只放名字和图标，不放消息和未读数：搜索结果出现在会被截图、被旁人看到的地方。

| | iOS | Android |
| --- | --- | --- |
| 写进系统 | Core Spotlight：快照里的会话、联系人、插件各一条 `CSSearchableItem`，标识里是 `wonton://` 链接 | 动态快捷方式：固定的“新会话”“问 Bot”和底栏插件页之后，剩下的名额放最近的会话（好友和 Bot 的私聊也在其中），长按图标和桌面搜索里看得到 |
| 点了 | `NSUserActivity` 带回标识，解成链接打开 | 快捷方式的 `wonton://` 链接进 App |
| 清掉 | 快照不是 `signed_in` 时全删 | 快照不是 `signed_in` 时，进程里的协程马上撤掉会话的快捷方式；用户固定到桌面的系统不让删，点了进 App 由 App 说明 |

- Android 的快捷方式名额由桌面定（多数桌面显示 4 个），所以 Android 只放最近的会话，不放全部联系人和插件。
- 不用 Android 的 AppSearch：系统搜索读不读它由各家桌面定，动态快捷方式是桌面都认的路，已有基建（ADR-0041）。
- iOS 不用 App Intents 的 `IndexedEntity`：它配合带实体参数的意图，第一版没有这样的意图；Core Spotlight 已经够“搜得到、点得开”。

### 5. 控制按钮（iOS 18 起）

| 按钮 | 做法 | 点了 |
| --- | --- | --- |
| 问 Bot | `ControlWidgetButton`，静态配置 | `OpenIntent` 打开和默认 Agent 的私聊（`wonton://agent-chat`） |
| 打开插件 | `ControlWidgetButton`，参数是插件，候选来自快照的 `plugins` | 打开 `wonton://tab/plugin/<插件>`；没选时打开应用页 |

- 按钮只把人送到 App 里，不在扩展里干活。iOS 17 不提供，不仿造。
- Android 不做快捷设置 Tile：官方建议不用 Tile 打开 App，我们的按钮只做这一件事；Android 上长按图标和桌面搜索的快捷方式已经覆盖。

### 6. 插件自带的组件：留口子

快照的 `plugins` 条目是插件在系统入口里的位置。插件声明的组件数据放进这一条，外层格式不动；核心不为某个插件加专门字段（ADR-0015）。

### 7. 链接

`AppLink` 加 `Room { room_id }`，写成 `wonton://room/<房间 ID>`：房间 ID 按百分号编码（`!` 和 `:` 原样留着，读得出来），解出来不是
`!` 开头、带空白或控制字符的不认。编、解都在核心，表见 ADR-0041。点进来的房间还没同步到时，App 等它出现再打开，和打开 matrix.to 房间链接同一条路。

### 8. iOS 工程

- 一个 WidgetKit 扩展目标（`$(WONTON_BUNDLE_ID).widgets`），小组件和控制按钮都在它的 `WidgetBundle` 里，接 PR #402 的 App Group。
- 扩展不链接 `WontonClient`：它只读一份 JSON，不必带上连网、Matrix 和存储。Swift 那边按快照的字段名解码，字段名由协议的契约测试锁住，
  改了契约测试先失败。
- 主屏小组件的预览文字标 `privacySensitive()`；锁屏的只放未读数和 Bot 状态，不放人名和正文。
- 界面走 ADR-0026/0029 的令牌，颜色用快照里的；文字进 String Catalog，四种语言（ADR-0043）。

## 考虑过的做法

- 扩展进程里跑核心：要连回家、恢复 Matrix 会话，扩展的内存和启动时间都不够，还和 App 抢同一个存储的锁。否。
- 通知扩展写快照：它只知道一条消息，写出来的快照要么缺、要么是拼的。和 M2 的推送一起再看。
- Android 定时刷新（WorkManager）：核心不在时拿不到新数据，只是把停更伪装成“会自己好”。否。
- 锁屏小组件放预览：锁屏谁都看得到。否。
- 不写文件、入口直接订阅核心的 ViewModel：入口进程订阅不到 App 进程里的通道，App 被杀后更不行。文件是唯一跨进程、跨生死的。
- Android 做快捷设置 Tile：主人否（2026-11-12），理由见第 5 节。

## 契约、提供方、消费方

- 契约：`WidgetSnapshot` 的 JSON（`wonton-protocol::widget`，版本号加契约测试）和 `wonton://room/<房间 ID>`（`parse_app_link`）。
- 提供方：`ClientCore` 的快照写入任务，唯一的写者。
- 消费方：Android 的三个 Glance 小组件和动态快捷方式；iOS 的 WidgetKit 扩展（小组件、控制按钮）和 Spotlight。

## 代价与待验证项

- iOS 17–25 上 App 被杀后小组件会旧，最多旧到下一次预算刷新之后 App 再写的时候。主人接受（2026-11-12）。
- 待实测：Android 动态快捷方式在 Pixel 桌面和国内厂商桌面的搜索里是否出现、显示几个；各家桌面对 `updateAll` 有没有节流。
- 待实测：iOS 控制按钮的 `OpenIntent` 和 Spotlight 的 `NSUserActivity` 进 App 后 `AppNavigation` 接得住（下一个 PR 在模拟器上走一遍）；
  锁屏上按钮的行为在真机上看。
- 待实测：iOS 通知扩展里调 `WidgetCenter` 能不能刷新，官方文档没写。
