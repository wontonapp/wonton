# ADR-0103 iPad 分屏、多窗口与 Android 大屏适配

- 状态：已接受（2026-11-13）。方向和取舍由主人定（11-13：宽屏两栏；多窗口用"内容窗口"；插件页在宽屏铺满右栏，是
  [ADR-0095](0095-no-tab-bar.md) 的大屏特例；Android 不做应用内多实例窗口，系统分屏、折叠屏和两栏照做）。

## 背景

主人要 iPad 支持分屏（Split View / Slide Over / Stage Manager）和多窗口（同时开两个会话，或一个会话 + 一个插件页）；
Android 对应大屏/折叠屏与多窗口，有对应能力的一起做。

现状（实测代码，main `7e77084`）：

- **iPad 不受支持**：`TARGETED_DEVICE_FAMILY: "1"`（`apps/ios/project.yml:89`）。iPhone 专属 App 在 iPad 上以 iPhone 兼容模式
  居中整屏运行，不能分屏、不能多窗口。
- **单窗口 SwiftUI**：`WontonApp` 只有一个 `WindowGroup`（`apps/ios/Wonton/WontonApp.swift:18-19`）；整个 App 进程一个
  `ClientCore`，`Launch.start()` 在 `App.init` 里建一次（同文件 :13-15），`ChatModel`、`HomeModel`、`PluginsModel`、
  `AppNavigation` 各一份（:21、:62）。工程开了 `UIApplicationSceneManifest_Generation`（project.yml:87），没有显式
  `UIApplicationSupportsMultipleScenes`，即不支持多窗口。
- **方向被锁**：`FullscreenOrientation`（`apps/ios/Wonton/FullscreenOrientation.swift:9-12`）非全屏视频时只允许竖屏
  （`fullscreen ? .allButUpsideDown : .portrait`）。
- **导航**：main 现状是三页 `TabView`（`apps/ios/Wonton/MainView.swift:124`）+ 底栏 + 覆盖层 + 自绘抽屉 `SidebarContainer`
  （同文件 :336-343）。[ADR-0095](0095-no-tab-bar.md)（PR #379）去底栏、应用整页铺满、页面栈放核心
  （`PagesViewModel { stack, … }`，栈上保活 4 个关掉的页面）。本 ADR 以 #379 之后的形态为基线。
- **页面/会话状态在核心只有一份**：`Scope` 每台服务器一份，挂 `Arc<ChatLink>`/`Arc<TabsLink>`/`Arc<PluginsLink>`
  （`crates/wonton-client/src/scope.rs:34-57`）；各 ViewModel 的 `watch::Sender` 集中在 `Views`（同文件 :62-73，经
  `Views::of(scope)` 取）。接收端可以多份（多订阅都收得到），但**状态本身（会话列表、页面栈、选中项）每核心一份**。
- **房间时间线同一时间只开一个**：`ChatLink.open_room` 是单槽 `Mutex<Option<…>>`（`crates/wonton-client/src/chat/mod.rs:108`），
  `subscribe_timeline` 的契约写明"同一时间只打开一个房间，打开另一个房间后前一个的订阅不再更新"
  （`crates/wonton-client/src/client/mod.rs:210-212`）。但 Matrix 层支持并发全量同步：`MatrixSession::watch_room`
  把房间加进被 watch 集合，"一直跟着一个已加入的房间：不管打开着哪个房间，都同步它的完整时间线"
  （`crates/wonton-matrix/src/rooms.rs:332-341`）。`TimelineSubscription` 是每次订阅一个实例，滚动位置（`set_scroll`）
  按订阅记（`crates/wonton-client/src/chat/timeline.rs:28`、:31、:90）——同一房间两个订阅、两个滚动位置在代码形状上是可能的，未测。
- **草稿按房间存核心**，跨窗口天然共享（`crates/wonton-client/src/client/mod.rs:226-229`，`draft(target)`）。
- **通知**：iOS 没有通知点击路由（ADR-0095"深链"一节自认）；Android 通知带 `EXTRA_ROOM`
  （`apps/compose/androidApp/src/main/kotlin/app/wonton/android/push/PushNotifications.kt:146`，常量在 `IncomingRequests.kt:74`）
  经 `ServerHost.kt:25` 的 `requestedRoom: RoomRequest?` 路由进会话（`MainActivity.kt:75` → `WontonApp.kt:78-84`）。
