//! 打开的房间的时间线：按增量推送（ADR-0005 的验证结果）。
//!
//! 时间线可能有成百上千条，每次变化都推整份太浪费，所以推的是增量批次，不能合并：壳要按顺序把每一批应用到
//! 自己持有的列表上。第一批总是 [`TimelineDiff::Reset`]，给出当前的全部条目。条目和 matrix-rust-sdk 的时间线
//! 一一对应（含日期分隔线、已读标记等），下标就是 SDK 的下标；哪些事件进时间线由 `wonton-matrix` 的事件过滤决定。
//!
//! 列表该停在哪也由核心说：壳用 [`TimelineAction::SetTimelineScroll`](crate::TimelineAction::SetTimelineScroll) 报告是不是离开了底部，
//! 核心在每一批里给出重新算过的位置和新消息数（[`TimelineUpdate::viewport`]），并在第一批给出该停在哪一条
//! （[`TimelineUpdate::anchor`]）。壳只按这两个值画和滚，不自己数。
//!
//! 还有解不开的消息时，每一批都带着该显示的加密入口（[`TimelineUpdate::key_notice`]，[`KeyNotice`]）：核心按账号的
//! 恢复状态决定是输恢复密钥还是请别的设备确认，加密状态变了就补发一批空的。壳只按类别写文字和按钮，不自己判断。

use serde::{Deserialize, Serialize};

use crate::{
    AgentRunChoice, AgentRunEnd, AgentRunState, ConfirmationView, FileId, QuotedMessage, ReplyQuote,
};

/// 一批时间线变化，以及向前翻页的状态和列表该停在哪。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineUpdate {
    /// 按顺序应用的增量；只有翻页状态或位置变化时为空。
    pub diffs: Vec<TimelineDiff>,
    pub back_pagination: BackPagination,
    /// 列表该停在哪。壳按它画“回到底部”和按钮上的数字；新消息不把列表拽走。
    pub viewport: TimelineViewport,
    /// 自己发出的新消息到了：壳应用这一批后回到底部，即使主人翻走了。
    /// 别人的消息和编辑只在原本贴底时跟随（[`TimelineViewport::Bottom`]），不能打断阅读。
    pub scroll_to_bottom: bool,
    /// 只有带 `Reset` 的那一批有（订阅的第一批，时间线重建后的第一批），其余为 `None`：壳先把这一条
    /// （`Reset` 里的 `TimelineItem::id`）摆到视野顶部，摆好之前不报 [`TimelineScroll::Bottom`]
    /// （报早了核心以为主人已经看到最新一条，会清掉落点）。
    ///
    /// 锚点是已读位置，主人一眼看到“以下是新消息”；没有未读时是最新一条，空房间为空。这一条摆过去之前就没了
    /// （撤回、被裁掉）时，壳停在最新一条。
    ///
    /// 这一批里有 [`TimelineContent::ReadMarker`] 就锚它自己（`__read_marker`），那一行正好在已读和新消息之间。
    /// 没有这一行时（SDK 还没插上，或者未读多到落点被加载窗口截断）按本账号已读位置算：位置在这一批里就锚它后面
    /// 第一条，不在就从末尾往回数未读条数、数不到就锚最早一条，让主人接着往前翻。
    ///
    /// 有没有未读另看未读数：有 `ReadMarker` 一定还有未读，没有它却可能只是读完了、未读被窗口截断或者标记还没插上。
    pub anchor: Option<String>,
    /// 打开的房间的面板（ADR-0052）：每一批都给当前的列表，空的时候壳画大加号。
    #[serde(default)]
    pub panel: Vec<crate::PanelItem>,
    /// 房间面板上次看的那一页（本机记住，不同步），下次拉开还是它。没记住时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_selected: Option<crate::TabRef>,
    /// 房间里另有东西开着（长按菜单、多选模式、转发或删除确认、交给 Bot 的选择、斜杠命令列表）：
    /// 上推拉面板的入口不响应。壳只读它，不自己合成；状态由
    /// [`Action::SetRoomEntryBlocked`](crate::Action::SetRoomEntryBlocked) 报告，换房间后按没盖算。
    #[serde(default)]
    pub entry_blocked: bool,
    /// 时间线里有本设备解不开的消息时，给主人的下一步；没有或者这台设备已经有密钥时长这样。
    ///
    /// 由核心按账号的恢复状态给出，壳在输入栏上方显示一行说明和按钮，按类别跳到备份页。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_notice: Option<KeyNotice>,
    /// 显示在这个房间的确认卡片（ADR-0016）：等回答的，以及这台设备这次打开 App 后看着结束的，最早到期的在前。
    /// 每一批都给当前的列表。壳画在输入栏上方，点开是完整的卡片，能直接回答。话题里发起的也在这里。
    #[serde(default)]
    pub confirmations: Vec<ConfirmationView>,
    /// 旁聊订阅的头部（ADR-0093）：这条时间线是旁聊时给标题和状态，主时间线为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_chat: Option<crate::SideChatCard>,
}

