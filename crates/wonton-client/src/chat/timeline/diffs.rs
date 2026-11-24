//! 时间线的批次与条目：发给壳的 [`View`] 外壳状态，SDK 增量折成增删的翻译，以及首批、插件重画、旁聊和送出。

use wonton_matrix::{ChatRoom, EntryKind, TimelineEntry, Vector, VectorDiff};
use wonton_protocol::{BackPagination, KeyNotice, TimelineDiff, TimelineScroll, TimelineViewport};

use super::super::agent_run::BotsView;
use super::super::links::LinkTargets;
use super::super::menu::Outbox;
use super::super::previews::Previews;
use super::super::read_mark::ReadMark;
use super::super::side_chat::{SideChatsView, Support};
use super::super::view::{timeline_diff, timeline_item};
use super::forward::Batch;
use super::rebuild::{anchor_of, move_read_mark, read_index};
use super::recompute::{links_of, room_bots};
#[cfg(feature = "test-hooks")]
use super::silent_clear::take_silent_clear;

/// 每一批随增量带出去的外壳状态：翻页状态、壳报告的滚动位置、未读数。位置只在壳报告到底时变成
/// [`TimelineScroll::Bottom`]，未读数从那一刻起不再算数（到底了就是 0）。
pub(crate) struct View {
    pub(super) status: BackPagination,
    pub(super) scroll: TimelineScroll,
    pub(super) unread: u32,
    /// 上一批送出去的未读数：同步之后未读数变了才补发一批，一样的数字不重复发。
    pub(super) sent_unread: u32,
    /// 上一批送出去的加密入口：加密状态变了时入口跟着变才补发一批，没变不重复发。
    pub(super) notice: Option<KeyNotice>,
    /// 房间里有没有人在通话（`m.call.member`），和上一批送出去的：变了补发一批（ADR-0096）。
    pub(super) call: bool,
    pub(super) sent_call: bool,
    /// 群里在场的 AI（本账号认识的 Bot 里在房间成员中的），和上一批送出去的：变了补发一批。
    /// 私聊里 Bot 本来就在顶栏，壳不消费这个字段。
    bots_in_room: Option<Vec<String>>,
    sent_bots_in_room: Option<Vec<String>>,
}

impl View {
    fn viewport(&self) -> TimelineViewport {
        match self.scroll {
            TimelineScroll::Bottom => TimelineViewport::Bottom,
            TimelineScroll::Away => TimelineViewport::Away {
                new_messages: self.unread,
            },
        }
    }

    pub(super) async fn refresh_unread(&mut self, room: &ChatRoom, bots: &BotsView) {
        self.unread = u32::try_from(room.unread_messages().await).unwrap_or(u32::MAX);
        self.call = room.call_active();
        self.bots_in_room = room_bots(room.are_members_synced(), bots, || room.member_user_ids());
    }

    /// 在场的 AI 变了没有（和上一批送出去的比）。
    pub(super) fn bots_changed(&self) -> bool {
        self.bots_in_room != self.sent_bots_in_room
    }
}

/// 首批：整条时间线发出去，并给出该停在哪。进房间停在已读位置，没有未读时到最新一条；壳把这个落点摆好之前
/// 不报到底。送回失败时返回 `None`。
#[allow(clippy::too_many_arguments)] // 一批要用的输入就是这些，再包一层结构体只是把参数搬个家。
pub(super) async fn start_batch(
    room: &ChatRoom,
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    sender: &Outbox,
    status: BackPagination,
    bots: &BotsView,
    previews: &Previews,
) -> Option<View> {
    let mut view = View {
        status,
        scroll: TimelineScroll::Bottom,
        unread: 0,
        sent_unread: 0,
        notice: None,
        call: false,
        sent_call: false,
        bots_in_room: None,
        sent_bots_in_room: None,
    };
    view.refresh_unread(room, bots).await;
    let position = read_index(room, entries).await;
    let anchor = anchor_of(entries, view.unread, position);
    if anchor.as_deref() != entries.last().map(|entry| entry.id.as_str()) {
        view.scroll = TimelineScroll::Away;
    }
    let reset = TimelineDiff::Reset {
        items: entries
            .iter()
            .map(|entry| timeline_item(entry.clone(), read, links, bots, previews))
            .collect(),
    };
    send(sender, entries, vec![reset], &mut view, anchor).ok()?;
    Some(view)
}

