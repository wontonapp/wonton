# ADR-0106 客户端 Action 按领域拆成子枚举

- 状态：提议（2026-11-13）

## 背景

客户端核心的 `Action` 是原生壳发给 Rust 核心的唯一操作入口，接口约定是“操作进、ViewModel 出”：壳只把用户手势翻译成操作，核心校验参数、执行并发布 ViewModel。`Action` 同时还要能用 serde 往返，供行为测试和将来的 Web 端使用；iOS、Android 的 UniFFI 绑定也都从同一份协议类型生成。

验证于 uniffi 0.32.2、main `eac726e`：`wonton-protocol` 的 `Action` 元数据是 **32669 字节**，只剩 **99 字节**。uniffi 0.32.2 在 `uniffi_core` 的 `metadata.rs` 中把每个类型的元数据放进固定的 `BUF_SIZE = 32768` 字节缓冲区；缓冲区会拼入变体名、字段名、字段类型以及枚举、变体和字段的文档字符串。基线命令 `cargo build -p wonton-protocol --features uniffi` 通过；原样加入带两行文档的 `SetPluginMediaPermission` 后，同一命令触发 `MetadataBuffer::concat_long_str` 的断言。按 `nm -S` 测得 `UNIFFI_META_WONTON_PROTOCOL_ENUM_ACTION` 为 32669 字节；基线 `Action` 有 167 个变体，本次从最新 main 盘点源码得到 165 个顶层变体（其中 `Timeline` 是已完成的包装变体）。

这不是 iOS 特有的问题：只要启用 `wonton-protocol` 的 `uniffi` feature（包括 Android 和绑定生成）就会失败。升级 uniffi 也不能规避：验证时使用的最新版仍是 0.32.2，上游 main 仍使用 32768 字节缓冲区。

聊天线已经用 [ADR-0093](0093-side-chats.md) 和 #355 验证了拆分范式：16 个按时间线的变体被搬进 `TimelineAction`，顶层 `Action` 只保留 `Timeline { action: TimelineAction }`；`TimelineAction` 自己获得独立的 32 KiB 元数据空间。时间线协议类型、聊天分发和统一 `ClientCore.dispatch(Action)` 入口分别由 `wonton-protocol`、`wonton-client` 和两端壳共同使用。

## 决定

### 1. 保留一个入口，按领域拆子枚举

