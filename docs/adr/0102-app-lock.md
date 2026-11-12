# ADR-0102 App 锁：生物识别 / 设备锁屏，和恢复密钥验证统一成一套

- 状态：已接受（2026-11-13）。取舍由主人定（11-13：后台宽限档位 立即 / 1 / 5 / 15 分钟、默认**立即**；开着 App 锁后系统锁屏
  被关掉时停在锁定页、指引去系统设置重新设上；锁定前已经弹出的通知不追溯处理；不加 App 自己的 PIN）。
- 前置：PR #405（看恢复密钥前先过系统验证）先合，本 ADR 以它为基础。

## 结论

1. **锁什么、什么时候锁**：启动必锁 + 退到后台超过所选档位才重新要求解锁。锁定时整个 App 盖一张锁定页，多任务切换器里不露内容
   （iOS 隐私遮罩、Android `FLAG_SECURE`）。
2. **谁存、谁判定**：开关和超时档位存核心（本机文件，同 `appearance.json` / `push-method.json` 模式），锁定状态由核心的状态机
   判定（厚核心，行为测试在核心）；壳只做三件事：探测设备有没有锁屏、调系统验证（`LAContext` / `BiometricPrompt`）、画锁定页。
   新能力用 `ClientCore` 方法，不加 `Action` 变体；前后台沿用已有的 `Action::AppForegrounded` / `Action::AppBackgrounded`。
3. **锁着就不露内容**（与 [ADR-0101](0101-notifications.md) 的锁屏隐藏同一方向）：锁定期间到达的通知走 Android 已有的 hidden
   版本；云吞没有小组件，规则先写死：App 锁开着且锁定时，小组件不显示任何消息内容，只显示"已锁定"或占位，数据面由核心同一
   状态驱动。分享进云吞写入收件箱不需要验证，打开 App 看到分享页之前先解锁。
4. **没设锁屏的设备开不了 App 锁**：设置页如实说明原因，不假装锁上（失败即停）。这与 PR #405 的"恢复密钥无锁屏直接显示"是
   同一机制、不同策略，理由见决策 6。
5. **恢复密钥的验证并进同一套机制**：每端一个系统验证小组件，恢复密钥和 App 锁共用；策略分开（见决策 6）。核心不新增恢复密钥接口。

## 背景

App 没有任何锁。别人拿起一部已解锁的手机，打开云吞就能看全部聊天、恢复密钥和插件。恢复密钥这条路上已经有一次先例：
PR #405 在两端给"查看恢复密钥"加了系统验证——Android 用 androidx.biometric 的 `BIOMETRIC_STRONG | DEVICE_CREDENTIAL`
（`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/RecoveryKeyAuth.kt`），iOS 用
`LAContext.evaluatePolicy(.deviceOwnerAuthentication)`（`apps/ios/Wonton/ChatBackupView.swift`），设备没设锁屏时不拦、直接显示。

App 锁要复用这套验证，并回答：什么时候锁、设置存哪谁判定、和通知/小组件/分享扩展什么关系、没锁屏怎么办。

**已有的地基**（都能直接用，不用新开接口）：

- 前后台信号已经到核心：`Action::AppForegrounded` / `AppBackgrounded`（`crates/wonton-protocol/src/action.rs:95-98`）。
  iOS 由 `apps/ios/Wonton/WontonApp.swift` 的 `scenePhase` 发出（:41-46），Android 由
  `apps/compose/androidApp/src/main/kotlin/app/wonton/android/MainActivity.kt` 的 `onResume` / `onStop` 发出。
- 本机设置文件的成熟模式：`appearance.json`（`crates/wonton-client/src/appearance/mod.rs`）、`push-method.json`
  （`crates/wonton-client/src/push_method.rs`，落盘成功才发布新值）。
- 设置首页由核心的 `SettingsViewModel.sections[].entries[]` 给出，不认识的入口落 `SettingsEntry::Other` 被旧壳忽略
  （`crates/wonton-protocol/src/settings.rs:33-88`）——新旧 App 分开升级时安全。
- Android 通知已经有"锁屏隐藏敏感内容"的 hidden 版本：`PushNotifications.kt` 里 `setVisibility(VISIBILITY_PRIVATE)` +
  `setPublicVersion(hidden)`（`apps/compose/androidApp/src/main/kotlin/app/wonton/android/push/PushNotifications.kt:81-82`）。
- Android 屏幕防窥有先例：密钥输入页用 `FLAG_SECURE`
  （`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/SecretInputs.kt:44-52`）。