- **Android**：`MainActivity` 单实例、`launchMode="singleTop"`，`configChanges` 含
  `orientation|screenSize|screenLayout|smallestScreenSize|keyboardHidden|uiMode`（不重建 Activity）；manifest 没写
  `resizeableActivity`（默认 true，系统分屏已可用）；minSdk 29、targetSdk 36
  （`apps/compose/gradle/libs.versions.toml:26-27`）；全仓没有用过 `WindowSizeClass` / `androidx.window`。壳模型
  `TabsState`、`ChatNavigation` 等 `remember(core)` 于 Activity 组合内
  （`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/WontonApp.kt:65-74`）。
- **数据目录**：PR #402（[ADR-0097](0097-share-in.md)）把 iOS 数据目录搬进 App Group 容器。多窗口同一进程共享这一个
  `ClientCore`，与此正交；扩展进程（分享、通知）不参与多窗口。

核心没有窗口概念：`wonton-protocol` 和 `wonton-client` 里没有任何窗口/场景类型；页面栈（#379 后 `PagesViewModel`）每作用域一份。
多窗口的一切都要新增。

## 决策

### 1. 打开 iPad 支持

- `TARGETED_DEVICE_FAMILY` 改 `"1,2"`；`UISupportedInterfaceOrientations` 增加 iPad 方向：除 `PortraitUpsideDown` 外全方向。
- `FullscreenOrientation` 的竖屏锁定收窄到 iPhone（或 compact 宽度）为止；iPad 上不锁。
- Info.plist 显式声明 `UIApplicationSupportsMultipleScenes`：多窗口一节要开就写 `true`；只做宽屏不做多窗口时不写
  （Generation 生成的默认是不支持）。
- 界面文字沿用 [ADR-0043](0043-app-languages.md) 的四语言壳资源，无新增来源。

### 2. 宽屏布局：两栏（宽度自适应）

- 聊天根页在 regular 宽度（iPad 全屏、竖屏大多数分屏比例）用 `NavigationSplitView`：
  - **左列**：现在的聊天列表，顶部一条入口行（新聊天、我的应用、商店、设置）。#379 的侧边栏抽屉在 regular 宽度下不再出现，
    它的组（我的应用）作为入口行的展开页或列表下方的分组进入（细节在实现 PR 定，两个摆法都不改数据模型）。
  - **右列（detail）**：当前房间；从我的应用/全部应用打开插件页时，插件页铺满右列（胶囊贴右列右上角，左列照常可点）。
    这是 [ADR-0095](0095-no-tab-bar.md) "插件页整页铺满"的大屏特例：手机上铺满整屏，iPad 宽屏铺满右列。
- compact 宽度（iPhone、iPad 竖屏三分之二以下分屏、Slide Over）：完全沿用 #379 的单列形态（列表 → 房间/整页插件页 + 胶囊），
  抽屉回归。判定用 `horizontalSizeClass`，iOS 17 没有 WindowSizeClass API。
- 两栏下"当前房间"从左列选中推导，仍是每窗口状态（见第 4 节）。

### 3. 多窗口：内容窗口

- 主窗口 = 完整 App（宽屏两栏/窄屏单列）。内容窗口 = 只显示一个目标的表面：
  - `WindowGroup(for: DetachedSurface.self)`：目标是房间（时间线 + 输入栏，完整可交互）或插件页（整页 WebView + 胶囊）。
  - 打开入口：会话行与插件页行长按菜单"在新窗口打开"（`openWindow(value:)`，Android 不做，见第 7 节）；
    会话拖拽出屏开窗是加分项，待验证后再做。
  - 内容窗口的退出：系统关窗手势即可，胶囊上的"关闭"只关插件页本身，不关窗口。
- **不做全克隆窗口**（Element X 式每窗口一套完整导航）：那要求核心给每个窗口一份页面栈（`PagesViewModel` 按 window id 分键）、
  `AppNavigation` 按窗口路由，核心要引入"窗口"这个概念并贯穿深链、恢复、面板；当前用例（两个会话 / 会话 + 插件页）
  用内容窗口即可覆盖。
- **主窗口重复开的风险**：iPadOS 的多任务菜单允许用户对主 `WindowGroup` 再开一个实例，`defaultLaunchBehavior` 在 iPadOS 不可用
  （macOS 15 限定）。第二个主窗口会渲染 `MainView` 克隆、与主窗口共享核心里的页面栈和选中项——两边开同一个页/关页会互相看见。
  第一版如实接受这个行为（不伪装成独立窗口），写进"待验证"观察；内容窗口的 `WindowGroup(for:)` 不受影响。