`Action` 继续是壳和核心之间唯一的可序列化操作入口，继续由 UniFFI 导出。把顶层变体一次性搬进下面的领域子枚举；每个领域子枚举用 `#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]` 独立导出。顶层 `Action` 只保留每个领域一个单字段包装变体，以及已经存在的时间线包装变体：

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "domain", rename_all = "snake_case")]
pub enum Action {
    Session { action: SessionAction },
    Home { action: HomeAction },
    Chat { action: ChatAction },
    Bot { action: BotAction },
    Push { action: PushAction },
    Plugins { action: PluginAction },
    Settings { action: SettingsAction },
    Onboarding { action: OnboardingAction },
    Files { action: FileAction },
    Pages { action: PageAction },
    Peers { action: PeerAction },
    BotContacts { action: BotContactAction },
    Family { action: FamilyAction },
    Friends { action: FriendsAction },
    Sharing { action: SharingAction },
    Timeline { action: TimelineAction },
    Agent { action: AgentAction },
}
```

上面的 serde 形状是决定的一部分：外层标签改名为 `domain`，单字段内容叫 `action`，避免内外两层枚举都使用 `type` 时发生标签冲突。`Timeline { action: TimelineAction }` 是 #355 已验证的绑定形状；第二阶段把现有 `AgentChainAction` 改名为 `AgentAction` 后沿用同一形状。实现 PR 仍需对 UniFFI 0.32.2 生成的 Swift/Kotlin 类型做编译核对，但不能改成多个运行时入口。无论生成器如何命名构造器，运行时都必须保持一个 `ClientCore.dispatch(Action)` 入口，不增加按领域的多个 dispatch 方法。

### 现状盘点：协议、核心和两端壳

- **协议。** `crates/wonton-protocol/src/action.rs` 定义顶层 `Action`；`crates/wonton-protocol/src/timeline_action.rs` 承载时间线操作；`crates/wonton-protocol/src/agent_chain.rs` 承载独立的 Agent 链操作。
- **核心入口。** 核心唯一解包点在 `crates/wonton-client/src/client/dispatch.rs`；页面、角色、分享和 Matrix 登录有各自的辅助分类，前后台、网络和推送由客户端生命周期路由处理；`crates/wonton-client/src/client/mod.rs` 接插件操作和剩余聊天操作。领域实现分别在 `chat/mod.rs`、`client/composer.rs`、`pages/mod.rs`、`files/mod.rs`、`peers/mod.rs`、`bot_contacts/mod.rs`、`friends/mod.rs`、`onboarding/mod.rs`、`matrix_login/mod.rs`、`plugins/core_upgrade.rs` 和 `roles.rs`。
- **iOS。** `AppearanceModel.swift`、`ChatModel.swift`、`HomeModel.swift`、`PluginsModel.swift`、`PeersView.swift`、`FriendsView.swift`、`BotContactsView.swift`、`OnboardingModel.swift`、`PagesModel.swift`、`FilesModel.swift` 和 `MatrixLoginView.swift` 统一处理操作错误；导航、安装确认、网页导航和网页加载回调也直接调用核心 dispatch。时间线调用统一构造 `Action.timeline(action:)`。
- **Android。** `ClientCoreExtensions.kt` 提供统一的 `send` 包装，`PluginsState.kt` 和 `PagesState.kt` 转发页面状态操作，聊天页以 `Action` 回调交给核心；系统生命周期、推送和少数插件页面回调直接构造或发送操作。时间线调用统一构造 `Action.Timeline(TimelineAction...)`。

### 现有直调方法的取舍

因 Action 元数据压力形成的第二条操作路径，需要和真正的请求–应答、订阅或平台回调区分：

- **第二阶段搬回统一 Action：** `ClientCore::set_plugin_media_permission`（`client/pages.rs`，#264 的详情页开关）加为 `PluginAction::SetPluginMediaPermission`；`ClientCore::close_onboarding`（`client/mod.rs`）加为 `OnboardingAction::CloseOnboarding`；`ClientCore::set_lock_screen_content`（`client/settings.rs`）加为 `SettingsAction::SetLockScreenContent`。它们都是壳发起、无须返回业务对象的普通状态变更，不能因为当时枚举快满就永久保留直调。
- **现有独立操作枚举并回统一入口：** `AgentChainAction`（`agent_chain.rs`）和 `ClientCore::dispatch_agent_chain`（`client/agent_chains.rs`）当时因为顶层元数据无空间而单独开入口；第二阶段改名为 `AgentAction`，包进 `Action::Agent { action: ... }`，删除并列的 `dispatch_agent_chain`。这样停链、改上限、改并发也遵守一个 `dispatch(Action)` 契约。
- **保留为方法：** `add_image_attachment` / `add_file_attachment` 返回附件标识，`subscribe_composer` 和 `attachment_thumbnail` 是订阅或取数据；`claim_share` / `send_share` / `discard_share` 管收件目录并返回处理结果；`start_side_chat` 等待服务器事件 ID；`answer_push_confirmation` / `reply_to_push` 是通知进程冷启动时的异步入口；`load_*`、`resolve_*`、`plugin_settings` 等是查询或请求–应答；`page_media_*` 是 WebView/系统媒体桥回调。`set_app_lock` / `unlock_app` 仍按 ADR-0102 的平台验证边界保留为方法，不把验证过程伪装成普通 Action。

因此第二阶段目标不是把所有 `ClientCore` 方法都塞进 Action，而是消除这几项“本来就是普通 Action、只是当时没空间”的临时绕路。

这四项迁回的目标形状和独立枚举的归属是：

| 第二阶段变更 | 归属 | 现有实现/变体 |
| --- | --- | --- |
| 插件网页媒体权限详情页开关 | `PluginAction` | 新增 `SetPluginMediaPermission`，取代 `ClientCore::set_plugin_media_permission` |
| 重看引导退出 | `OnboardingAction` | 新增 `CloseOnboarding`，取代 `ClientCore::close_onboarding` |
| 通知锁屏内容开关 | `SettingsAction` | 新增 `SetLockScreenContent`，取代 `ClientCore::set_lock_screen_content` |
| Agent 链操作 | `AgentAction` | 把现有 `AgentChainAction` 的 `StopChain`、`SetAgentLimits`、`SetConsumerAgentConcurrency` 搬进 `Action::Agent` |

领域清单与变体归属如下。表中列出本次盘点的 main 变体；“第二阶段新增”是本 ADR 已决定、但本阶段尚未改代码的迁回项。

| 子枚举 | 变体 |
| --- | --- |
| `SessionAction` | `CreateSession`、`SelectSession`、`DeleteSession` |
| `HomeAction` | `ConnectHome`、`PairHome`、`DismissPairingFailure`、`CancelPairing`、`InviteDevice`、`CloseDeviceInvite`、`AddServer`、`CancelAddServer`、`SwitchServer`、`RenameServer`、`RemoveServer`、`StartMatrixLogin`、`SubmitMatrixPassword`、`StartMatrixOidc`、`CompleteMatrixOidc`、`CancelMatrixLogin`、`AppForegrounded`、`AppBackgrounded`、`NetworkChanged`、`RetryConnection` |
| `ChatAction` | `JoinRoom`、`LeaveRoom`、`StartDirectChat`、`AskAgent`、`AnswerAgentChoice`、`ResetRecovery`、`ConfirmRecoveryKeySaved`、`RecoverEncryption`、`StartDeviceVerification`、`AcceptDeviceVerification`、`ConfirmVerification`、`DenyVerification`、`CancelDeviceVerification`、`SkipKeyHandoff`、`DismissChatProblem`、`AnswerConfirmation`、`StopConfirmationWindow`、`RetryConfirmations`、`SetChatLayout` |
| `BotAction` | `SetBotName`、`SetBotAvatar`、`ClearBotAvatar`、`DismissBotProfileProblem` |
| `PushAction` | `SetPushEndpoint`、`PushUnavailable`、`SetPushMethod` |
| `PluginAction` | `RefreshPlugins`、`ListOwnerSecrets`、`ShowOwnerSecret`、`HideOwnerSecret`、`DismissOwnerSecrets`、`StartPlugin`、`SetPortMapping`、`RefreshStore`、`InstallPlugin`、`UpgradePlugin`、`UninstallPlugin`、`DismissPluginTask`、`InstallNow`、`UpgradeNow`、`UpgradeAll`、`DismissStoreConfirmation`、`StartTakeover`、`EndTakeover`、`RefreshEgressProxy`、`SaveEgressProxy`、`ConfirmEgressProxy`、`DismissEgressConfirmation`、`RefreshRegistryMirror`、`SaveRegistryMirror`、`TestRegistryMirrors`、`RefreshCoreUpgrade`、`StartCoreUpgrade`、`CancelCoreUpgrade`、`SkipCoreUpgrade`；第二阶段新增 `SetPluginMediaPermission` |
| `SettingsAction` | `SetLanguages`、`SetAccentColor`、`SetThemeMode`、`SetTextSize`、`SetFont`；第二阶段新增 `SetLockScreenContent` |
| `OnboardingAction` | `RefreshOnboarding`、`RestartOnboarding`、`ChooseOnboardingAgent`、`ContinueOnboarding`、`SkipOnboardingStep`、`BackOnboardingStep`、`ToggleOnboardingNeed`、`EditOnboardingProfile`、`TestModelProvider`、`UseCurrentModel`、`ChooseModelLogin`、`RetryOnboardingItem`、`DismissModelProviderProblem`、`SetPrimaryAgent`、`DismissReplacedAgentOffer`、`DismissOnboardingTip`；第二阶段新增 `CloseOnboarding` |
| `FileAction` | `UploadFile`、`DownloadFile`、`CancelTransfer`、`DismissTransfer`、`ResendFileLink`、`DeleteFile`、`SendFileToDevices`、`GrantFileToRoom`、`DismissFileProblem` |
| `PageAction` | `OpenPage`、`ClosePage`、`AddMyApp`、`RemoveMyApp`、`MoveMyApp`、`SetAutoAddApps`、`RetryMyAppSync`、`AddPanelPage`、`RemovePanelPage`、`MovePanelPage`、`SelectPanelPage`、`SetBotPinConsent`、`PinCurrentPage`、`SetRoomEntryBlocked`、`ReloadTab`、`TabLoadFailed`、`TabLoadSucceeded`、`DismissUninstalledTabs` |
| `PeerAction` | `RefreshPeers`、`ShowPeerCode`、`HidePeerCode`、`ResetPeerCode`、`ScanPeerCode`、`ApprovePeerRequest`、`RejectPeerRequest`、`UnrejectPeerRequest`、`RemovePeer` |
| `BotContactAction` | `RefreshBotContacts`、`RequestBotContact`、`RequestPersonContact`、`ApproveBotContact`、`RejectBotContact`、`RemoveBotContact` |
| `FamilyAction` | `RefreshRoles`、`CreateRole`、`UpdateRole`、`DeleteRole`、`SetDefaultRole`、`AssignMemberRole`、`AssignFriendRole` |
| `FriendsAction` | `RefreshFriends`、`AddFriend`、`ApproveFriendRequest`、`RejectFriendRequest`、`CancelFriendRequest`、`RemoveFriend`、`BlockFriend`、`UnblockFriend`、`SaveFriendGroup`、`RemoveFriendGroup`、`SetFriendGroups` |
| `SharingAction` | `CreatePluginShare`、`UpdatePluginShare`、`RevokePluginShare`、`RefreshSharedApps`、`OpenSharedApp` |
| `AgentAction` | 第二阶段把现有独立 `AgentChainAction` 的 `StopChain`、`SetAgentLimits`、`SetConsumerAgentConcurrency` 搬进来 |

这些边界按核心分发责任划分，而不是按某一页的 UI 结构划分：`dispatch.rs` 把会话、服务器/连接、聊天、推送、引导、文件、外观、页面、跨服务器、联系人、好友/家庭和分享分成不同路由；插件安装、商店、出口、镜像和核心升级也由插件链路统一处理。这样子枚举的操作和负责它们的核心作用域一一对应，迁移后可把 `Action` 的通配拒绝分支缩小为各领域的穷举匹配。

### 2. UniFFI 导出和元数据门槛

- 所有子枚举和顶层 `Action` 都保留 `Serialize`、`Deserialize`、`Clone`、`Debug`、`PartialEq`、`Eq`，并在 `uniffi` feature 下各自 derive `uniffi::Enum`。子枚举不能只在 Rust 内部使用：Swift/Kotlin 壳必须能构造它们。
- 顶层包装变体只引用子枚举类型名，不展开子枚举的变体。调研报告已经确认，UniFFI 元数据记录外层字段的枚举类型名而不把嵌套枚举展开；因此每个子枚举拥有自己的 32 KiB 缓冲区，外层只保留很小的领域列表。
- 每个子枚举以 **16 KiB** 作为预警线，不等到 32 KiB 才拆。实现阶段应加入一个能在 CI 或本地检查中读取 `UNIFFI_META_*_ENUM_*` 符号大小的门禁/警告；超过 16 KiB 报警，接近 32 KiB 时必须先拆子领域再加操作。
- 变体上的文档只写给 Swift/Kotlin 使用者的一两句契约；背景、取舍和长解释写 ADR，不把文档重新堆回单个元数据缓冲区。
- serde 的外层 `domain`/`action` 标签只解决新结构的序列化冲突，不保留旧的扁平 JSON 形状。按仓库兼容规则直接切换，行为测试和将来的 Web 端一起迁移。

### 3. 以后新增操作的规矩

1. 先按真正负责该操作的核心作用域选一个子枚举；操作属于聊天时间线时加到 `TimelineAction`，属于插件网页页面状态时加到 `PageAction`，不能因为调用方恰好在同一张页面就放进别的枚举。
2. 新操作不得直接加到顶层 `Action`。顶层只允许增加新的“领域 → 子枚举”包装变体，而且只有出现一个尚未存在、拥有独立状态和分发责任的新领域时才能增加。
3. 子枚举达到 16 KiB 预警线后，先按子领域再拆一个子枚举，再把父子枚举包进该领域的单字段变体；不要继续在接近 32 KiB 的枚举上堆变体。拆分仍要保留一个顶层 `dispatch(Action)`，不按子领域造多个入口。
4. 变体文档保持短小；长背景放 ADR。新增字段和变体必须同时考虑 serde 往返、Swift/Kotlin 绑定和两端 dispatch 穷举。
5. `ClientCore` 直调方法只在以下情况允许：操作需要返回值或订阅/异步请求–应答语义，不能自然地表示为“不返回值的操作进、ViewModel 出”；或者操作属于平台/进程入口，生命周期独立于 `Action`（例如通知进程冷启动时的推送回答）。直调方法也必须经过核心校验、返回明确错误、能在测试和绑定中使用。
6. 不能因为“当前 Action 快满了”就把普通状态变更临时改成直调方法。那会产生第二套操作入口，绕开 `Action` 的 serde 覆盖和统一 dispatch。若一个不返回值的普通操作暂时来不及拆，先停在接口 PR 的拆分工作里，不以永久直调方法收尾。

## 迁移顺序

这是第二阶段“只改接口的 PR”的实施顺序；本篇不改 `crates/` 或 `apps/`。

1. **先迁协议类型。** 从最新 main 切分支，新增各领域子枚举文件或模块，把表中的变体原样搬入；先保留字段名和字段类型，只改变外层包装。`TimelineAction` 和 `TimelineTarget` 沿用 #355 已合并的形式，不再改名或重复搬运。
2. **再迁 serde 和核心 dispatch。** 外层 Action 改为领域包装，`crates/wonton-client/src/client/dispatch.rs` 先拆出各领域入口，再把 `crates/wonton-client/src/chat/`、`plugins/`、`pages/`、`files/`、`peers/`、`friends/`、`onboarding/` 等现有分发函数的参数改成对应子枚举。实现应尽量让领域函数穷举匹配；`ClientCore.dispatch(Action)` 只负责解包和把子枚举交给现有作用域。`Action::Timeline` 继续先交给 `composer_action` 或聊天时间线分发。
3. **补协议和行为测试。** 更新所有 Action 构造、serde round-trip 和 `ui-fake` 夹具；新增至少一组跨领域 JSON 往返样例，验证外层 `domain`/`action` 标签和嵌套字段。协议编译必须同时跑 `--features uniffi`，确认每个子枚举和顶层 Action 都能生成绑定。
4. **最后迁两端壳的构造调用。** iOS 和 Android 仍调用同一个 `ClientCore.dispatch`；只把构造从 `.setThemeMode(...)`、`Action.SetThemeMode(...)` 等扁平形式改为相应子枚举包装，保持各模型的发送错误处理不变。`TimelineAction` 的现有调用已经是包装形式，只需核对生成绑定的构造名。壳不新增领域分发逻辑、不直接调用作用域方法。
5. **直调方法按语义保留或搬回。** 第二阶段把四条普通操作路径一次迁回：`set_plugin_media_permission` → `PluginAction::SetPluginMediaPermission`，`close_onboarding` → `OnboardingAction::CloseOnboarding`，`set_lock_screen_content` → `SettingsAction::SetLockScreenContent`，以及现有 `AgentChainAction` → `AgentAction` 并入 `Action::Agent`；对应壳调用改走统一 `send`/`dispatch`。保留的方法有独立契约：附件区的 `add_image_attachment` / `add_file_attachment` 返回附件 ID，`claim_share` / `send_share` / `discard_share` 管分享收件目录并返回处理结果，通知的 `answer_push_confirmation` / `reply_to_push` 是冷启动推送入口且需要异步应答；`subscribe_*`、`load_*`、`resolve_*` 等返回 ViewModel、媒体或订阅的方法不属于 `Action`。`set_app_lock` / `unlock_app` 继续按 ADR-0102 的平台验证边界保留为方法。
6. **最后删旧路径。** 所有调用点、测试、文档链接和生成绑定适配后，删除旧的扁平 Action 变体及临时直调别名；不保留旧 JSON 形状、兼容构造器或重导出。两端的 dispatch 签名仍只保留一个 `Action` 入口。

### 两端 dispatch 的跟法

- **iOS**：`ChatModel`、`HomeModel`、`PluginsModel`、`PagesModel`、`PeersModel`、`FriendsModel`、`OnboardingModel`、`AppearanceModel` 等模型的 `send(_ action: Action)` 和 `ClientCore.dispatch(action:)` 签名不变；各调用点只包一层领域枚举。时间线继续由 `ChatModel`/`ComposerModel` 构造 `Action.timeline(action:)`。
- **Android**：`ClientCoreExtensions.send(action: Action)`、`PluginsState.send`、`PagesState.send` 以及 `core.dispatch` 调用签名不变；各 UI 处把 `Action.X(...)` 换成 `Action.Domain(DomainAction.X(...))`。页面和插件状态仍由核心 ViewModel 驱动，壳不按领域自行判断操作是否合法。
- **核心**：`ClientCore.dispatch` 是唯一解包点；作用域函数从接收 `Action` 改为接收领域子枚举后，错误仍在核心产生并经同一入口返回。这样两个壳不会各自维护一份领域路由。

## 没选的路线

### 升级 uniffi

不选。验证时的最新版是 0.32.2，上游 main 仍有 32768 字节 `BUF_SIZE`；升级没有改变失败条件。即使未来上游把缓冲区翻倍，也只是把结构性问题推后，不能替代按领域管理一个不断增长的操作协议。

### 提高上游 `BUF_SIZE` 或维护 uniffi 分叉

不选。把缓冲区改成 65536 会让每个元数据常量的编译期开销和内存跟着翻倍，只把问题推迟；本地 `[patch.crates-io]` 分叉还会引入长期维护负担，违背“不造轮子”。应把上游 bug 修复留给上游，客户端协议不依赖私有分叉。

### 删除或隐藏 Action 文档

不选。这份元数据中，文档约占主要部分；删掉或只在非-UniFFI 构建保留，能暂时腾出空间，但 Swift/Kotlin 使用者会失去操作契约，且变体继续增长后仍会撞上限。这是削弱接口可读性的推迟，不是根治。

### 所有新操作都改成 `ClientCore` 直调方法

不选。#264 的 `SetPluginMediaPermission` 证明它能短期绕过单个枚举上限，但会把“操作进、ViewModel 出”的统一可序列化入口分裂成几十个方法；serde 测试和未来 Web 端也要为每个方法另造入口。需要回值或有独立生命周期的方法可以保留，普通不返回值的操作不走这条路。

### 每个领域各暴露一个 dispatch 方法

不选。它会让 Swift/Kotlin 和未来 Web 端识别多个入口，序列化时还要额外包一层，失去单一的 `dispatch(Action)` 契约。外层包装只在元数据层面分组，运行时仍一个入口。

## 代价与待验证项

- 第二阶段会一次性迁移协议、核心分发、iOS、Android 和大量测试构造；这是有意的接口 PR，不能和 #523、#550 等尚未合并的协议改动并行修改。实现分支必须基于这些 PR 合并后的 main。
- 顶层 serde JSON 形状会从扁平 `type` 改成 `domain` + `action`；仓库内所有 round-trip 测试和 Web 预留代码都要一起迁移。
- UniFFI 对嵌套枚举单字段变体的 Swift/Kotlin 名称和默认值表示，需要在实现 PR 的绑定生成中实测；本 ADR 不把未实测的生成名字当成契约。
- 领域大小会随文档和字段变化；16 KiB 预警检查必须先落地，否则下一次拆分仍可能在 CI 才发现。

四条迁回项的领域归属已经在本 ADR 决定：媒体权限进 `PluginAction`，重看引导退出进 `OnboardingAction`，通知锁屏内容进 `SettingsAction`，Agent 链进 `AgentAction`。实现 PR 仍需实测 UniFFI 生成绑定的具体构造器名称，以及把四条路径的行为测试迁到统一入口；这不是领域归属的待定项。

## 关联

- [ADR-0005](0005-native-clients-thick-core.md)：厚 Rust 核心和 UniFFI 的“操作进、ViewModel 出”接口
- [ADR-0093](0093-side-chats.md)：`TimelineAction` 的已验证拆分范式
- [ADR-0094](0094-chat-attachments.md)：附件区方法与时间线 Action 的边界
- [ADR-0097](0097-share-in.md)：分享入口用 `ClientCore` 方法的先例
- [ADR-0101](0101-notifications.md)：冷启动通知动作方法
- [ADR-0102](0102-app-lock.md)：本机核心方法与前后台 Action 的边界
- PR #264：插件网页媒体权限；实现位于 `crates/wonton-client/src/client/pages.rs`，详情页开关走客户端核心接口。