- 小组件不存在（全仓 grep 无 `WidgetKit` / `GlanceAppWidget`）。"锁着就不露内容"的规则先写在本篇，小组件的 ADR 定稿时带上。

**业内参考**（行为参考，代码按许可证不同只看不抄；Element X 是 AGPL，ADR-0003 已定为不使用）：

| 产品 | 做法 | 出处 |
| --- | --- | --- |
| Signal（Android） | 设置里"Screen Lock"开关 + 超时秒数；锁定时清掉内存里的主密钥，重开要过生物识别 | `ScreenLockSettingsRepository.kt`（`screenLockEnabled`、`screenLockTimeout`、`canAuthenticate`）；`KeyCachingService.java`（主密钥清掉即视为 locked） |
| Element X（Android） | PIN（可选）+ 生物识别；进程启动即锁；后台有 2 分钟宽限（`GRACE_PERIOD`）；连错 3 次 PIN 登出 | `LockScreenLockState.kt`（只有 `Unlocked`/`Locked` 两态）；`DefaultLockScreenService.kt`（init 即 `lockIfNeeded(Duration.ZERO)`，退后台按宽限期上锁）；`LockScreenConfig.kt` |
| Element X（iOS） | App 锁独立成 Screen 组：`AppLockScreen`（PIN 键盘）、`AppLockSetupPINScreen`、`AppLockSetupBiometricsScreen`，由 `AppLockFlowCoordinator` 串起来 | `ElementX/Sources/Screens/AppLock/…`、`ElementX/Sources/FlowCoordinators/AppLockFlowCoordinator.swift` |
| Telegram（Android） | 自设密码锁（Passcode），自动锁定档位 关/1 分/5 分/1 时/5 时（`SharedConfig.autoLockIn` = 0/60/300/3600/18000 秒） | `PasscodeActivity.java`、`SharedConfig.java` |

**数据边界（如实写）**：App 锁是"人不在场时挡一眼"的门，不是静态数据加密。手机被 root / 越狱、被 ADB 备份抽取时它挡不住；
那类威胁归系统的设备加密和密钥管理管。

## 决策

### 1. 什么时候要求解锁

- **启动必锁**：App 锁开着时，每次进程冷启动都是锁定的（无宽限）。Element X 的锁定服务在 init 时就
  `lockIfNeeded(Duration.ZERO)`，Signal 的 Screen Lock 也是每次打开都要验证；启动必锁免去持久化"上次活跃时间"，状态最少。
- **后台宽限**：进程活着退到后台时起算，超过所选档位即锁定；宽限期内回前台不要求验证。**档位 立即 / 1 / 5 / 15 分钟，
  默认「立即」**（主人 11-13 定）：默认下切换器、通知中心里都不露内容；"回个消息就要重验"的人自己调档。
- 判定只用已经存在的两个信号：核心收到 `AppBackgrounded` 记下时刻，收到 `AppForegrounded` 时比较档位；宽限期内不锁，
  超了置为锁定。进程被系统杀掉再冷启动，回到"启动必锁"。

### 2. 设置存在哪、谁判定：核心，壳只验证

新增客户端核心模块 `app_lock.rs`（`crates/wonton-client/src/`），形态照抄 `PushMethodLink`：

- **存储**：本机文件 `app-lock.json`，内容只有 `{ enabled, timeout }`（枚举，不是裸秒数）。文件读坏了按"失败即停"不静默回退：
  报原因，`enabled` 视为关，设置页能重设。
- **状态机**：`{ enabled, timeout, locked }` 经 `watch` 通道发布，订阅形态同现有 ViewModel（`subscribe_app_lock()`）。
  `locked` 的迁移动作只有三个：进程启动（enabled → locked）、`AppBackgrounded` 后超时（enabled → locked）、
  `unlock_app()`（→ unlocked）。行为测试写在核心：给定操作序列断言状态，不测实现。
- **接口**（新 `ClientCore` 方法，**不加 `Action` 变体**，同 [ADR-0097](0097-share-in.md)（PR #402）分享扩展的先例）：
  - `app_lock() -> AppLockViewModel`、`subscribe_app_lock()`：壳首帧和后续渲染用；
  - `set_app_lock(enabled, timeout) -> Result<(), CoreError>`：落盘成功才发布，失败把原因给壳；
  - `unlock_app()`：壳的系统验证成功后调用；幂等，核心不关心验证是怎么过的（验证是壳的事，核心不重复验证、也不存验证结果）。