### 4. 状态归属（每窗口 vs 全局）

| 归属 | 内容 | 理由 |
| --- | --- | --- |
| 每窗口（壳内） | 当前房间（`ChatModel.path` 同类状态）、列表滚动位置、内容窗口的房间、插件页 WebView 实例与胶囊、侧边栏开合、输入框焦点 | SwiftUI 每窗口 `@State` 独立是系统行为（WindowGroup 文档）；这些本来就是壳侧状态 |
| 每窗口（核心侧，新） | 时间线订阅的滚动回执（`TimelineSubscription` 已按订阅记）；内容窗口插件页的加载状态（见第 5 节） | 订阅对象本来就是每次一份 |
| 全局（核心现有） | 连接与重连（`HomeLink`）、会话列表（`ChatViewModel`）、服务器作用域、我的应用布局（`app.wonton.layout`，`crates/wonton-matrix/src/panels.rs:20`）、房间草稿与回复、已读回执、插件列表与权限、Bot 活动、外观、文件传输与媒体缓存、推送登记、引导 | 这些是"这一台设备"的状态，跨窗口一致是产品语义；watch 多接收端已支持 |

明确不共享的取舍：两个窗口看同一房间时，各自滚动位置独立、互不同步（Telegram 桌面版同款行为）；草稿和已读是房间级全局，
两边一致。发送等操作按房间路由，无窗口歧义。

时间线注册表保留所有存活订阅，同目标共用 Matrix 时间线（发送队列、分页和回执），各订阅独立维护滚动和消息菜单。
Main/Thread focus 的房间 watch 按引用计数持有；取消、释放或切换服务器后释放，不替换其他房间的同步名单。
房间面板按 room ID 保存和订阅，桥、权限、重载、选中页均按对应房间查找，不依赖最后打开的房间。

### 5. 内容窗口的插件页

- 页面内容的解析走**新的 `ClientCore` 方法**（如 `page_view(tab: TabRef) -> TabPage`，命名实现时定），复用
  `tabs/view.rs` 里现成的 `build_page`/`compose_page`（`crates/wonton-client/src/tabs/view.rs:190`、:212）逻辑；
  不走全局页面栈，也就不改变主窗口的栈。不加 `Action` 变体（uniffi 元数据已贴着上限，加变体编不过：
  `crates/wonton-client/src/client/tabs.rs:100-101`）。
- 同一插件页可以同时出现在主窗口栈和内容窗口，各自一份 WebView 实例；核心里按 `TabRef` 记的 `TabState`（加载/失败）是页面级的，
  两个实例会互相看见对方的加载态——第一版如实接受（同一页面的状态一致是诚实显示），不为内容窗口再开一套状态键。
  若实测不可接受，再给 `TabState` 加实例键（改动限制在 `tabs` 模块内）。
- 桥（`tab_document_script`/`tab_handle_message`，`crates/wonton-client/src/client/tabs.rs:13`、:27）按 `PlacedTab` 寻址
  （定义在 `crates/wonton-protocol/src/panel.rs:138`，取值 `Bar`/`Panel`），内容窗口需要一个新的 `PlacedTab` 取值或参数
  （实现 PR 定），桥协议本身不改。

### 6. 窗口状态恢复与生命周期

- 内容窗口靠 `WindowGroup(for:)` 的系统恢复拿回目标值（WWDC22 会话 10061）；主窗口恢复到根页（恢复整条页面栈不做，栈在核心，
  跨进程恢复要另存，价值低）。
- 每窗口会话滚动锚点以房间 ID 为键存于 `SceneStorage`，保存实际可见条目的 ID，不存全局像素偏移；恢复时等时间线加载，
  必要时向前分页直到锚点出现。已到历史开头仍不存在时，使用核心给出的已读落点。聊天列表视口是否恢复，以模拟器实测为准。
- `scenePhase`：现在 `home.send(.appForegrounded/.appBackgrounded)` 挂在主 `WindowGroup` 内容上
  （`apps/ios/Wonton/WontonApp.swift:41-46`）。改成监听 `NotificationCenter` 的
  `UIApplication.didBecomeActive/willResignActiveNotification`（应用级），多窗口下任一窗口前台就算前台，全部后台才算后台——
  语义与现在一致，不按窗口数误报。
