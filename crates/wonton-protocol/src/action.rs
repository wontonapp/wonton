use serde::{Deserialize, Serialize};

use crate::{
    Answer, DeviceNetwork, FileId, FontStyle, ImageUpload, InstallPreview, OnboardingTip,
    PushEndpoint, Rgb, SecretString, TabDisplayChoice, TabRef, TextSize, ThemeMode, UploadSource,
};

/// 原生壳发给客户端核心的用户操作。
///
/// 壳只负责把用户手势翻译成操作；操作是否合法、状态怎么变，都由核心决定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// 新建一个会话并选中它。标题去掉首尾空白后不能为空。
    CreateSession { title: String },
    /// 选中一个已有会话。
    SelectSession { session_id: String },
    /// 删除一个会话；删除的是选中的会话时，清空选中。
    DeleteSession { session_id: String },
    /// 不经配对，直接用给定的连接信息连接家里的服务器，并存在本机，之后每次启动自动连接。App 走
    /// [`Action::PairHome`]，连接信息由核心按配对码生成；这个操作给测试用。连接信息不合法时返回
    /// `InvalidInput`；连接的进展和结果体现在 [`HomeViewModel`](crate::HomeViewModel) 里。
    ConnectHome { ticket: String },
    /// 扫码配对：`code` 是二维码里的文字（`wonton://pair?…`，格式见 ADR-0009 的“接口：扫码配对”），里面有一次性的
    /// 配对密钥，所以和恢复密钥一样打码；`device_name` 是主人在设备列表里认这台设备用的名字。不是配对码时返回
    /// `InvalidInput`；进展和结果在 [`HomeViewModel::pairing`](crate::HomeViewModel::pairing) 里，配对成功后存下连接
    /// 信息并连上，之后每次启动自动连接，和 `ConnectHome` 一样。
    PairHome {
        code: SecretString,
        device_name: String,
    },
    /// “添加设备”：经隧道向家里的服务器要一个一次性配对码，显示给新设备扫（ADR-0009 的“接口：扫码配对”）。还没配对
    /// 时返回 `NotReady`；进展和码在 [`HomeViewModel::invite`](crate::HomeViewModel::invite) 里。再发一次换一个新码，
    /// 旧码到期前仍然能用。
    InviteDevice,
    /// App 回到前台。核心检查隧道和本地代理是否还能用，必要时重建；系统挂起 App 期间可能回收了套接字。
    /// Matrix 同步停了的话也在这时重启。
    AppForegrounded,
    /// App 进了后台。核心停止探测和自动重连，直到 [`Action::AppForegrounded`]；连接不主动断，后台音乐和正在传的文件
    /// 还在用它。
    AppBackgrounded,
    /// 系统报告网络变了。壳每次收到系统回调都发，不做判断：核心让 iroh 重新检查路径，并立刻重试（ADR-0032）。
    NetworkChanged { network: DeviceNetwork },
    /// 主人点了“重试”：清零退避，立刻试一次。
    RetryConnection,

    // 聊天（Matrix）。网络操作都在后台执行：进展体现在 `ChatViewModel` 和时间线里，失败放进
    // `ChatViewModel::problem`。参数不合法、还没登录时直接返回错误。
    /// 接受房间邀请。
    JoinRoom { room_id: String },
    /// 拒绝邀请，或者离开房间。
    LeaveRoom { room_id: String },
    /// 和另一个用户（比如 Bot）发起私聊；已经有和他的私聊时什么都不做。
    StartDirectChat { user_id: String },
    /// 发送文字。正文去掉首尾空白后不能为空。消息先以本地回显出现在时间线里。
    SendText { room_id: String, body: String },
    /// 发送图片。上传和发送的进展同 [`Action::SendText`]。
    SendImage { room_id: String, image: ImageUpload },
    /// 马上重发一条发送失败的消息（[`SendState::Failed`](crate::SendState::Failed)），不等核心自动重发。`item_id` 是
    /// 它的 [`TimelineItem::id`](crate::TimelineItem::id)，要先订阅这个房间的时间线。
    RetrySend { room_id: String, item_id: String },
    /// 取消发送：删掉一条还没发出去的消息（排队中或发送失败的），同一房间排在它后面的接着发。`item_id` 同
    /// [`Action::RetrySend`]。已经发出去的取消不了。
    CancelSend { room_id: String, item_id: String },
    /// 加载更早的消息。要先订阅这个房间的时间线，结果体现在时间线的增量和翻页状态里。
    PaginateBackwards { room_id: String },
    /// 把房间标为读到最新一条，发出已读回执，房间的未读数随之清零。
    MarkRoomRead { room_id: String },
    /// 设置恢复：必要时先建交叉签名身份，再建密钥备份和秘密存储，生成恢复密钥，放进
    /// `EncryptionStatus::recovery_key` 给主人抄下。
    SetUpRecovery,
    /// 重置恢复：删掉账号的密钥备份和秘密存储，换新的交叉签名身份，本设备成为新身份下已验证的设备，再开启备份，
    /// 新的恢复密钥放进 `EncryptionStatus::recovery_key`。旧的恢复密钥作废，备份里的旧消息取不回了，其他设备要
    /// 输入新的恢复密钥。主人不用输密码，核心发的登录令牌就能向服务器确认身份。
    ResetRecovery,
    /// 主人已经把恢复密钥另外保存：核心把它从快照和本机清掉，之后不再显示。
    ConfirmRecoveryKeySaved,
    /// 用恢复密钥恢复：取回交叉签名的秘密和密钥备份，本设备随之被验证，能解密历史消息。
    RecoverEncryption { recovery_key: SecretString },
    /// 清掉 `ChatViewModel::problem`。
    DismissChatProblem,
    /// 主人点了确认卡片上的按钮：`id` 是 [`ConfirmationCard::id`](crate::ConfirmationCard::id)，`answer` 原样用
    /// [`CardChoice::answer`](crate::CardChoice::answer)。核心经隧道发出去，进展和结果都在时间线里的卡片上
    /// （[`ConfirmationView`](crate::ConfirmationView)），不进 `ChatViewModel::problem`。这张卡片已经在发、或者已经答完时
    /// 什么都不做。[`Answer::Unknown`] 返回 `InvalidInput`，家里的服务器还没连上时返回 `NotReady`。`choice_id` 原样用
    /// [`CardChoice::choice_id`](crate::CardChoice::choice_id)。
    AnswerConfirmation {
        id: String,
        answer: Answer,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        choice_id: Option<String>,
    },

    // 推送（ADR-0003 的“设计：加密推送”）。进展在 `ChatViewModel::push` 里。
    /// 平台给了（或者换了）推送地址。核心经隧道把它登记给家里的服务器，再在 Matrix 上注册指向家里推送代理的
    /// pusher；之后每次登录都重新注册一遍。地址或公钥不合法时返回 `InvalidInput`。
    SetPushEndpoint { endpoint: PushEndpoint },
    /// 平台收不了推送了：没有装分发器，分发器拒绝了登记或者注销了本 App。核心删掉家里的登记和 pusher，
    /// 把 `message` 照实放进 `ChatViewModel::push`。
    PushUnavailable { message: String },

    // 插件（ADR-0017）。网络操作都在后台执行，进展和结果在 `PluginsViewModel` 里。
    /// 主人偏好的语言（BCP 47，按优先级），壳在启动时和系统语言变了时告诉核心。家里的服务器按它挑插件的名字和说明。
    SetLanguages { languages: Vec<String> },
    /// 重新取已安装的插件和它们能不能升级。
    RefreshPlugins,
    /// 取商店的列表。`force` 为真时家里的服务器不等缓存过期，直接去来源取（下拉刷新）。
    RefreshStore { force: bool },
    /// 主人在确认页上点了“安装”：按预览安装，授予预览里列出的全部权限。插件包在预览之后变了时失败
    /// （`PluginFailure::Changed`），要重新预览。主人打开了确认页上的开关时，把预览的 `lan_webui.enabled` 改成 `true` 再发。
    InstallPlugin { preview: InstallPreview },
    /// 主人在升级确认页上点了“升级”，同 [`Action::InstallPlugin`]。
    UpgradePlugin { preview: InstallPreview },
    /// 卸载。`delete_data` 为真时同时删除它的数据、密钥和 Bot 账号，不可恢复。
    UninstallPlugin { id: String, delete_data: bool },
    /// 清掉一个做完的任务（成功或失败的提示）。
    DismissPluginTask { id: String },

    // 出网代理的设置页（ADR-0023）。网络操作都在后台执行，进展和结果在 `PluginsViewModel.egress` 里。
    /// 打开设置页：取设置。探测结果太旧时家里的服务器在后台再测，这里接着取几次，把新结果带回来。
    RefreshEgressProxy,
    /// 主人点了保存。`address` 空着表示取消代理；`direct_cidrs` 用逗号或空白分隔。要重启插件时先放进
    /// `egress.confirmation`，等 [`Action::ConfirmEgressProxy`]。
    SaveEgressProxy {
        address: String,
        direct_cidrs: String,
    },
    /// 主人确认了 `egress.confirmation` 列出的重启。
    ConfirmEgressProxy,
    /// 主人没有确认，什么都不改。
    DismissEgressConfirmation,

    /// 主人关掉了配对后弹出的填资料页面（[`PluginsViewModel::owner_profile_prompt`](crate::PluginsViewModel::owner_profile_prompt)）。
    DismissOwnerProfilePrompt,

    // 新用户引导（ADR-0034）。网络操作都在后台执行，进展和结果在 `OnboardingViewModel` 里。
    /// 取引导的状态、推荐清单和服务商列表。
    RefreshOnboarding,
    /// 主人在设置里选了重看：从头走一遍，已经做过的不会撤销。
    RestartOnboarding,
    /// 选中一个 Agent（[`OnboardingAgentRow::id`](crate::OnboardingAgentRow::id)）。要安装时壳接着走现有的确认页。
    ChooseOnboardingAgent { id: String },
    /// 做完这一步，到下一步。选 Agent 这一步同时把选中的记成默认 Agent。
    ContinueOnboarding,
    /// 跳过这一步，不算做完：没接模型时 Agent 的私聊照常说明要接模型。
    SkipOnboardingStep,
    /// 选中或者取消一项需求。
    ToggleOnboardingNeed { id: String },
    /// 保存一个服务商的 key 并测试。
    SaveModelProviderKey { id: String, key: SecretString },
    /// 清掉 [`OnboardingModelStep::problem`](crate::OnboardingModelStep::problem)。
    DismissModelProviderProblem,
    /// 在设置里改默认 Agent。
    SetPrimaryAgent { id: String },
    /// 主人回答了要不要卸掉被换下来的 Agent，不再问。
    DismissReplacedAgentOffer,
    /// 这条提示看过了，这台设备不再显示。
    DismissOnboardingTip { tip: OnboardingTip },

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
    DownloadFile { id: FileId },
    /// 取消进行中、暂停或者链接没发出的传输，并从列表里拿掉；上传时连服务器上传了一半的内容一起删，字节已经传完的不删，文件留在
    /// 服务器上。已经做完或失败的不受影响：取消和传完撞上时不算错。`id` 是 [`Transfer::id`](crate::Transfer::id)，不是文件编号。
    CancelTransfer { id: String },
    /// 从列表里清掉不再进行的传输。服务器上的文件不受影响，要删用 [`Action::DeleteFile`]。`delete_local_file` 为真时连 App
    /// 目录里下载的文件一起删，只对下载有效，上传时返回 `InvalidInput`：手机上的原文件不归 App 管。为假时文件留着，卡片还能
    /// 打开。还在进行的传输要先取消，否则返回 `InvalidInput`。
    DismissTransfer { id: String, delete_local_file: bool },
    /// 只重发链接：再授权一次，再把链接发进上传时的房间，不重传文件。只对
    /// [`TransferState::LinkNotSent`](crate::TransferState::LinkNotSent) 的传输有效，其他返回 `InvalidInput`。
    ResendFileLink { id: String },
    /// 删掉编号。设备上传的文件核对没变过后连文件一起删，其他来源的原文件不动。
    DeleteFile { id: FileId },
    /// 把链接发到和主人的“Wonton”私聊，主人所有设备都看得到卡片。
    SendFileToDevices { id: FileId },
    /// 把这个房间的 Bot 加进文件的访问范围。在 Bot 的房间里发链接时一起调，Bot 才能看和再分享。
    GrantFileToRoom { id: FileId, room: String },
    /// 清掉 [`TransfersViewModel::problem`](crate::TransfersViewModel::problem)。
    DismissFileProblem,

    // 核心的一键升级（ADR-0025）。网络操作都在后台执行，进展和结果在 `CoreUpgradeViewModel` 里。
    /// 取升级状态、查新版本。打开升级页时 `manual` 为假，主人点“检查”时为真。
    RefreshCoreUpgrade { manual: bool },
    /// 主人在确认页上点了“升级”。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    StartCoreUpgrade { digest: String },
    /// 请求取消，只在 [`CoreUpgradePhase::Pulling`](crate::CoreUpgradePhase::Pulling) 阶段有效。
    CancelCoreUpgrade,
    /// 这台设备不再提示这个版本。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    SkipCoreUpgrade { digest: String },

    // 外观（ADR-0026、ADR-0028）。设置存在本机，写不了时返回 `Storage`，外观不变。
    /// 把重点色换成这个颜色。任何颜色都可以，读字的颜色由核心推导。
    SetAccentColor { color: Rgb },
    /// 选明暗。
    SetThemeMode { mode: ThemeMode },
    /// 选字号。
    SetTextSize { size: TextSize },
    /// 选字体，只从 [`AppearanceViewModel::fonts`](crate::AppearanceViewModel::fonts) 里选。
    SetFont { font: FontStyle },

    // 底栏（ADR-0033）。配置存在本机，写不了时返回 `Storage`，底栏不变。
    /// 打开一个页面。插件页已固定时切到它的标签，没固定时盖在主界面上（[`TabsViewModel::overlay`](crate::TabsViewModel::overlay)）。
    SelectTab { tab: TabRef },
    /// 关掉盖在主界面上的插件页。
    CloseTabOverlay,
    /// 固定到底栏。栏满了（[`TabsViewModel::full`](crate::TabsViewModel::full)）要给 `replacing`，否则返回 `InvalidInput`。
    PinTab {
        tab: TabRef,
        replacing: Option<TabRef>,
    },
    /// 从栏上拿掉。聊天和应用不能拿，返回 `InvalidInput`。
    UnpinTab { tab: TabRef },
    /// 把一项移到 `index`（移动后的下标，从 0 开始）。
    MoveTab { tab: TabRef, index: u32 },
    /// 给插件页选打开方式，不是插件页时返回 `InvalidInput`。
    SetTabDisplay {
        tab: TabRef,
        display: TabDisplayChoice,
    },
    /// 全屏显示插件页。主人把这个页面设成了 `Standard` 也可以手动进。
    EnterFullscreen { tab: TabRef },
    /// 退出全屏。手动进的回到带栏的样子；打开就全屏的页面回到进来之前的标签，没有就回第一项。
    ExitFullscreen { tab: TabRef },
    /// 重新加载插件页（重试、“···”里的重新加载）。
    ReloadTab { tab: TabRef },
    /// 壳报告网页加载失败。`status` 是 HTTP 状态码，没拿到响应时为空；原因由核心判断。
    TabLoadFailed { tab: TabRef, status: Option<u16> },
    /// 主人看过了“已卸载，已从底栏移除”的提示。
    DismissUninstalledTabs,
}