- **协议**：`wonton-protocol` 加 `AppLockTimeout`（枚举）、`AppLockViewModel`，`SettingsEntry::AppLock { enabled, timeout }`。
  设置入口放 [ADR-0028](0028-settings-page.md) 第三组（外观、语言、底栏、通知）那一行附近。旧壳不认识落 `Other`，不显示——已有的
  向前兼容机制直接覆盖。

**能力探测在壳，理由**：只有平台知道设备有没有录指纹 / 面容 / 锁屏密码。核心不存"设备支持生物识别"这种会过时的缓存，
每次进设置页由壳现查（Android `BiometricManager.canAuthenticate(…)`，iOS `LAContext.canEvaluatePolicy(.deviceOwnerAuthentication)`）。

### 3. 锁着时界面显示什么

- **锁定页**：居中 App 图标 + 名字 + "解锁"按钮（按 ADR-0026/0029 的设计系统画，两端同一布局；文案走四语言壳资源，键
  `applock.*`）。锁定页出现时自动弹一次系统验证，取消或失败后停在锁定页可再点；验证成功调 `unlock_app()`，遮罩撤掉，
  回到原来的页面（通知点进来的深链、`wonton://` 链接都还在，解锁后照常走）。
- **多任务切换器 / 最近任务**：
  - iOS：壳在锁定或退到后台时（`scenePhase != .active`）盖全屏遮罩，切换器快照是遮罩。注意核心只有在 `phase == .background`
    时才收到 `AppBackgrounded`，进切换器是 `.inactive`（Apple 文档把进切换器和进后台列为不同状态）——所以遮罩的显示条件必须包含
    壳本地的 `scenePhase`，不能只等核心的锁定状态，否则进切换器的瞬间快照还是明文。
  - Android：锁定期间 `FLAG_SECURE`（系统文档写的是阻止屏幕截图和非安全显示，实际效果包括最近任务缩略图变黑，
    同 `SecretInputs.kt` 的先例）。
  - **如实写明的边界**：宽限期内退到后台，切换器/最近任务里短暂可见内容（Element X 的 2 分钟宽限同样如此；默认档位"立即"
    下窗口极小）。API 33+ 有不改截屏行为的 `Activity.setRecentsScreenshotEnabled(false)` 可以把缩略图换成图标，运行时可切换性
    待实测，实现时真机验证，不行就接受这个边界。

### 4. 和通知、小组件、分享扩展、快捷方式的关系

- **通知**：锁定期间到达的通知不露内容。核心在锁定状态下生成的通知（保持连接的本地通知路径，
  `crates/wonton-client/src/chat/background.rs`）带"藏内容"标记，壳照 ADR-0003 的 hidden 版本画（标题 App 名、正文"有新消息"）。
  **已弹出的通知不追溯撤回**（主人 11-13 定）："通知自己消失"容易被当成数据丢失，撤回还要两端各写一套通知管理。
  iOS 的推送链路以 Apple 付费开发者账号为前提（[ADR-0101](0101-notifications.md) 的实施顺序）：通知扩展和主 App 是两个进程，
  扩展不能问核心——PR #402 把数据目录搬进 App Group 容器之后，核心把锁定标记写进组容器、扩展发通知前读它，是顺路的做法，
  随 ADR-0101 的 NSE 实现时验证。
- **小组件**：云吞没有小组件。规则先写死：App 锁开着且锁定时，小组件不显示任何消息内容，只显示"已锁定"或占位；
  数据面由核心同一状态驱动。
- **分享进云吞**（PR #402 / [ADR-0097](0097-share-in.md)）：分享的**写入**不要求解锁——iOS 扩展只往收件目录复制文件，
  Android `ShareInbox` 在后台复制进 `filesDir/share-inbox/`，都是"往自己家投递"，不暴露任何内容；**查看和发送**要求解锁：
  `wonton://share/<令牌>` 把 App 拉到前台，锁定页先盖上，解锁后分享页照常出现（Intent 的数据还在，不需要为锁专门改 #402 的路由）。
  收件目录一天后自动清理的行为照旧。
- **App Intents / 快捷方式 / 通知点击**（[ADR-0041](0041-app-links-and-shortcuts.md)）：锁定期间"问 Bot"不直接把话发出去
  （等于绕过锁替主人说话），打开 App 进解锁后再发；Android 静态快捷方式只是打开页面，被锁定页盖住，解锁后自然继续。
  iOS Intent 的无 UI 执行路径在实现时验证，验证不过就把该 Intent 标记为锁定时不可用并说明原因。