/// 插件表变了：整条时间线按现在的插件表重画一遍；没有条目时返回 `None`。
#[allow(clippy::too_many_arguments)] // 和 [`apply`] 一样，参数就是批处理要用的那几个。
pub(super) fn relinked_batch(
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    view: &mut View,
    bots: &BotsView,
    previews: &Previews,
) -> Option<Batch> {
    let links = LinkTargets::of(&plugins.view());
    let diffs = relinked(entries, read, &links, bots, previews);
    if diffs.is_empty() {
        return None;
    }
    Some(Batch::Sent(send(sender, entries, diffs, view, None), false))
}

/// 整条时间线按给定的插件表重画：链接的类别可能变。
pub(super) fn relinked(
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &Previews,
) -> Vec<TimelineDiff> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), read, links, bots, previews),
        })
        .collect()
}

/// 一批增量：清空批折成增删，其余逐条翻译后发出。
#[allow(clippy::too_many_arguments)] // 一批要用的输入就是这些，再包一层结构体只是把参数搬个家。
pub(super) fn apply(
    room: &ChatRoom,
    entries: &mut Vector<TimelineEntry>,
    read: &mut ReadMark,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    view: &mut View,
    bots: &BotsView,
    previews: &Previews,
    batch: Vec<VectorDiff<TimelineEntry>>,
) -> Batch {
    if batch.iter().any(|diff| matches!(diff, VectorDiff::Clear)) {
        // 测试钩子：这次清空不算 SDK 清空，只清自己的列表，不发 Clear 也不从缓存重建。留在时间线里
        // 缺着的缓存事件交给下方每秒的补检发现，走的就是活事件被丢掉的路径。
        #[cfg(feature = "test-hooks")]
        if take_silent_clear(room) {
            for diff in batch {
                diff.apply(entries);
            }
            return Batch::Sent(Ok(()), false);
        }
        return clear_batch(
            room,
            entries,
            batch,
            read,
            &links_of(plugins),
            sender,
            view,
            bots,
            previews,
        );
    }
    let links = LinkTargets::of(&plugins.view());
    let mut out = Vec::new();
    let mut arrived = Vec::new();
    for diff in batch {
        match &diff {
            VectorDiff::Insert { value, .. } | VectorDiff::Set { value, .. } => {
                if let EntryKind::Message(message) = &value.kind {
                    arrived.push(message.sender.to_string());
                }
            }
            _ => {}
        }
        out.push(timeline_diff(diff.clone(), read, &links, bots, previews));
        diff.apply(entries);
    }
    // 发消息的人不用再等 30 秒的正在输入超时。
    sender.live.typing.forget(&arrived);
    out.extend(move_read_mark(entries, read, &links, bots, previews));
    // 对账不放在每批之后：它会读一遍整个缓存，批多一点就把时间线的更新通道挤到跟不上（15 秒一次都嫌多）。
    Batch::Sent(send(sender, entries, out, view, None), false)
}

/// 这一批里有 Clear：SDK 重置时间线时会把重读的内容跟在同一批里发过来。把重读的结果折成最小的增删交给壳，
/// 不惊动它的清屏；开头的条目变了、或者一下子少了东西（同步从头来过）才返回 `None`，由调用方清空重建。
#[allow(clippy::too_many_arguments)] // 和 [`apply`] 一样，参数就是批处理要用的那几个。
pub(super) fn clear_batch(
    room: &ChatRoom,
    entries: &mut Vector<TimelineEntry>,
    batch: Vec<VectorDiff<TimelineEntry>>,
    read: &mut ReadMark,
    links: &LinkTargets,
    sender: &Outbox,
    view: &mut View,
    bots: &BotsView,
    previews: &Previews,
) -> Batch {
    let replaced = reset_diffs(entries, &batch, read, links, bots, previews);
    tracing::info!(room_id = %room.room_id(), reason = "sdk-clear", silent = replaced.is_some(), "timeline got a Clear");
    let Some(mut out) = replaced else {
        return Batch::Sent(
            send(sender, entries, vec![TimelineDiff::Clear], view, None),
            true,
        );
    };
    out.extend(move_read_mark(entries, read, links, bots, previews));
    Batch::Sent(send(sender, entries, out, view, None), false)
}