- 通知（[ADR-0003](0003-matrix-and-encrypted-push.md)、[ADR-0101](0101-notifications.md)）不受影响：推送登记、解密都是应用级；
  iOS 通知点击仍无路由（现状如实保留），Android 的 `EXTRA_ROOM` 路由进主窗口（`RoomRequest` 只在 ServerApp 组合里消费，
  多窗口不会多消费一份）。
- App Intents（[ADR-0041](0041-app-links-and-shortcuts.md)）：共享 `AppNavigation` 单例，落主窗口；内容窗口不接深链
  （`onOpenURL` 只在主窗口视图上注册）。

### 7. Android 大屏

- **系统多窗口照常工作**：`resizeableActivity` 保持默认 true，不写 `false`；分屏（API 24+）、折叠屏、大屏桌面窗口化下 App
  都会被系统拉成任意尺寸的窗口，两栏/单列随宽度切换，这就是 Android 的"分屏"答案。
- **应用内大屏布局**：`ChatScreen` 在 expanded 宽度换两栏 list-detail（官方规范布局），用 `androidx.window` 的
  `WindowSizeClass` + `androidx.compose.material3.adaptive` 的 `NavigableListDetailPaneScaffold`；插件页铺满 detail 列。
  依赖 `androidx.window`（Apache-2.0）与 material3-adaptive（Apache-2.0）。
- **应用内多实例窗口不做**：官方支持"多实例"，但那要求每 Activity 实例一套导航状态，和 iOS 全克隆同价；"同时开两个会话"
  在 Android 上由两栏同屏覆盖。
- `configChanges` 现有声明（含尺寸类）保留：折叠/分屏时 Activity 不重建，Compose 按新宽度重组，壳模型（`remember(core)`）不丢。

## 理由

- **内容窗口是"两个会话 / 会话 + 插件页"的最短路径**：核心只动两处（订阅钉房间、页面解析方法），窗口概念不进核心；
  全克隆是 Element X 为"每个窗口都是完整 App"付的价，第一版不需要。
- **两栏是行业收敛形态**：三个参考 App 全是"列表 + 详情"两栏起步，Telegram/Slack 收起态也只是把侧边栏变窄，没有第三栏；
  Slack 的"两个会话并排"用应用内 split view，对应我们的内容窗口。两栏与 Android 的 list-detail 规范布局同构，两端代码形状一致。
- **失败即停**：主窗口重复开时页面栈共享是系统行为造成的真实状态，第一版如实让它发生（两个窗口显示同一份导航），
  不在壳里伪造独立栈；记入待验证，若不可接受再升级到全克隆。
- **厚核心不动摇**（ADR-0005）：窗口是壳的 UI 概念，核心只暴露"多订阅一条时间线"和"解析一个页面"两个能力，
  不出现 window id 穿透业务接口的样子。

## 没选的做法

- **三栏布局**（侧边栏面板 + 会话列表 + 详情，Element Web 桌面式）：入口和列表分层更像桌面 IM，但入口组（我的应用）要同时
  维护"抽屉里"和"独立栏里"两种容器，两端布局代码约翻倍；iPad mini 竖屏塞不下三栏，还得做收起态，等于两套都写。
- **全克隆窗口（Element X 式）**：每窗口完整导航。没选：核心要加窗口维度（页面栈、路由、恢复、深链分流全要 window id），
  第一版收益（多开一整个 App）大于两个内容窗口。
- **不做多窗口，只做宽屏两栏**：最小实现。没选：主人明确要"同时开两个会话或一个会话 + 一个插件页"；内容窗口做完后这一条自动满足。
- **插件页继续整页铺满窗口**：左列被盖，开个插件就丢掉聊天列表可见性，iPad 大屏优势没用上。铺满右栏（第 2 节），
  手机行为不变。
- **会话拖拽出屏开窗**：iPadOS 支持拖拽场景（`NSItemProvider` + `UIDropInteraction` 到屏幕边缘），体验最好但手势/边缘判定
  实现和验收成本高，放加分项。
- **Android 应用内多实例窗口**：同全克隆理由，没做；折叠屏/分屏下两栏已覆盖日常，"两个会话并排 + 各自独立输入栏"这种用法
  需要多实例，在 iPad 上由内容窗口满足。
- **iPad 上继续锁竖屏**：不行，iPad 分屏下横竖屏由用户决定，锁竖屏会被系统拒绝分屏。