### 5. 设备没设生物识别或锁屏：开不了，说清原因

- **开启时**：壳先探测。Android `canAuthenticate(BIOMETRIC_STRONG | DEVICE_CREDENTIAL)` 返回 `BIOMETRIC_ERROR_NONE_ENROLLED`
  （组合常量 API 30 起支持，minSdk 29 的设备上按平台版本分支处理，实现 PR 定），
  或 iOS `canEvaluatePolicy(.deviceOwnerAuthentication)` 报 `LAError.passcodeNotSet`——设置页这一行显示"设备没有设置锁屏，
  开不了 App 锁"，不给开。不写开关再在背后失效，不假装锁上（失败即停）。`NO_HARDWARE` / `HW_UNAVAILABLE` 分支的真机表现
  待实测，实现时验证后按同样方式说明原因。
- **开启后设备锁被移除**（比如主人把系统密码关了）：解锁验证过不了，App 停在锁定页，页面上说明"设备没有设置锁屏，无法验证"
  并指引去系统设置重新设上，设上即可解锁（主人 11-13 定）。不自动解锁、不降级成无验证放行——设备锁都关了的手机本来就对任何
  来者敞开，App 锁少一个验证手段就不再提供它声称的保护，如实停在锁页比假装能锁诚实。

### 6. 和恢复密钥"看之前先验证"统一成一套

**机制共用，策略分开。** 机制（怎么调系统验证）每端收敛成一个小组件：Android 把 PR #405 的 `RecoveryKeyAuth` 泛化成
`SystemAuth`（包住 `BiometricPrompt` 和 `canAuthenticate`，参数是标题和说明），iOS 抽出同构的 helper（包住 `LAContext`）。
恢复密钥页和 App 锁锁定页都调它；PR #405 先合（当前范围、当前策略），随后一个收尾 PR 把两处换成同一组件，行为不变。

| | 恢复密钥（PR #405，保持不变） | App 锁（本 ADR） |
| --- | --- | --- |
| 什么时候验证 | 每次显示密钥前，单独验证 | 锁定状态期间的首次解锁 |
| 设备没锁屏 | 不拦，直接显示 | 开不了，说明原因 |
| 取消/失败 | 停在原页，可再点，不另弹提示 | 停在锁定页，可再点，不另弹提示（系统弹窗自带原因） |
| 核心参与 | 无（壳本地状态） | 状态机在核心，`unlock_app()` 由壳调用 |

为什么恢复密钥保留"无锁屏直接显示"：那把密钥本来就是发给主人抄走的值，验证只是防路过的人瞟一眼；设备连锁屏都没有时没有
可用的验证手段，拦住只会让主人以为坏了。App 锁相反：它的全部价值就是"有验证手段"，没有就不提供这个开关。

**共用入口放核心还是壳**：判定（何时锁、锁着没有）在核心；验证（能不能验、怎么验）在壳。核心不新增恢复密钥相关接口，
恢复密钥的验证保持壳本地状态（`remember`，不跨页面保留），这已经是 #405 的实现，不动。

### 7. 壳的实现轮廓（不给代码，给落点）

- Android：`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/AppLock.kt`（锁定页 + 遮罩 + `FLAG_SECURE` 的
  DisposableEffect，样式随 `SecretInputs.kt` 先例）；`SystemAuth` 泛化自 `RecoveryKeyAuth`。
- iOS：`apps/ios/Wonton/AppLock.swift`（锁定页 + `scenePhase` 遮罩，盖在 `MainView` 根上）；`SystemAuth` helper 抽自
  `ChatBackupView.swift`。
- 界面文字四语言进 `strings.xml` / `Localizable.xcstrings`（`check-ui-text.py` 现有门禁覆盖），核心只给枚举和原始值
  （[ADR-0043](0043-app-languages.md)）。

## 没选的做法

- **壳自己存开关、自己计时**（Element X 的锁定服务就在壳层）：状态和"锁着不露内容"要被通知、小组件、将来的桌面端共用，
  壳里各写一份计时逻辑必然漂移，行为测试也没法只写一次。按 [ADR-0005](0005-native-clients-thick-core.md) 的厚核心放核心。
  壳保留的只有验证和遮罩这两件本来就是平台能力的事。