/// 把一次重置折成增删：返回 `Some` 时已经换上新的条目，`None` 表示这次重置动到了开头或丢了条目，要清空重建。
/// 广播频道跟不上时重发的就是同一批事件（只多不少、开头逐条相同），这样壳不会被清屏；同步从头来过时会从最新
/// 一段重来，开头对不上。
fn reset_diffs(
    entries: &mut Vector<TimelineEntry>,
    batch: &[VectorDiff<TimelineEntry>],
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &Previews,
) -> Option<Vec<TimelineDiff>> {
    let mut replacement = entries.clone();
    for diff in batch {
        diff.clone().apply(&mut replacement);
    }
    let render = |items: &Vector<TimelineEntry>| {
        items
            .iter()
            .map(|entry| timeline_item(entry.clone(), read, links, bots, previews))
            .collect::<Vec<_>>()
    };
    let (old, new) = (render(entries), render(&replacement));
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    if prefix == 0 || new.len() < old.len() {
        return None;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let mut out = Vec::new();
    for index in (prefix..old.len() - suffix).rev() {
        out.push(TimelineDiff::Remove {
            index: u32::try_from(index).unwrap_or(u32::MAX),
        });
    }
    for (offset, item) in new[prefix..new.len() - suffix].iter().enumerate() {
        out.push(TimelineDiff::Insert {
            index: u32::try_from(prefix + offset).unwrap_or(u32::MAX),
            item: item.clone(),
        });
    }
    *entries = replacement;
    Some(out)
}

/// 旁聊情况重算以后：卡片、结果行、根的显示变了的发 `Set`，头部变了也发一批；都没变时返回 `None`。
#[allow(clippy::too_many_arguments)] // 同 [`apply`]，参数就是批处理要用的那几个。
pub(super) async fn side_chat_batch(
    room: &ChatRoom,
    support: &Support,
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    view: &mut View,
    bots: &mut BotsView,
    previews: &Previews,
) -> Option<Batch> {
    let side_chats = SideChatsView::load(room, bots.supports(support, room)).await;
    if &side_chats == bots.side_chats() {
        return None;
    }
    let header_changed = sender.set_side_chat(side_chats.header());
    let next = bots.with_side_chats(side_chats);
    let links = links_of(plugins);
    let diffs: Vec<TimelineDiff> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            matches!(&entry.kind, EntryKind::Message(message) if SideChatsView::affects(message))
        })
        .filter_map(|(index, entry)| {
            let item = timeline_item(entry.clone(), read, &links, &next, previews);
            (timeline_item(entry.clone(), read, &links, bots, previews) != item).then_some(
                TimelineDiff::Set {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    item,
                },
            )
        })
        .collect();
    *bots = next;
    if diffs.is_empty() && !header_changed {
        return None;
    }
    Some(Batch::Sent(send(sender, entries, diffs, view, None), false))
}

pub(crate) fn send(
    sender: &Outbox,
    entries: &Vector<TimelineEntry>,
    diffs: Vec<TimelineDiff>,
    view: &mut View,
    anchor: Option<String>,
) -> Result<(), ()> {
    // 要跳的原消息可能刚随这一批进来。
    sender.live.jumps.progress(entries);
    view.sent_unread = view.unread;
    view.notice = sender.key_notice(entries);
    view.sent_call = view.call;
    view.sent_bots_in_room = view.bots_in_room.clone();
    sender.send(
        entries,
        diffs,
        view.status,
        view.viewport(),
        anchor,
        view.call,
        view.bots_in_room.clone(),
    )
}
