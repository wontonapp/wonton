//! 一批时间线变化，以及解不开的消息旁边的入口。

use serde::{Deserialize, Serialize};

use super::diff::TimelineDiff;
use super::position::{BackPagination, TimelineViewport};
use super::unread::UnreadJump;
use crate::ConfirmationView;

/// 一批时间线变化，以及向前翻页的状态和列表该停在哪。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineUpdate {
    /// 按顺序应用的增量；只有翻页状态或位置变化时为空。
    pub diffs: Vec<TimelineDiff>,
    pub back_pagination: BackPagination,
    /// 列表该停在哪。壳按它画“回到最新”和按钮上的数字；新消息不把列表拽走。
    pub viewport: TimelineViewport,
    /// 自己发出的新消息到了：壳应用这一批后回到底部，即使主人翻走了。
    /// 别人的消息和编辑只在原本贴底时跟随（[`TimelineViewport::Bottom`]），不能打断阅读。
    pub scroll_to_bottom: bool,
    /// 只有带 `Reset` 的那一批有（订阅的第一批，时间线重建后的第一批），其余为 `None`：壳先把这一条
    /// （`Reset` 里的 `TimelineItem::id`）摆到视野顶部，摆好之前不报 [`TimelineScroll::Bottom`]
    /// （报早了核心以为主人已经看到最新一条，会清掉落点）。
    ///
    /// 锚点是最新一条，进房停在底部看最新一句（ADR-0121）；空房间为空。这一条摆过去之前就没了（撤回、被裁掉）时，
    /// 壳停在最新一条。未读的“以下是新消息”由 [`TimelineUpdate::unread_jump`](crate::TimelineUpdate::unread_jump)
    /// 的会话锚点单独画，不再决定这里的落点。
    pub anchor: Option<String>,
    /// 打开的房间的面板（ADR-0052）：每一批都给当前的列表，空的时候壳画大加号。
    #[serde(default)]
    pub panel: Vec<crate::PanelItem>,
    /// 房间面板上次看的那一页（本机记住，不同步），下次拉开还是它。没记住时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_selected: Option<crate::TabRef>,
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
    /// 房间顶栏的 widget 入口（ADR-0096）：声明了 `matrix_widget`、在运行的插件，按安装顺序。空的时候不画按钮。
    #[serde(default)]
    pub widgets: Vec<crate::RoomWidget>,
    /// 房间里有人在通话（`m.call.member` 里有没过期的通话成员）：入口显示“通话中”，点了就是加入。
    #[serde(default)]
    pub call_active: bool,
    /// 房间里在场的 AI（本账号认识的 Bot 的 Matrix 用户 ID）：按房间成员和 Bot 记录算，每批都带当前值，
    /// 变了也会补发一批。私聊不算：私聊的 Bot 顶栏本来就显示。`None` 是成员名单还没从服务器取全、在场
    /// 的算不准，壳别下“群里没有 AI”的结论；`Some` 是取全后的名单，空才是确定没有。壳据此展示
    /// “群里有谁可以用”，确定没有时说明 @ 不会有人回应。
    #[serde(default)]
    pub room_bots: Option<Vec<String>>,
    /// 房间里正在输入的其他人（显示名），按开始输入的先后排，不含本账号。开始、结束和超时听 Matrix 的正在输入状态，
    /// 这个人的消息到了核心马上去掉他。壳在顶栏状态行写“正在输入…”：私聊不写名字，群里“某某 正在输入…”，
    /// 多人时“某某 和另外 N 人”（ADR-0109）。
    #[serde(default)]
    pub typing: Vec<String>,
    /// 房间里各 Bot 的斜杠命令（ADR-0054、ADR-0109），按 Bot 加入房间的先后、表里的顺序排。空的时候不画“菜单”按钮，
    /// 输入“/”也不弹列表。
    #[serde(default)]
    pub commands: Vec<crate::RoomCommand>,
    /// 各 Bot 的表里因超过上限被截掉的条数之和（[`BotCommands::omitted`](crate::BotCommands::omitted)）。壳在列表末尾写“还有 N 条命令没列出”。
    #[serde(default)]
    pub commands_omitted: u32,
    /// 只在要壳滚到某一条的那一批里有：点引用跳到原消息，或者“向下”按钮回到跳之前的位置（ADR-0109）。
    /// 和 [`TimelineUpdate::scroll_to_bottom`] 不会同时出现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<crate::TimelineFocus>,
    /// 跳转的进展，每一批都带当前值。
    #[serde(default)]
    pub jump: crate::JumpState,
    /// 跳之前那一条（栈顶）：列表离开底部时，“向下”按钮先发 [`TimelineAction::JumpBack`](crate::TimelineAction::JumpBack)
    /// 回到这里，为空时才滚到底。壳报告回到底部（[`TimelineScroll::Bottom`]）后核心清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_to: Option<String>,
    /// 输入框正在编辑哪条（ADR-0109 第 10 节），每一批都带当前值。有值时输入框上方画一条“编辑消息”和原文摘要、
    /// 带取消，发送键变成确认；正文由壳从 [`MessageAction::Edit`] 的 `text` 预填，原草稿在取消后由核心恢复。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editing: Option<crate::ComposerEdit>,
    /// 壳最近一次报的跳转序号（[`TimelineAction::RegisterFocusRevision`](crate::TimelineAction::RegisterFocusRevision)）；
    /// 核心没收到过为 0。壳按它重滚同一条原消息。
    #[serde(default)]
    pub focus_revision: u32,
    /// 右上角“回到最早未读”的胶囊（ADR-0121）。每一批都给当前值；为空时不画。
    ///
    /// 由核心按本账号的 `m.fully_read` 和最新回执算，锚点在进房时冻结：标记已读不动它，别的设备读了也不动它。
    /// 壳只按 [`UnreadJump::anchor_item_id`] 和 `state` 画，不自己比回执、不自己数条数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub unread_jump: Option<UnreadJump>,
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