/// 解不开的消息旁边的入口，壳按它写按钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum KeyNotice {
    /// 账号设过密钥备份，本设备还没拿到密钥：输入恢复密钥就能解开。
    EnterRecoveryKey,
    /// 账号没设密钥备份：让账号里已验证的设备确认本设备，那台设备才会把密钥发过来。
    VerifyWithOtherDevice,
    /// 更新的核心给出的类别：显示说明，不给按钮。
    #[serde(other)]
    Other,
}

/// 列表该停在哪，方向是核心到壳。核心按已读位置、壳报告的滚动（[`TimelineAction::SetTimelineScroll`](crate::TimelineAction::SetTimelineScroll)）
/// 和条目算出。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineViewport {
    /// 贴着最新一条。新消息照常往这里来，已读回执照发。
    Bottom,
    /// 翻到别处了，壳在右下角画“回到底部”按钮。`new_messages` 是房间的未读数（和聊天列表那一行同一个数）：
    /// 是 0 时只画按钮不写数字。点按钮滚到最新一条，壳随后报告 [`TimelineScroll::Bottom`]，`new_messages` 变回 0。
    Away { new_messages: u32 },
}

/// 壳报告列表是不是离开了底部，方向是壳到核心。跨过底部时发，不用每一帧发。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineScroll {
    /// 贴着最新一条：壳照发已读回执，核心把 [`TimelineViewport`] 变成 [`TimelineViewport::Bottom`]、`new_messages`
    /// 归零。第一批的 [`TimelineUpdate::anchor`] 落定之前不要报，免得把落点当成到底了。
    Bottom,
    /// 翻到别处了：已读位置不动，之后到的消息都算新消息。
    Away,
}

/// 对条目列表的一次修改，语义同 `eyeball-im` 的 `VectorDiff`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineDiff {
    /// 在末尾追加多条。
    Append {
        items: Vec<TimelineItem>,
    },
    /// 清空。
    Clear,
    PushFront {
        item: TimelineItem,
    },
    PushBack {
        item: TimelineItem,
    },
    PopFront,
    PopBack,
    /// 在 `index` 处插入，原来的条目后移。
    Insert {
        index: u32,
        item: TimelineItem,
    },
    /// 替换 `index` 处的条目（同一条消息的内容、发送状态或已读者变了）。
    Set {
        index: u32,
        item: TimelineItem,
    },
    Remove {
        index: u32,
    },
    /// 只保留前 `length` 条。
    Truncate {
        length: u32,
    },
    /// 整个换成 `items`。
    Reset {
        items: Vec<TimelineItem>,
    },
}

/// 时间线里的一条。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineItem {
    /// 在这条时间线里稳定的标识，可以直接用作列表的 ID：本地回显发出后、内容被编辑后都不变。
    pub id: String,
    pub content: TimelineContent,
}