- **自设 PIN**（Telegram / Element X 都有）：**不做**（主人 11-13 定）。PIN 是要核心保管的又一份秘密，还要带回错误次数上限、
  防窥键盘、忘记 PIN 的恢复链路；系统锁屏密码已经覆盖"没有生物识别"的场景，有真实需求时另写 ADR。
- **锁定时暂停同步 / 断开隧道**：没必要。锁是呈现层的门，消息照常收（通知被藏内容）；断连只会让解锁后还要等同步，
  体验差且不增加安全。
- **追溯撤回/替换已发出的通知**：Android 做得到（同 tag 重发或 cancel），iOS 也能 `removeAllDeliveredNotifications`；但
  "通知消失"本身可能被当成数据丢失。**不做**（主人 11-13 定）。
- **用 KeyCachingService 式的"清内存密钥"当锁**（Signal 的做法）：我们的核心不持有等价的"主密钥"可清——聊天密钥在
  matrix-sdk 的本地存储里，清不掉；要清就得先把本地存储做成加密的，那是另一个（更大的）决策。App 锁只承诺"挡住界面"，
  不承诺防离线抽取（见"数据边界"）。

## 代价与待验证项

- 两端各一份锁定页和遮罩代码（厚核心不改变这一点：判定在核心，画法在壳）。
- 锁定页自动弹系统验证与首帧的时序（冷启动先出锁定页再弹），两端都要真机走一遍。
- Android `canAuthenticate` 的 `NO_HARDWARE` / `HW_UNAVAILABLE` 分支、iOS 非 `passcodeNotSet` 的 `canEvaluatePolicy` 失败
  （如生物识别硬件损坏）：实现时真机验证，按"说明原因、不给开/停在锁页"处理。
- `Activity.setRecentsScreenshotEnabled(false)` 的运行时可切换性（API 33+）。
- iOS 通知扩展读组容器里的锁定标记（随 ADR-0101 的 NSE 实现时验证）。
- iOS App Intents 在锁定时的拒绝路径。
- 兼容性风险低：`SettingsEntry` 未知值落 `Other`，旧壳不显示新入口；新壳遇到旧核心（没有 `app_lock()`）时设置页不出现这一行
  ——按 [ADR-0028](0028-settings-page.md) "不提供保存不起作用的设置"的先例。

## 查证来源（2026-11-13）

- Apple：`LAPolicy.deviceOwnerAuthentication`（原文注明设备没设密码时报 `passcodeNotSet`）、`scenePhase`（进切换器与进后台
  列为不同状态）。
- Android：`BiometricManager.canAuthenticate(int)` 与三个错误常量（`BIOMETRIC_STRONG | DEVICE_CREDENTIAL` 组合 API 30 起
  支持）；`FLAG_SECURE`（阻止屏幕截图和非安全显示）；`Activity.setRecentsScreenshotEnabled`（API 33+，文档未提运行时可切换性）。
- 上游代码：Signal Android 的 `ScreenLockSettingsRepository.kt`、`KeyCachingService.java:82-89`（`masterSecret == null` 即
  locked；超时定时在 240-245 行）；Element X Android 的 `LockScreenLockState.kt`（`Unlocked`/`Locked` 两态）、
  `DefaultLockScreenService.kt`（init 即 `lockIfNeeded(Duration.ZERO)`，退后台按 `gracePeriod` 上锁）、`LockScreenConfig.kt`
  （`GRACE_PERIOD = 2.minutes`、`MAX_PIN_CODE_ATTEMPTS_BEFORE_LOGOUT = 3`）；Element X iOS 的 `AppLockFlowCoordinator.swift`
  与 `Screens/AppLock/`；Telegram Android 的 `PasscodeActivity.java:336-378`、`SharedConfig.autoLockIn`
  （0/60/300/3600/18000 秒）。

## 关联

[ADR-0005](0005-native-clients-thick-core.md)（厚核心 + 薄壳）、[ADR-0011](0011-security-model.md)（安全只做必要的：默认不开，
主人自己开）、[ADR-0028](0028-settings-page.md)（设置页分组）、[ADR-0041](0041-app-links-and-shortcuts.md)（`wonton://` 链接、快捷指令与
App Intents）、[ADR-0043](0043-app-languages.md)（四语言壳资源）、[ADR-0003](0003-matrix-and-encrypted-push.md)（加密推送、通知
hidden 版本、保持连接）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)（设计系统）、
[ADR-0097](0097-share-in.md)（分享进云吞、App Group 容器，PR #402）、PR #405（恢复密钥的系统验证）、
[ADR-0101](0101-notifications.md)（通知与锁屏内容）。