## 代价与待验证项

- **核心两处改动要过行为测试**：`subscribe_timeline` 改成"订阅存活即钉房间"后，关页/关窗要能摘掉 watch（防泄漏）；
  页面解析方法要过插件未就绪、页面被删、`opensInBrowser` 等分支，错误类别照 [ADR-0011](0011-security-model.md) 分。
- **待验证（模拟器/真机）**：
  1. 同一房间两条 `TimelineSubscription` 并发（代码形状支持，无测试）。
  2. 主窗口被系统再开一个实例时共享页面栈的实际表现（是否可接受，或逼出全克隆）。
  3. `WindowGroup(for:)` 场景恢复在杀进程后能否带回内容窗口目标值。
  4. `NavigationSplitView` 与 #379 的整页铺满、左边缘返回手势在 iPad 上的组合（`FullscreenOrientation` 收窄后手势 delegate
     仍要工作）。
  5. 内容窗口 WebView 与主窗口同页 WebView 的内存（iPad 上两个 Immich 全页）。
- **Android 两栏是新增布局面**：截图测试参考图要按新宽度重录（PR 打标签由 CI 重录）；时间线贴底、输入栏等已验证的行为
  要在两栏下重走一遍。
- 噪点/材质（[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)）在 iPad 大表面上帧率沿用既有"待验证"结论，
  不新增。

## 查证来源（2026-11-13）

- Element X iOS（develop 分支）：`UIApplicationSceneManifest` 里 `UIApplicationSupportsMultipleScenes: true`
  （`ElementX/SupportingFiles/target.yml` 约 67 行，Info.plist 同键）；CHANGES.md 的 #5319（mac/iPadOS 多窗口）、#5528（多窗口修复）、
  #5964（iPad 自定义 TabRail）、#6180（split view 修复）；生成的工程文件里 `TARGETED_DEVICE_FAMILY = "1,2"`（yml 未显式写，
  是 XcodeGen 对 iOS app target 的默认值落盘，22 处）。
- SwiftUI：`WindowGroup`（iPadOS 多窗口要 Info.plist `UIApplicationSupportsMultipleScenes: true`；每个窗口的 `@State`/`@StateObject`
  独立；`WindowGroup(for:)` 的值由系统持久化用于恢复）、`Window` 场景仅 macOS 13+ 和 visionOS 26、`defaultLaunchBehavior(_:)`
  仅 macOS 15+（不支持多窗口的平台上被忽略）；WWDC22 会话 10061（`WindowGroup(for:)` 的数据传递与场景恢复，恢复值要求 `Codable`）。
- Slack：slack.design "How we redesigned Slack for the iPad"（侧栏 + 内容区、size classes；semi-compact 自动隐藏侧栏）；
  帮助中心 "Use split view in Slack"（并排开两个会话或文件）。
- Telegram-iOS：issue #2199（iPad mini 收起态头像栏）、#2178（平板可用性长期问题；注意两个 issue 都是 2026 年新开的，
  社区热度低，只作行为参考）。
- Android：多窗口（分屏 API 24+；`resizeableActivity="false"` 在非大屏上让系统永远全屏，Android 12+ 大屏和 Android 16
  sw≥600dp 上系统仍会以兼容模式/强制多窗口）；多实例（官方："应用可在独立窗口启动自身的多个实例"）；规范布局 list-detail 与
  `NavigableListDetailPaneScaffold`（`androidx.compose.material3.adaptive:navigation`，≥1.1.0-beta01）。

## 关联

[ADR-0005](0005-native-clients-thick-core.md)（厚核心，窗口是壳概念）、[ADR-0015](0015-small-components-standard-interfaces.md)
（组件接口）、[ADR-0033](0033-plugin-tabs.md)/[ADR-0095](0095-no-tab-bar.md)（插件页与页面栈）、
[ADR-0041](0041-app-links-and-shortcuts.md)（链接与快捷指令，落主窗口）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)
（设计系统）、[ADR-0003](0003-matrix-and-encrypted-push.md)（推送，应用级不受影响）、[ADR-0042](0042-multiple-servers.md)（服务器作用域）、
[ADR-0052](0052-chat-pull-panel.md)（面板）、[ADR-0097](0097-share-in.md)（App Group 数据目录，与多窗口正交）、
[ADR-0101](0101-notifications.md)（通知）。