/// 条目的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized by value across FFI, so variant sizes do not matter; UniFFI record fields do not support Box"
)]
pub enum TimelineContent {
    Message {
        message: TimelineMessage,
    },
    /// Agent 一轮任务的进度（MSC4564，ADR-0082）：Bot 发的一条状态消息，核心按最新编辑解读。
    /// 只有核心认得的 Bot 账号发的才是 AgentRun，别人发的同样内容是 [`TimelineContent::Message`]。
    /// 字段是发送者的自述，客户端从不自动作答；聊天列表预览跳过这种条目。
    AgentRun {
        /// 这一轮针对的请求消息的事件 ID（`run`）。
        run: Option<String>,
        state: AgentRunState,
        /// 当前步骤的一句话（`step`），Bot 给的原话，原样显示。
        step: Option<String>,
        /// 到现在完成的步数（`steps`）。
        steps: Option<u32>,
        /// 进度，`0.0..=1.0`（`progress`）。
        progress: Option<f32>,
        /// 结束时的一句话总结（`result`），Bot 给的原话，原样显示。
        result: Option<String>,
        /// 怎么结束的（`app.wonton.end`），只有状态是 [`AgentRunState::Failed`] 时有。
        end: Option<AgentRunEnd>,
        /// 状态消息的 `body`：写给不认识这个格式的客户端的纯文字，状态是
        /// [`AgentRunState::Other`] 时壳原样显示它。
        body: String,
        /// 状态是 [`AgentRunState::Waiting`] 且这个 Bot 在这个房间有等回答的核心卡片（ADR-0016）时，
        /// 是那张卡片：点条目打开它。
        card_id: Option<String>,
        /// Bot 自己发的待回答选项（`choices`），只在等待回答时有；点了一个经
        /// [`Action::AnswerAgentChoice`](crate::Action::AnswerAgentChoice) 以主人的身份发反应。
        choices: Vec<AgentRunChoice>,
    },
    /// 一行说明，比如成员加入、离开，房间改名。文字由核心给出。
    Notice {
        text: String,
    },
    /// 有人改了名字或头像（ADR-0044），文字由壳写：名字变了是“旧名 改名为 新名”，只换了头像是“某某 换了头像”。
    ProfileChange {
        user_id: String,
        /// 改之后的名字。
        name: String,
        /// 改之前的名字；名字没变时为空。
        previous_name: Option<String>,
        avatar_changed: bool,
    },
    /// 日期分隔线，`timestamp_ms` 是这一天里第一条消息的时间（Unix 毫秒），壳按本地时区显示日期。
    DateDivider {
        timestamp_ms: u64,
    },
    /// “以下是新消息”：本账号的已读位置。
    ReadMarker,
    /// 已经翻到房间的开头。
    TimelineStart,
    /// 旁聊根（ADR-0093）：带 `app.wonton.side_chat` 的 `m.notice`，只显示成卡片，不进输入框回复。
    SideChat {
        card: crate::SideChatCard,
    },
    /// 旁聊结果行（ADR-0093）：主时间线里单独的一种条目；同一个旁聊只留最新一条，更早的不显示。
    SideChatResult {
        root_event_id: String,
        title: String,
        state: crate::SideChatState,
    },
}

/// 一条消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineMessage {
    /// 服务器分配的事件 ID；本地回显还没发出去时为空。
    pub event_id: Option<String>,
    /// 发送者的 Matrix 用户 ID。
    pub sender_id: String,
    /// 发送者的显示名，没有设置时是用户 ID。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
    pub send_state: SendState,
    pub body: MessageContent,
    /// 是否被编辑过（显示的是最新内容）。Bot 的流式回复靠编辑消息实现。
    pub edited: bool,
    /// 读过这条的其他人（显示名）。只有自己发的、别人读到的最后一条才有，壳有就显示“已读”。
    pub read_by: Vec<String>,
    /// 这条回复的是哪条消息，壳画在正文上方；不是回复时为空。
    pub reply: Option<ReplyQuote>,
    /// 这条消息挂着一个旁聊卡片（ADR-0093）：普通消息是有回复时的卡片，带 `app.wonton.side_chat`
    /// 的根只显示成卡片。不是卡片时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_chat: Option<crate::SideChatCard>,
    /// 第一个链接的预览卡片（ADR-0065）：事件里带的直接用，没有的向家里的核心取；没有链接、还没取到或取不到时
    /// 按 [`crate::LinkPreviewState`] 给。
    pub preview: Option<crate::LinkPreviewState>,
    /// 长按这条消息能做的事，按菜单顺序排好。空的就没有菜单。
    pub actions: Vec<MessageAction>,
    /// 一次发送里的几条之一（ADR-0094）：壳把同一发送者、同一 `id`、挨着的几条画在一起。不是一组时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub group: Option<crate::MessageGroup>,
}

