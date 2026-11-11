use serde::{Deserialize, Serialize};

use crate::{
    Answer, AvatarUpload, BotActivity, BotRef, ChatLayoutChoice, ChatLayoutTarget, DeviceNetwork,
    FileId, FontStyle, FriendBlockScope, FriendBlockTarget, ImageUpload, InstallPreview,
    MatrixOidcCallback, MatrixOidcLogin, MatrixPasswordLogin, ModelProviderInput, OnboardingTip,
    PushEndpoint, PushMethod, Rgb, SecretString, ShareGrantUpdate, TabDisplayChoice, TabRef,
    TextSize, ThemeMode, UploadSource,
};

/// 原生壳发给客户端核心的用户操作。
///
/// 壳只负责把用户手势翻译成操作；操作是否合法、状态怎么变，都由核心决定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// 新建一个会话并选中它。标题去掉首尾空白后不能为空。
    CreateSession {
        title: String,
    },
    /// 选中一个已有会话。
    SelectSession {
        session_id: String,
    },
    /// 删除一个会话；删除的是选中的会话时，清空选中。
    DeleteSession {
        session_id: String,
    },
    /// 不经配对，直接用给定的连接信息连接家里的服务器，并存在本机，之后每次启动自动连接。App 走
    /// [`Action::PairHome`]，连接信息由核心按配对码生成；这个操作给测试用。连接信息不合法时返回
    /// `InvalidInput`；连接的进展和结果体现在 [`HomeViewModel`](crate::HomeViewModel) 里。
    ConnectHome {
        ticket: String,
    },
    /// 扫码配对：`code` 是二维码里的文字（`wonton://pair?…`，格式见 ADR-0009 的“接口：扫码配对”），里面有一次性的
    /// 配对密钥，所以和恢复密钥一样打码；`device_name` 是主人在设备列表里认这台设备用的名字，去掉首尾空白后不能为
    /// 空。不是配对码、名字为空时返回 `InvalidInput`；进展和结果在 [`HomeViewModel::pairing`](crate::HomeViewModel::pairing) 里，配对成功后存下连接
    /// 信息并连上，之后每次启动自动连接，和 `ConnectHome` 一样。
    PairHome {
        code: SecretString,
        device_name: String,
    },
    /// 收起本次配对失败，返回扫码或粘贴；不取消正在进行的配对、不移除已保存的服务器。
    DismissPairingFailure,
    /// 取消正在进行的配对，回到扫码或粘贴；没有正在进行的配对时返回 `InvalidInput`。本机放弃这一次尝试，
    /// 服务器可能已经按交出的码登记了这台设备：同一个码再配对，服务端幂等地回成功，不会卡在“码已用过”。
    CancelPairing,
    /// “添加设备”：经隧道向家里的服务器要一个一次性配对码，显示给新设备扫（ADR-0009 的“接口：扫码配对”）。还没配对
    /// 时返回 `NotReady`；进展和码在 [`HomeViewModel::invite`](crate::HomeViewModel::invite) 里。再发一次换一个新码，
    /// 旧码到期前仍然能用。
    InviteDevice,
    /// 添加一台服务器：在一个新位置上走配对，当前那台的连接先断开、数据留着。配对成功后新的那台成为当前这台；
    /// 配对前可以 [`Action::CancelAddServer`] 回去（ADR-0042）。
    AddServer,
    /// 放弃添加，回到原来那台服务器。没在添加时返回 `InvalidInput`。
    CancelAddServer,
    /// 换到另一台配过的服务器：断开当前这台（进行中的传输暂停，换回来接着传），连上那台。`id` 是
    /// [`ServerRow::id`](crate::ServerRow::id)。
    SwitchServer {
        id: String,
    },
    /// 给配过的服务器改名，只存在这台手机上。名字去掉首尾空白后不能为空。
    RenameServer {
        id: String,
        name: String,
    },
    /// 从这台手机上移除一台服务器：删掉它在手机上的全部数据，不吊销这台设备（吊销在服务器那边）。移除的是当前这台时
    /// 换到剩下的第一台；一台都不剩时回到配对页。
    RemoveServer {
        id: String,
    },
    /// 开始添加外部 Matrix 账号，None 使用 matrix.org；方式和进展由服务器列表的 matrix_login 给出。
    StartMatrixLogin {
        homeserver: Option<String>,
    },
    SubmitMatrixPassword {
        login: MatrixPasswordLogin,
    },
    StartMatrixOidc {
        login: MatrixOidcLogin,
    },
    CompleteMatrixOidc {
        callback: MatrixOidcCallback,
    },
    CancelMatrixLogin {
        login_id: String,
    },
    /// App 回到前台。核心检查隧道和本地代理是否还能用，必要时重建；系统挂起 App 期间可能回收了套接字。
    /// Matrix 同步停了的话也在这时重启。
    AppForegrounded,
    /// App 进了后台。核心停止探测和自动重连，直到 [`Action::AppForegrounded`]；连接不主动断，后台音乐和正在传的文件
    /// 还在用它。
    AppBackgrounded,
    /// 系统报告网络变了。壳每次收到系统回调都发，不做判断：核心让 iroh 重新检查路径，并立刻重试（ADR-0032）。
    NetworkChanged {
        network: DeviceNetwork,
    },
    /// 主人点了“重试”：清零退避，立刻试一次。
    RetryConnection,

    // 聊天（Matrix）。网络操作都在后台执行：进展体现在 `ChatViewModel` 和时间线里，失败放进
    // `ChatViewModel::problem`。参数不合法、还没登录时直接返回错误。
    /// 接受房间邀请。
    JoinRoom {
        room_id: String,
    },
    /// 拒绝邀请，或者离开房间。
    LeaveRoom {
        room_id: String,
    },
    /// 和另一个用户（比如 Bot）发起私聊；已经有和他的私聊时什么都不做，对方不存在时失败。`user_id` 是完整的 ID，
    /// 主人输入的先用客户端核心的 `resolve_user_id` 补全。
    StartDirectChat {
        user_id: String,
    },
    /// 把一句话发给默认 Agent（Siri、快捷指令的“问 Bot”）：和它的私聊还没有就先建。壳随后按
    /// [`OnboardingViewModel::primary_bot_user_id`](crate::OnboardingViewModel::primary_bot_user_id) 打开这个私聊。
    AskAgent {
        text: String,
    },
    /// 发送文字。正文去掉首尾空白后不能为空。消息先以本地回显出现在时间线里。
    ///
    /// `reply_to` 是被回复消息的事件 ID（[`MessageAction::Reply`](crate::MessageAction::Reply) 里的），发出的是对它的
    /// 回复；那条消息不在这个房间里时返回 `InvalidInput`。
    SendText {
        room_id: String,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 发送图片。上传和发送的进展、`reply_to` 同 [`Action::SendText`]。
    SendImage {
        room_id: String,
        image: ImageUpload,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 把一条消息转发到另一个房间（ADR-0073）：照原样再发一条，不带“转发自”，加密附件不重新上传。`item_id` 同
    /// [`Action::RetrySend`]，只能转发菜单里有 [`MessageAction::Forward`](crate::MessageAction::Forward) 的消息。
    /// 进展在目标房间的时间线里（本地回显、发送状态），壳随后打开目标房间。
    ForwardMessage {
        room_id: String,
        item_id: String,
        to_room_id: String,
    },
    /// 撤回一条已经发出去的消息（Matrix 的 redaction），所有人那里都变成 [`MessageContent::Redacted`](crate::MessageContent::Redacted)。
    /// 只能撤回菜单里有 [`MessageAction::Redact`](crate::MessageAction::Redact) 的消息；还没发出去的用
    /// [`Action::CancelSend`]。失败进 `ChatViewModel::problem`。
    RedactMessage {
        room_id: String,
        item_id: String,
    },
    /// 马上重发一条发送失败的消息（[`SendState::Failed`](crate::SendState::Failed)），不等核心自动重发。`item_id` 是
    /// 它的 [`TimelineItem::id`](crate::TimelineItem::id)，要先订阅这个房间的时间线。
    RetrySend {
        room_id: String,
        item_id: String,
    },
    /// 取消发送：删掉一条还没发出去的消息（排队中或发送失败的），同一房间排在它后面的接着发。`item_id` 同
    /// [`Action::RetrySend`]。已经发出去的取消不了。
    CancelSend {
        room_id: String,
        item_id: String,
    },
    /// 加载更早的消息。要先订阅这个房间的时间线，结果体现在时间线的增量和翻页状态里。
    PaginateBackwards {
        room_id: String,
    },
    /// 壳报告列表停在哪儿：滚动停下、进出底部时发，不用每一帧发。要先订阅这个房间的时间线。核心据此重算
    /// [`TimelineUpdate::viewport`](crate::TimelineUpdate::viewport)，下面的新消息数随之变。
    SetTimelineScroll {
        room_id: String,
        scroll: crate::TimelineScroll,
    },
    /// 把房间标为读到最新一条，发出已读回执，房间的未读数随之清零。只在列表真的贴着最新一条时发（新消息到了、
    /// 滚回底部），翻在别处时不发。
    MarkRoomRead {
        room_id: String,
    },
    /// 记下这个房间输入框里的草稿，主人换个房间、关掉 App 再回来还在；正文为空、也没在回复时表示清掉。只存本机，
    /// 不上服务器。发送成功后核心自己清掉。`reply_to` 是正在回复的消息的事件 ID，读回时在
    /// [`ComposerDraft::reply`](crate::ComposerDraft::reply)。
    SetDraft {
        room_id: String,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 重置恢复：删掉账号的密钥备份和秘密存储，换新的交叉签名身份，本设备成为新身份下已验证的设备，再开启备份，
    /// 新的恢复密钥放进 `EncryptionStatus::recovery_key`。旧的恢复密钥作废，备份里的旧消息取不回了，其他设备要
    /// 输入新的恢复密钥。主人不用输密码，核心发的登录令牌就能向服务器确认身份。
    ResetRecovery,
    /// 主人已经把恢复密钥另外保存：核心把它从快照和本机清掉，之后不再显示。
    ConfirmRecoveryKeySaved,
    /// 用恢复密钥恢复：取回交叉签名的秘密和密钥备份，本设备随之被验证，能解密历史消息。
    RecoverEncryption {
        recovery_key: SecretString,
    },
    /// 请账号里的另一台设备确认本设备（ADR-0046）：发给自己账号已验证的设备，进展在
    /// `EncryptionStatus::verification_flow`。`EncryptionStatus::can_verify_with_other_device` 为假时前置条件不满足。
    StartDeviceVerification,
    /// 接受另一台设备发来的确认请求，随后两边出表情。
    AcceptDeviceVerification {
        flow_id: String,
    },
    /// 主人看到两台设备上的表情一样。
    ConfirmVerification {
        flow_id: String,
    },
    /// 主人看到两台设备上的表情不一样：取消这次确认，不信任对方。
    DenyVerification {
        flow_id: String,
    },
    /// 取消这次确认，或者拒绝收到的请求。
    CancelDeviceVerification {
        flow_id: String,
    },
    /// 清掉 `ChatViewModel::problem`。
    DismissChatProblem,
    /// 主人点了确认卡片上的按钮：`id` 是 [`ConfirmationCard::id`](crate::ConfirmationCard::id)，`answer` 原样用
    /// [`CardChoice::answer`](crate::CardChoice::answer)。核心经隧道发出去，进展和结果都在卡片上
    /// （[`ConfirmationView`](crate::ConfirmationView)，会话里和“等你确认”列表里是同一份），不进 `ChatViewModel::problem`。这张卡片已经在发、或者已经答完时
    /// 什么都不做。[`Answer::Unknown`] 返回 `InvalidInput`，家里的服务器还没连上时返回 `NotReady`。`choice_id` 原样用
    /// [`CardChoice::choice_id`](crate::CardChoice::choice_id)。
    AnswerConfirmation {
        id: String,
        answer: Answer,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        choice_id: Option<String>,
        /// 装插件、升级插件的卡片上填的密钥（ADR-0064）；别的卡片是空表。
        secrets: crate::SecretValues,
        /// 装插件、升级插件的卡片上填的设置（ADR-0077）；别的卡片是空表。
        #[serde(default)]
        settings: crate::SettingValues,
    },
    /// 主人点了卡片上放行时段的“停止”：只拦后面的调用，已经发给插件的不撤回（ADR-0024）。进展和结果在
    /// [`ConfirmationView::window`](crate::ConfirmationView::window)。已经在发时什么都不做，家里的服务器还没连上时返回
    /// `NotReady`。
    StopConfirmationWindow {
        id: String,
    },
    /// 重新取“等你确认”列表（[`ConfirmationsViewModel::problem`](crate::ConfirmationsViewModel::problem) 之后）。
    RetryConfirmations,

    // Bot 的资料（ADR-0044）。进展和结果在 `ChatViewModel::bots` 对应那一项的 `saving`、`problem` 里。
    /// 改 Bot 的名字。名字先过 [`check_bot_name`](crate::check_bot_name)，不行时返回 `InvalidInput`。
    SetBotName {
        plugin_id: String,
        name: String,
    },
    /// 换 Bot 的头像：`state` 为空是静态头像，否则是这个状态的头像。核心先以主人的身份传到 Matrix 的媒体库，再请家里的
    /// 服务器写进 Bot 的资料。
    SetBotAvatar {
        plugin_id: String,
        state: Option<BotActivity>,
        upload: AvatarUpload,
    },
    /// 清掉静态头像（`state` 为空）或一个状态的头像。
    ClearBotAvatar {
        plugin_id: String,
        state: Option<BotActivity>,
    },
    /// 清掉这个 Bot 的 `BotView::problem`。
    DismissBotProfileProblem {
        plugin_id: String,
    },

    // 推送（ADR-0003 的“设计：加密推送”）。进展在 `ChatViewModel::push` 里。
    /// 平台给了（或者换了）推送地址。核心经隧道把它登记给家里的服务器，再在 Matrix 上注册指向家里推送代理的
    /// pusher；之后每次登录都重新注册一遍。地址或公钥不合法时返回 `InvalidInput`。
    SetPushEndpoint {
        endpoint: PushEndpoint,
    },
    /// 平台收不了推送了：没有装分发器，分发器拒绝了登记或者注销了本 App。核心删掉家里的登记和 pusher，
    /// 把 `message` 照实放进 `ChatViewModel::push`。
    PushUnavailable {
        message: String,
    },
    /// 换收新消息的方式，存在本机，写不了时返回 `Storage`。换成 [`PushMethod::Keepalive`](crate::PushMethod::Keepalive)
    /// 时核心删掉家里的推送登记和 pusher，同一条消息不会既推送又本地通知；壳不再给推送地址。
    SetPushMethod {
        method: PushMethod,
    },

    // 插件（ADR-0017）。网络操作都在后台执行，进展和结果在 `PluginsViewModel` 里。
    /// App 使用的语言在第一项，后接系统语言偏好（BCP 47）；壳在启动和语言改变时发送。客户端详情用第一项，服务器按列表挑插件文字。
    SetLanguages {
        languages: Vec<String>,
    },
    /// 重新取已安装的插件和它们能不能升级。
    RefreshPlugins,
    /// 展开插件生成给主人的密钥分组；每次重新取列表，不预取值。
    ListOwnerSecrets {
        plugin_id: String,
    },
    /// 展开一项密钥，每次向服务器重新取值；只接受列表里的名字。
    ShowOwnerSecret {
        plugin_id: String,
        name: String,
    },
    /// 收起一项并清除值；作废尚未完成的取值请求。
    HideOwnerSecret {
        plugin_id: String,
        name: String,
    },
    /// 收起分组或离开详情页，清除列表、值和问题；晚到的请求不能恢复它们。
    DismissOwnerSecrets {
        plugin_id: String,
    },
    /// 启动已停止的插件；已经运行时由核心幂等处理。
    StartPlugin {
        plugin_id: String,
    },
    /// 在这台设备上开关插件的同号端口映射；未实现映射的客户端返回 `NotReady`，不改变开关或绑定端口。
    SetPortMapping {
        plugin_id: String,
        enabled: bool,
    },
    /// 屏幕上正显示着哪个插件的页面；没有插件页面在屏幕上时发 `None`。
    ///
    /// 壳在页面换走、被盖住、重新露出来时发。App 进后台不用补发：核心按 [`Action::AppBackgrounded`] 一律当作
    /// 不可见，回前台恢复，壳也不用重报。核心照它自动开关这个插件的端口映射（ADR-0072），不动
    /// [`Action::SetPortMapping`] 记下的手动开关：手动打开的不因页面看不见而关掉。
    SetVisiblePlugin {
        plugin_id: Option<String>,
    },
    /// 取商店的列表。`force` 为真时家里的服务器不等缓存过期，直接去来源取（下拉刷新）。
    RefreshStore {
        force: bool,
    },
    /// 主人在确认页上点了“安装”：按预览安装，授予预览里列出的全部权限。插件包在预览之后变了时失败
    /// （`PluginFailure::Changed`），要重新预览。主人打开了确认页上的开关时，把预览的 `lan_webui.enabled` 改成 `true` 再发。
    InstallPlugin {
        preview: InstallPreview,
        /// 主人在确认页上填的密钥（ADR-0064）。
        secrets: crate::SecretValues,
        /// 主人在确认页上填的设置（ADR-0077）。
        #[serde(default)]
        settings: crate::SettingValues,
    },
    /// 主人在升级确认页上点了“升级”，同 [`Action::InstallPlugin`]。
    UpgradePlugin {
        preview: InstallPreview,
        secrets: crate::SecretValues,
        #[serde(default)]
        settings: crate::SettingValues,
    },
    /// 卸载。`delete_data` 为真时同时删除它的数据、密钥和 Bot 账号，不可恢复。
    UninstallPlugin {
        id: String,
        delete_data: bool,
    },
    /// 清掉一个做完的任务（成功或失败的提示）。
    DismissPluginTask {
        id: String,
    },
    /// 主人在商店列表或详情页点了"获取"：核心取预览、判断要不要请主人拿主意（规则在核心，ADR-0017）。
    /// 不用拿主意的直接开始装（ [`PluginsViewModel::tasks`](crate::PluginsViewModel::tasks) 里出现任务）；
    /// 要拿主意的放进 [`PluginsViewModel::confirmation`](crate::PluginsViewModel::confirmation)，壳打开确认页。
    /// 取预览失败也进 `tasks`，按钮回到"获取"。正在取预览的插件在 [`PluginsViewModel::pending`](crate::PluginsViewModel::pending) 里。
    InstallNow {
        source: String,
    },
    /// 主人点了"更新"，同 [`Action::InstallNow`]，对装着的应用升级到提示的版本。
    UpgradeNow {
        plugin_id: String,
    },
    /// 主人在商店页点了"全部更新"：逐个升级 [`PluginsViewModel::updatable`](crate::PluginsViewModel::updatable) 里的应用，
    /// 不用拿主意的直接开始；要拿主意的排进 [`PluginsViewModel::confirmation`](crate::PluginsViewModel::confirmation)，一次一条。
    UpgradeAll,
    /// 主人关掉了一键操作的确认页，没装也没升。还有排着队的就放下一条进来，没有就清空。
    DismissStoreConfirmation,

    // “我来操作”（ADR-0024）。续约在核心里，进展和结果在 `PluginsViewModel.takeover` 里。
    /// 主人在插件网页的标题栏上点了“我来操作”。正在操作别的插件时，那个先交还。
    StartTakeover {
        id: String,
    },
    /// 主人点了“交还”或者离开了这个网页。`id` 不是正在操作的插件时什么都不做，换页面时后一页的开始可能先到。
    /// 进后台不用发，核心自己交还。
    EndTakeover {
        id: String,
    },

    // 出网代理的设置页（ADR-0023）。网络操作都在后台执行，进展和结果在 `PluginsViewModel.egress` 里。
    /// 打开设置页：取设置。探测结果太旧时家里的服务器在后台再测，这里接着取几次，把新结果带回来。
    RefreshEgressProxy,
    /// 主人点了保存。`use_plugin` 选用已安装的代理插件，此时 `address` 必须为空；两者都空表示取消代理。
    /// `direct_cidrs` 用逗号或空白分隔。要重启插件时先放进 `egress.confirmation`，等 [`Action::ConfirmEgressProxy`]。
    SaveEgressProxy {
        address: String,
        direct_cidrs: String,
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = false))]
        use_plugin: bool,
    },
    /// 主人确认了 `egress.confirmation` 列出的重启。
    ConfirmEgressProxy,
    /// 主人没有确认，什么都不改。
    DismissEgressConfirmation,

    /// 取镜像加速设置。
    RefreshRegistryMirror,
    /// 保存加速地址；空串表示关闭。
    SaveRegistryMirror {
        mirror: String,
    },
    /// 测官方候选地址和当前使用地址，按顺序去重。
    TestRegistryMirrors,

    // 新用户引导（ADR-0034）。网络操作都在后台执行，进展和结果在 `OnboardingViewModel` 里。
    /// 取引导的状态、推荐清单和服务商列表。
    RefreshOnboarding,
    /// 主人在设置里选了重看：从头走一遍，每一步显示现在的设置，点完成之前什么都不改。
    RestartOnboarding,
    /// 选中一个 Agent（[`OnboardingAgentRow::id`](crate::OnboardingAgentRow::id)）。没装的点完成时安装。
    ChooseOnboardingAgent {
        id: String,
    },
    /// 记下这一步的选择，到下一步。最后一页上是“完成”：一起执行所有选择；执行完了再发一次是离开引导。
    ContinueOnboarding,
    /// 跳过这一步，不算做完：没接模型时 Agent 的私聊照常说明要接模型。
    SkipOnboardingStep,
    /// 回到上一步，记下的选择还在。
    BackOnboardingStep,
    /// 选中或者取消一项需求。选中的需求推荐的插件，点完成时安装。
    ToggleOnboardingNeed {
        id: String,
    },
    /// 你的资料这一步改了名字或邮箱。只记下来，点完成时才保存。
    EditOnboardingProfile {
        display_name: String,
        email: String,
    },
    /// 测试填的服务商，什么都不保存。测过了（包括连不上服务商）就记成这一步的选择；`then_continue` 为真时接着到下一步。
    TestModelProvider {
        input: ModelProviderInput,
        then_continue: bool,
    },
    /// 选用模型网关里已经接好的模型（[`OnboardingModelChoice::Current`](crate::OnboardingModelChoice::Current)）。
    UseCurrentModel,
    /// 选用 Agent 官方程序的账号登录（[`AgentLogin::id`](crate::AgentLogin::id)）。
    ChooseModelLogin {
        id: String,
    },
    /// 最后一页上重试失败的一项（[`OnboardingReviewItem::id`](crate::OnboardingReviewItem::id)）。
    RetryOnboardingItem {
        id: String,
    },
    /// 清掉 [`OnboardingModelStep::problem`](crate::OnboardingModelStep::problem)。
    DismissModelProviderProblem,
    /// 在设置里改默认 Agent。
    SetPrimaryAgent {
        id: String,
    },
    /// 主人回答了要不要卸掉被换下来的 Agent，不再问。
    DismissReplacedAgentOffer,
    /// 这条提示看过了，这台设备不再显示。
    DismissOnboardingTip {
        tip: OnboardingTip,
    },

    // 文件（ADR-0020）。网络操作都在后台执行，进展和结果在传输列表和文件卡片里。
    /// 上传一个文件，传完放进共享文件夹 `transfers`。`name`、`mime_type` 是系统给的文件名和类型，核心读不到内容 URI 的这些
    /// 信息。`room` 不为空时，传完把这个房间的 Bot 加进访问范围，再把链接发进这个房间。
    UploadFile {
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
        room: Option<String>,
    },
    /// 下载到 App 自己的目录，路径在 [`Transfer::destination`](crate::Transfer::destination)，传完才出现在这个路径上。断了从断点
    /// 接着下，文件变了从头下。要存到别处，壳调用系统的分享或保存。这个文件正在下载时返回 `InvalidInput`。
    DownloadFile {
        id: FileId,
    },
    /// 取消进行中、暂停或者链接没发出的传输，并从列表里拿掉；上传时连服务器上传了一半的内容一起删，字节已经传完的不删，文件留在
    /// 服务器上。已经做完或失败的不受影响：取消和传完撞上时不算错。`id` 是 [`Transfer::id`](crate::Transfer::id)，不是文件编号。
    CancelTransfer {
        id: String,
    },
    /// 从列表里清掉不再进行的传输。服务器上的文件不受影响，要删用 [`Action::DeleteFile`]。`delete_local_file` 为真时连 App
    /// 目录里下载的文件一起删，只对下载有效，上传时返回 `InvalidInput`：手机上的原文件不归 App 管。为假时文件留着，卡片还能
    /// 打开。还在进行的传输要先取消，否则返回 `InvalidInput`。
    DismissTransfer {
        id: String,
        delete_local_file: bool,
    },
    /// 只重发链接：再授权一次，再把链接发进上传时的房间，不重传文件。只对
    /// [`TransferState::LinkNotSent`](crate::TransferState::LinkNotSent) 的传输有效，其他返回 `InvalidInput`。
    ResendFileLink {
        id: String,
    },
    /// 删掉编号。设备上传的文件核对没变过后连文件一起删，其他来源的原文件不动。
    DeleteFile {
        id: FileId,
    },
    /// 把链接发到和主人的“Wonton”私聊，主人所有设备都看得到卡片。
    SendFileToDevices {
        id: FileId,
    },
    /// 把这个房间的 Bot 加进文件的访问范围。在 Bot 的房间里发链接时一起调，Bot 才能看和再分享。
    GrantFileToRoom {
        id: FileId,
        room: String,
    },
    /// 清掉 [`TransfersViewModel::problem`](crate::TransfersViewModel::problem)。
    DismissFileProblem,

    // 核心的一键升级（ADR-0025）。网络操作都在后台执行，进展和结果在 `CoreUpgradeViewModel` 里。
    /// 取升级状态、查新版本。打开升级页时 `manual` 为假，主人点“检查”时为真。
    RefreshCoreUpgrade {
        manual: bool,
    },
    /// 主人在确认页上点了“升级”。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    StartCoreUpgrade {
        digest: String,
    },
    /// 请求取消，只在 [`CoreUpgradePhase::Pulling`](crate::CoreUpgradePhase::Pulling) 阶段有效。
    CancelCoreUpgrade,
    /// 这台设备不再提示这个版本。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    SkipCoreUpgrade {
        digest: String,
    },

    // 外观（ADR-0026、ADR-0028）。设置存在本机，写不了时返回 `Storage`，外观不变。
    /// 把重点色换成这个颜色。任何颜色都可以，读字的颜色由核心推导。
    SetAccentColor {
        color: Rgb,
    },
    /// 选明暗。
    SetThemeMode {
        mode: ThemeMode,
    },
    /// 选字号。
    SetTextSize {
        size: TextSize,
    },
    /// 选字体，只从 [`AppearanceViewModel::fonts`](crate::AppearanceViewModel::fonts) 里选。
    SetFont {
        font: FontStyle,
    },

    // 聊天显示方式（ADR-0060）。选择存在 Matrix 账号数据里，写不了时进 `ChatViewModel::problem`，显示方式不变。
    /// 把这一份的选择换成 `choice`：为空的项回到默认值。
    SetChatLayout {
        target: ChatLayoutTarget,
        choice: ChatLayoutChoice,
    },

    // 底栏（ADR-0033）。配置存在本机，写不了时返回 `Storage`，底栏不变。
    /// 打开一个页面。插件页已固定时切到它的标签，没固定时盖在主界面上（[`TabsViewModel::overlay`](crate::TabsViewModel::overlay)）。
    SelectTab {
        tab: TabRef,
    },
    /// 关掉盖在主界面上的插件页。
    CloseTabOverlay,
    /// 固定到底栏。栏满了（[`TabsViewModel::full`](crate::TabsViewModel::full)）要给 `replacing`，否则返回 `InvalidInput`。
    PinTab {
        tab: TabRef,
        replacing: Option<TabRef>,
    },
    /// 从栏上拿掉。聊天不能拿，返回 `InvalidInput`。
    UnpinTab {
        tab: TabRef,
    },
    /// 把页面放进面板的最后（ADR-0052）。`tab` 必须是插件页、插件里的某一页或 `https://` 公网网页，
    /// 聊天、应用、设置和分享页不行；已经在面板里、面板已满
    /// （[`MAX_PANEL_PAGES`](crate::MAX_PANEL_PAGES)）或页面现在放不了时返回 `InvalidInput`。
    /// 写进账号数据才算成功；连不上服务器时改动不生效，错误按类别返回。
    AddPanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 从面板拿掉。不在面板里时返回 `InvalidInput`。
    RemovePanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 把面板里的一项移到 `index`（移动后的下标，从 0 开始）。不在面板里或下标越界时返回 `InvalidInput`。
    MovePanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
        index: u32,
    },
    /// 壳报告正在看面板的哪一页：核心记住它（本机，下次拉开还是它），并清掉这一页的红点。
    /// 拉开面板和翻页时都发。
    SelectPanelPage {
        scope: crate::PanelScope,
        tab: TabRef,
    },
    /// 收回或给回一个 Bot 往主面板钉页的同意（设置里）；只改以后还问不问，已经钉上的页面不动。
    SetBotPinConsent {
        plugin_id: String,
        bot: String,
        allowed: bool,
    },
    /// 把正在看的页面钉到面板或底栏（胶囊“···”菜单里的“钉到面板”“钉到底栏”）。
    ///
    /// `url` 是页面此刻的网址，核心按它算出要钉的是哪一种页面：装着的插件的网页入口下的路径，
    /// 钉成插件里的某一页（路径和清单里的页面对上时是那个页面本身）；不是插件的网站就只收
    /// `https://`，钉成公网网页。`title` 是页面自己的标题，没有时核心按插件或域名补。
    /// 目标是底栏且栏满了时，`replacing` 同 [`Action::PinTab`]。
    PinCurrentPage {
        url: String,
        title: Option<String>,
        to: crate::PinTarget,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        replacing: Option<TabRef>,
    },
    /// 壳报告房间里盖着消息列表的东西开没开：长按菜单、多选模式、转发或删除确认、交给 Bot 的选择、
    /// 斜杠命令列表。核心把它和核心自己知道的状态合成 [`TimelineUpdate::entry_blocked`]，
    /// 壳只读那一个字段决定上推拉面板的入口响不响应。换房间后按没盖算，壳不用补发。
    SetRoomEntryBlocked {
        room_id: String,
        blocked: bool,
    },
    /// 把一项移到 `index`（移动后的下标，从 0 开始）。
    MoveTab {
        tab: TabRef,
        index: u32,
    },
    /// 底栏上显示文字还是只显示图标。
    SetTabLabels {
        visible: bool,
    },
    /// 给插件页选打开方式，不是插件页时返回 `InvalidInput`。
    SetTabDisplay {
        tab: TabRef,
        display: TabDisplayChoice,
    },
    /// 全屏显示插件页。主人把这个页面设成了 `Standard` 也可以手动进。
    EnterFullscreen {
        tab: TabRef,
    },
    /// 退出全屏。手动进的回到带栏的样子；打开就全屏的页面回到进来之前的标签，没有就回第一项。
    ExitFullscreen {
        tab: TabRef,
    },
    /// 重新加载插件页（重试、“···”里的重新加载）。`page` 指是哪一份：底栏的和面板里的同一页面
    /// 各自加载，互不影响（ADR-0052）。
    ReloadTab {
        page: crate::PlacedTab,
    },
    /// 壳报告网页加载失败。`status` 是 HTTP 状态码，没拿到响应时为空；原因由核心判断。
    /// `page` 指是哪一份实例。
    TabLoadFailed {
        page: crate::PlacedTab,
        status: Option<u16>,
    },
    /// 壳报告网页加载成功（如按返回回到成功的历史页），核心清掉这个页面的加载失败显示。
    TabLoadSucceeded {
        page: crate::PlacedTab,
    },
    /// 主人看过了“已卸载，已从底栏移除”的提示。
    DismissUninstalledTabs,

    // 跨服务器配对（ADR-0050）。进展和失败原因都在 PeersViewModel 里。
    /// 打开“其他服务器”页面或下拉刷新：经隧道取配对过的服务器列表和出码状态（`GET /_wonton/api/peers`），
    /// 结果在 [`PeersViewModel`](crate::PeersViewModel) 里。
    RefreshPeers,
    /// 打开出码页：取本机长期有效的配对码（`GET /_wonton/api/peers/code`），码在
    /// [`PeersViewModel::code`](crate::PeersViewModel::code) 里，壳画成二维码、放进“分享”。出码页开着时客户端几秒取一次列表，
    /// 好让对方当面扫了以后请求马上出现。
    ShowPeerCode,
    /// 关掉出码页：`PeersViewModel::code` 清空，客户端不再快速取列表。
    HidePeerCode,
    /// 重置配对码（`POST /_wonton/api/peers/code/reset`）：换一个令牌，旧码作废，用旧码发来、还没处理的请求一并清掉。
    ResetPeerCode,
    /// 提交扫来或粘贴的配对码（`wonton://peer?…`）：核心经 `POST /_wonton/api/peers` 交给自己的服务器，向出码方发一条
    /// 配对请求，出码方同意后配对成立。码已作废、对方拒绝过、对方暂时收不了这些原因见
    /// [`PeerError`](crate::PeerError)，体现在 [`PeersViewModel::state`](crate::PeersViewModel::state) 里。
    ScanPeerCode {
        code: SecretString,
    },
    /// 同意一条发来的请求，`id` 是 [`PeerRequestRow::id`](crate::PeerRequestRow::id)。
    ApprovePeerRequest {
        id: String,
    },
    /// 拒绝一条发来的请求：记进“已拒绝”，同一台服务器再发来直接回“对方拒绝了”。
    RejectPeerRequest {
        id: String,
    },
    /// 撤销一次拒绝，`id` 是 [`RejectedPeerRow::id`](crate::RejectedPeerRow::id)：对方再发来照常收下。
    UnrejectPeerRequest {
        id: String,
    },
    /// 移除和一台服务器的配对，或取消自己发出、对方还没处理的请求（`DELETE /_wonton/api/peers/{server_name}`）：
    /// 两边的名单各自撤掉，已有的跨机房间留着，消息停发。`server_name` 是 [`PeerRow::server_name`](crate::PeerRow::server_name)。
    RemovePeer {
        server_name: String,
    },

    // Bot 的联系人（ADR-0045、ADR-0050），归属人在 Bot 资料页管；共享 Bot 是主人。结果和失败原因都在
    // BotContactsViewModel 里。`user_id` 是 [`BotContact::user_id`](crate::BotContact::user_id)。
    /// 打开某个 Bot 的联系人或下拉刷新（`GET`
    /// [`bot_contacts_path`](crate::bot_contacts_path)），之后的结果都是这个 Bot 的。
    RefreshBotContacts {
        bot: BotRef,
    },
    /// 请对方做这个 Bot 的联系人：本机的 Bot，或配对过的服务器上的人或 Bot。
    RequestBotContact {
        bot: BotRef,
        user_id: String,
    },
    /// 同意对方发来的请求（[`ContactState::Incoming`](crate::ContactState::Incoming)）。
    ApproveBotContact {
        bot: BotRef,
        user_id: String,
    },
    /// 拒绝对方发来的请求。
    RejectBotContact {
        bot: BotRef,
        user_id: String,
    },
    /// 删掉联系人、取消自己发出的请求，或清掉“对方拒绝了”“没有这个人”的行；同时通知对方。
    RemoveBotContact {
        bot: BotRef,
        user_id: String,
    },

    /// 取当前账号的好友、请求和屏蔽名单；外部 Matrix 账号同样可用。
    RefreshFriends,
    /// 发送带 app.wonton.friend 的标准 Matrix 私聊邀请，user_id 必须为完整 Matrix ID。
    AddFriend {
        user_id: String,
    },
    ApproveFriendRequest {
        room_id: String,
    },
    RejectFriendRequest {
        room_id: String,
    },
    CancelFriendRequest {
        room_id: String,
    },
    /// 同时收回这个人所有插件访问（包含组授权），不退出私聊。
    RemoveFriend {
        user_id: String,
    },
    BlockFriend {
        target: FriendBlockTarget,
        scope: FriendBlockScope,
    },
    UnblockFriend {
        target: FriendBlockTarget,
        scope: FriendBlockScope,
    },
    /// 新建组时 id 为空，修改时给现有组 ID；名字去掉首尾空白后不能为空。
    SaveFriendGroup {
        id: Option<String>,
        name: String,
    },
    /// 删除组时同时收回该组授权，不删除组内好友。
    RemoveFriendGroup {
        id: String,
    },
    /// 完整替换好友的分组，不重复包含同一个组 ID。
    SetFriendGroups {
        user_id: String,
        groups: Vec<String>,
    },
    /// 新增授权，只有分享方主人能做；未声明好友识别能力的插件要确认风险。
    CreatePluginShare {
        grant: ShareGrantUpdate,
    },
    UpdatePluginShare {
        id: String,
        grant: ShareGrantUpdate,
    },
    /// 立即拒绝后续请求，同时断开这个授权不再允许的现有长连接。
    RevokePluginShare {
        id: String,
    },
    /// 重取一个来源或全部来源的分享；None 表示全部。
    RefreshSharedApps {
        owner_user_id: Option<String>,
    },
    /// 来源和插件一起定位，不能只按插件 ID 打开；失败放在该项的 ShareProblem。
    OpenSharedApp {
        owner_user_id: String,
        plugin_id: String,
    },
}