/// 长按消息的菜单里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAction {
    /// 把 `text` 放进剪贴板。
    Copy { text: String },
    /// 回复这条：壳把 `message` 画在输入框上方，发送时把它的事件 ID 作为 `reply_to` 交回，并用
    /// [`TimelineAction::SetDraft`](crate::TimelineAction::SetDraft) 记进草稿。有这一项时壳也允许右滑这条消息来回复。
    Reply { message: QuotedMessage },
    /// 转发到另一个房间（[`TimelineAction::ForwardMessage`](crate::TimelineAction::ForwardMessage)），壳先让主人选房间。
    Forward,
    /// 撤回（[`TimelineAction::RedactMessage`](crate::TimelineAction::RedactMessage)）：所有人那里都会变成“已撤回”，壳先请主人确认。
    Redact,
}

/// 发送状态。别人的消息和从服务器同步来的消息都是 [`SendState::Sent`]。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SendState {
    /// 本地回显，在排队或正在发送。排在失败消息后面时，直到那条发出或取消才继续。
    /// `progress` 是核心合并原图和缩略图后算好的上传比例，范围 `0.0..=1.0`；`None` 表示不确定（文字、排队或总量未知）。
    /// 重试、重发或重启后可能倒退或回到 `None`，壳原样绘制，不自行换算或保持单调。
    /// `Some(1.0)` 不代表发送成功，成功和失败分别由 [`SendState::Sent`] 和 [`SendState::Failed`] 表示。
    Sending {
        progress: Option<f32>,
    },
    Sent,
    /// 发送失败，`message` 是原因。没发出去的都可以取消（[`TimelineAction::CancelSend`](crate::TimelineAction::CancelSend)）。
    ///
    /// - `recoverable` 为真：连接出了问题（连不上、超时、服务器暂时出错）。核心隔一段时间自动重发，连续失败时间隔
    ///   逐渐拉长，直到发出去；也可以马上重发（[`TimelineAction::RetrySend`](crate::TimelineAction::RetrySend)）。
    /// - 为假：服务器拒收（比如太大、没有权限）。核心不自动重发，同一房间之后发的消息都排在它后面，直到主人重发或者
    ///   取消这一条。
    Failed {
        message: String,
        recoverable: bool,
    },
}

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized by value across FFI, so variant sizes do not matter; UniFFI record fields do not support Box"
)]
pub enum MessageContent {
    /// 文字，包括 Bot 常用的 `m.notice`。`body` 是去掉文件链接后的原文，消息只有链接时为空；`blocks` 是它排好版的样子，
    /// 壳画这个，不解析 Markdown；`files` 是认出的文件，壳在消息下面按编号画卡片（ADR-0020）。
    Text {
        body: String,
        blocks: Vec<RichBlock>,
        files: Vec<FileId>,
    },
    Image {
        image: ImageContent,
    },
    /// 确认卡片（ADR-0016）。只有核心账号发的才是卡片，别人发的同样结构的内容是 [`MessageContent::Text`]。卡片靠编辑
    /// 消息更新状态，[`TimelineMessage::edited`] 总是假。
    Confirmation {
        view: ConfirmationView,
    },
    /// 端到端加密的消息，本设备还没有解密密钥；拿到密钥后核心会把它换成解密后的内容。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 还不支持显示的类型（文件、语音、投票等），`kind` 是消息或事件类型。
    Unsupported {
        kind: String,
    },
}

/// 排好版的一块文字。没有嵌套：列表和引用的层次摊平成每一块的 `depth` 和 `quoted`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichBlock {
    pub kind: RichBlockKind,
    /// 缩进几层。
    pub depth: u32,
    /// 在引用里。
    pub quoted: bool,
    pub spans: Vec<RichSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RichBlockKind {
    Paragraph,
    Heading,
    /// `marker` 是项目符号、序号或任务框，壳原样画在文字前面。
    ListItem {
        marker: String,
    },
    /// 代码块：等宽字体，不折行，横向滚动。
    Code,
    /// 表格，第一行是表头，每行格数相同。不折行，横向滚动。
    Table {
        rows: Vec<Vec<String>>,
    },
    /// 分隔线，`spans` 为空。
    Rule,
}

/// 一段样式相同的文字。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<RichLink>,
}

/// 文字里的一个链接，以及核心判断的类别：壳按类别打开，不自己看地址（ADR-0065）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichLink {
    /// 打开或交给系统的地址。不带协议的网址已经补成 http，纯文本邮箱补成 `mailto:`，号码补成 `tel:`。
    pub url: String,
    pub kind: RichLinkKind,
}

/// 链接的类别，决定点开以后去哪里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RichLinkKind {
    /// 别处的网页：iOS 用 `SFSafariViewController`、Android 用 Custom Tabs 打开。
    External,
    /// 这台服务器上插件的网页：打开那个插件的页面，经隧道。
    Plugin { plugin_id: String },
    /// matrix.to 的链接：在 App 里打开对应的用户或房间。
    Matrix { target: MatrixTarget },
    /// 邮件：交给系统。
    Email,
    /// 电话：交给系统。
    Phone,
    /// 其他交给系统的地址，比如云吞自己的 `wonton://`。
    Other,
}

/// matrix.to 链接指向的东西。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MatrixTarget {
    /// 一个用户。
    User { user_id: String },
    /// 一个房间：房间 ID，或房间别名（`#alias:server`）。`event_id` 是链接指向的事件（事件永久链接），只有房间时为 `None`。
    Room {
        room_id: String,
        event_id: Option<String>,
    },
}

/// 图片消息。原图和缩略图都经 `ClientCore::load_media` 取，核心负责下载、解密和缓存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageContent {
    /// 原图。
    pub media: MediaHandle,
    /// 发送端给的缩略图；时间线里先显示它，点开再取原图。没有时为空。
    pub thumbnail: Option<MediaHandle>,
    pub filename: String,
    /// MIME 类型，发送方没有声明时为空。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 像素宽度，壳据此预留版面。
    pub width: Option<u64>,
    pub height: Option<u64>,
    /// 说明文字（Matrix v1.10 的媒体说明：`filename` 和 `body` 不同时 `body` 是说明），壳画在图片下面。没有时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub caption: Option<ImageCaption>,
}

/// 图片的说明文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageCaption {
    /// 原文，复制时用。
    pub body: String,
    /// 排好版的样子，壳画这个，同 [`MessageContent::Text`] 的 `blocks`。
    pub blocks: Vec<RichBlock>,
}

/// 取媒体用的句柄。对壳是不透明的，原样交回核心即可；里面可能有加密附件的密钥，不要写进日志。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaHandle {
    pub token: String,
}

impl std::fmt::Debug for MediaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaHandle(<redacted>)")
    }
}

/// 向前翻页（加载更早的消息）的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackPagination {
    /// 没在翻页，还有更早的消息。
    Idle,
    Paginating,
    /// 已经到了房间的开头。
    ReachedStart,
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::{
        Answer, CardChoice, CardLine, ChatFailure, ChatProblem, ConfirmationCard, ConfirmationState,
    };

    /// 条目列表的样例，从下面的往返测试里挪出来，函数保持在门禁行数内。
    fn sample_items() -> Vec<TimelineItem> {
        let media = MediaHandle {
            token: "mxc://home.example/abc#key".into(),
        };
        vec![
            item("1", TimelineContent::TimelineStart),
            item(
                "2",
                TimelineContent::DateDivider {
                    timestamp_ms: 1_793_000_000_000,
                },
            ),
            item(
                "3",
                message(
                    MessageContent::Image {
                        image: ImageContent {
                            media: media.clone(),
                            thumbnail: Some(media.clone()),
                            filename: "cat.jpg".into(),
                            mime_type: Some("image/jpeg".into()),
                            size: Some(4096),
                            width: Some(640),
                            height: Some(480),
                            caption: Some(ImageCaption {
                                body: "哪张好看".into(),
                                blocks: Vec::new(),
                            }),
                        },
                    },
                    SendState::Sending {
                        progress: Some(0.5),
                    },
                ),
            ),
            item(
                "4",
                message(
                    MessageContent::Undecryptable,
                    SendState::Sending { progress: None },
                ),
            ),
            item(
                "4a",
                message(confirmation(Some(Answer::Approve), None), SendState::Sent),
            ),
            item(
                "4b",
                message(
                    confirmation(
                        None,
                        Some(ChatProblem {
                            reason: ChatFailure::TransportUnavailable,
                            message: "连不上".into(),
                        }),
                    ),
                    SendState::Sent,
                ),
            ),
            item(
                "4c",
                TimelineContent::AgentRun {
                    run: Some("$request".into()),
                    state: AgentRunState::Waiting,
                    step: Some("执行命令".into()),
                    steps: Some(3),
                    progress: Some(0.5),
                    result: None,
                    end: None,
                    body: "等你确认".into(),
                    card_id: Some("card-1".into()),
                    choices: vec![AgentRunChoice {
                        key: "allow".into(),
                        label: "继续".into(),
                    }],
                },
            ),
            item("5", TimelineContent::ReadMarker),
            item(
                "6",
                TimelineContent::Notice {
                    text: "Hermes 离开了房间".into(),
                },
            ),
        ]
    }

    /// 契约：时间线增量的每种变化和每种条目都能经 JSON 往返；媒体句柄不进 Debug 输出。
    #[test]
    fn timeline_updates_round_trip_through_json() {
        let items = sample_items();
        let update = TimelineUpdate {
            diffs: vec![
                TimelineDiff::Reset {
                    items: items.clone(),
                },
                TimelineDiff::Append { items },
                TimelineDiff::Set {
                    index: 2,
                    item: item(
                        "3",
                        message(
                            text_with_file(),
                            SendState::Failed {
                                message: "网络断开".into(),
                                recoverable: true,
                            },
                        ),
                    ),
                },
                TimelineDiff::Insert {
                    index: 0,
                    item: item("0", message(MessageContent::Redacted, SendState::Sent)),
                },
                TimelineDiff::Remove { index: 1 },
                TimelineDiff::Truncate { length: 3 },
                TimelineDiff::PopFront,
                TimelineDiff::Clear,
            ],
            back_pagination: BackPagination::ReachedStart,
            viewport: TimelineViewport::Away { new_messages: 3 },
            scroll_to_bottom: false,
            anchor: Some("5".into()),
            panel: Vec::new(),
            panel_selected: None,
            entry_blocked: true,
            key_notice: Some(KeyNotice::EnterRecoveryKey),
            confirmations: Vec::new(),
            side_chat: None,
        };
        assert_eq!(round_trip(&update), update);
        assert!(
            !format!("{update:?}").contains("mxc://"),
            "Debug 不能输出媒体句柄"
        );
    }

    /// 契约：滚动位置双向都能经 JSON 往返，壳和核心对“在哪儿”的每个取值都认。
    #[test]
    fn every_scroll_and_viewport_round_trips_through_json() {
        for scroll in [TimelineScroll::Bottom, TimelineScroll::Away] {
            assert_eq!(round_trip(&scroll), scroll);
        }
        for viewport in [
            TimelineViewport::Bottom,
            TimelineViewport::Away { new_messages: 0 },
            TimelineViewport::Away { new_messages: 27 },
        ] {
            assert_eq!(round_trip(&viewport), viewport);
        }
        assert_eq!(
            serde_json::to_value(TimelineViewport::Away { new_messages: 2 }).unwrap(),
            serde_json::json!({"type": "away", "new_messages": 2})
        );
    }

    /// 契约：加密入口的每个取值都能经 JSON 往返，不认识的取值落到 `Other`，没有入口时整个字段可以缺。
    #[test]
    fn key_notices_round_trip_and_unknown_ones_fall_back() {
        for notice in [
            KeyNotice::EnterRecoveryKey,
            KeyNotice::VerifyWithOtherDevice,
        ] {
            assert_eq!(round_trip(&notice), notice);
        }
        assert_eq!(
            serde_json::from_value::<KeyNotice>(serde_json::json!("another")).unwrap(),
            KeyNotice::Other
        );
        let empty = || TimelineUpdate {
            diffs: Vec::new(),
            back_pagination: BackPagination::Idle,
            viewport: TimelineViewport::Bottom,
            scroll_to_bottom: true,
            anchor: None,
            panel: Vec::new(),
            panel_selected: None,
            entry_blocked: false,
            key_notice: Some(KeyNotice::VerifyWithOtherDevice),
            confirmations: Vec::new(),
            side_chat: None,
        };
        let mut value = serde_json::to_value(empty()).unwrap();
        value.as_object_mut().unwrap().remove("key_notice");
        let update: TimelineUpdate = serde_json::from_value(value).unwrap();
        assert_eq!(update.key_notice, None);
    }

    fn text_with_file() -> MessageContent {
        MessageContent::Text {
            body: "**你好**".into(),
            blocks: vec![RichBlock {
                kind: RichBlockKind::ListItem {
                    marker: "•".into()
                },
                depth: 1,
                quoted: true,
                spans: vec![RichSpan {
                    text: "你好".into(),
                    bold: true,
                    link: Some(RichLink {
                        url: "https://example.com/".into(),
                        kind: RichLinkKind::External,
                    }),
                    ..RichSpan::default()
                }],
            }],
            files: vec![FileId::from_bytes([7; 16])],
        }
    }

    fn confirmation(sending: Option<Answer>, problem: Option<ChatProblem>) -> MessageContent {
        MessageContent::Confirmation {
            view: ConfirmationView {
                card: ConfirmationCard {
                    secret_inputs: Vec::new(),
                    settings: None,
                    requester: None,
                    origin: None,
                    audience: crate::CardAudience::Keeper,
                    answered_by: None,
                    category: None,
                    id: "abc".into(),
                    state: ConfirmationState::Pending,
                    title: "Hermes 想安装 Grafana".into(),
                    lines: vec![CardLine {
                        text: "能访问 Docker".into(),
                        emphasis: true,
                    }],
                    choices: vec![CardChoice {
                        answer: Answer::Approve,
                        label: "安装".into(),
                        choice_id: None,
                    }],
                    expires_at_ms: 1_793_000_600_000,
                    plugin_id: None,
                    window: None,
                },
                sending,
                sending_choice_id: None,
                problem,
                window: None,
            },
        }
    }

    #[test]
    fn sending_progress_has_a_stable_json_contract() {
        for (state, json) in [
            (
                SendState::Sending {
                    progress: Some(0.5),
                },
                serde_json::json!({"type": "sending", "progress": 0.5}),
            ),
            (
                SendState::Sending { progress: None },
                serde_json::json!({"type": "sending", "progress": null}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&state).unwrap(), json);
            assert_eq!(serde_json::from_value::<SendState>(json).unwrap(), state);
        }
        let state: SendState = serde_json::from_str(r#"{"type":"sending"}"#).unwrap();
        assert_eq!(state, SendState::Sending { progress: None });
    }

    /// 契约：每个链接类别都能经 JSON 往返，壳收到的类别和地址都还在。
    #[test]
    fn every_link_kind_round_trips_through_json() {
        for kind in [
            RichLinkKind::External,
            RichLinkKind::Plugin {
                plugin_id: "app.grafana".into(),
            },
            RichLinkKind::Matrix {
                target: MatrixTarget::User {
                    user_id: "@alice:home.example".into(),
                },
            },
            RichLinkKind::Matrix {
                target: MatrixTarget::Room {
                    room_id: "!abc:home.example".into(),
                    event_id: None,
                },
            },
            RichLinkKind::Matrix {
                target: MatrixTarget::Room {
                    room_id: "#room:home.example".into(),
                    event_id: Some("$event".into()),
                },
            },
            RichLinkKind::Email,
            RichLinkKind::Phone,
            RichLinkKind::Other,
        ] {
            let link = RichLink {
                url: "https://example.com/".into(),
                kind,
            };
            assert_eq!(round_trip(&link), link);
        }
    }

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    fn message(body: MessageContent, send_state: SendState) -> TimelineContent {
        TimelineContent::Message {
            message: TimelineMessage {
                event_id: Some("$event".into()),
                sender_id: "@hermes:home.example".into(),
                sender_name: "Hermes".into(),
                is_own: false,
                timestamp_ms: 1_793_000_000_000,
                send_state,
                body,
                edited: true,
                read_by: vec!["主人".into()],
                reply: None,
                side_chat: None,
                preview: None,
                actions: vec![MessageAction::Copy {
                    text: "好的".into(),
                }],
                group: Some(crate::MessageGroup {
                    id: "3f0c9a1e5b7d42c6a8e1f0b2c4d6e8a0".into(),
                    index: 0,
                    count: 2,
                }),
            },
        }
    }

    fn item(id: &str, content: TimelineContent) -> TimelineItem {
        TimelineItem {
            id: id.into(),
            content,
        }
    }
}
