//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use wonton_matrix::{
    BackPaginationStatus, ChatRoom, EntryKind, MissedEvent, OwnedRoomId, TimelineEntry, Vector,
    VectorDiff,
};
use wonton_protocol::{
    BackPagination, ConfirmationView, KeyNotice, TimelineDiff, TimelineScroll, TimelineUpdate,
    TimelineViewport,
};

use super::agent_run::BotsView;
use super::links::LinkTargets;
use super::menu::{Menus, Outbox};
use super::previews::Previews;
use super::read_mark::ReadMark;
use super::reply::QuoteFetches;
use super::side_chat::{SideChatsView, Support};
use super::timeline_previews::{ensure_previews, preview_diffs};
use super::view::{timeline_diff, timeline_item};

/// 一个房间的时间线订阅。壳循环调用 [`next`](Self::next)，把每一批增量按顺序应用到自己的列表上；不再需要时
/// 调用 [`cancel`](Self::cancel)。第一批是 `Reset`，给出当前的全部条目。
#[derive(uniffi::Object)]
pub struct TimelineSubscription {
    updates: Mutex<mpsc::UnboundedReceiver<TimelineUpdate>>,
    cancelled: watch::Sender<bool>,
    scroll: watch::Sender<TimelineScroll>,
    opening: std::sync::Mutex<Option<super::target::TimelineOpening>>,
}

/// 一个房间的时间线订阅要用的部件：房间、能显示的卡片和各路账号数据。[`TimelineSubscription::start`] 一次带走。
pub struct TimelineSource {
    pub room: Arc<ChatRoom>,
    /// 打开的房间：卡片行按 `origin.room_id` 等于它过滤（ADR-0016）。
    pub room_id: OwnedRoomId,
    /// 这个房间能显示的确认卡片。
    pub confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    pub plugins: Arc<crate::plugins::PluginsLink>,
    pub previews: Previews,
    pub menus: Arc<std::sync::Mutex<Menus>>,
    pub room_view: watch::Receiver<Option<Arc<crate::pages::RoomPanelView>>>,
    pub chat: watch::Receiver<wonton_protocol::ChatViewModel>,
    /// 各房间的旁聊支持声明（ADR-0093 §1）。
    pub support: Arc<Support>,
}

impl TimelineSubscription {
    /// 在核心的运行时里开始转发 `room` 的时间线。订阅释放或取消后转发自动停止。
    ///
    /// `room_view` 是 tabs 合成的房间面板视图（ADR-0052）：每一批都带上当前值，变了也发一批空的。`chat` 读加密状态，
    /// 决定解不开的消息旁给什么入口。`confirmations` 是这个房间能显示的确认卡片（ADR-0016），出口按
    /// `origin.room_id` 过滤后随每一批带上，变了也发一批空的。
    pub(crate) fn start(
        runtime: &Handle,
        source: TimelineSource,
        opening: super::target::TimelineOpening,
    ) -> Arc<Self> {
        let TimelineSource {
            room,
            room_id,
            confirmations,
            plugins,
            previews,
            menus,
            room_view,
            chat,
            support,
        } = source;
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancelled, mut stop) = watch::channel(false);
        // 落点摆好并由壳确认之前，不能把首屏量出来的默认位置当成已经读到底。
        let (scroll, mut scrolled) = watch::channel(TimelineScroll::Away);
        runtime.spawn(async move {
            let outbox = Outbox::new(
                sender,
                menus,
                room_view.clone(),
                chat.clone(),
                room_id,
                // 只读快照用的克隆。等变化必须用下面交给 `forward` 的那个接收者：克隆带着克隆那一刻的
                // 已读位置，每轮新克隆一个去等 `changed()` 会立刻返回，订阅就空转着一批接一批往外发。
                confirmations.clone(),
            );
            tokio::select! {
                () = forward(
                    &room,
                    &plugins,
                    previews,
                    &outbox,
                    &mut scrolled,
                    room_view,
                    chat,
                    confirmations,
                    &support,
                ) => {}
                () = outbox.closed() => {}
                _ = stop.wait_for(|cancelled| *cancelled) => {}
            }
        });
        Arc::new(Self {
            updates: Mutex::new(receiver),
            cancelled,
            scroll,
            opening: std::sync::Mutex::new(Some(opening)),
        })
    }

    pub(super) fn is_at_bottom(&self) -> bool {
        *self.scroll.borrow() == TimelineScroll::Bottom && !*self.cancelled.borrow()
    }
}

#[uniffi::export]
impl TimelineSubscription {
    /// 等待下一批增量。和 ViewModel 的订阅不同，这里的批次不会合并，壳要逐批应用。订阅取消、房间关闭或核心
    /// 释放后返回 `None`。
    pub async fn next(&self) -> Option<TimelineUpdate> {
        let mut cancelled = self.cancelled.subscribe();
        let mut updates = self.updates.lock().await;
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => None,
            update = updates.recv() => update,
        }
    }

    /// 壳报告列表停在哪（[`TimelineAction::SetTimelineScroll`](wonton_protocol::TimelineAction::SetTimelineScroll)）。
    /// 跨过底部时发一次，下一批更新里带重新算过的位置和新消息数。
    pub fn set_scroll(&self, scroll: TimelineScroll) {
        self.scroll.send_replace(scroll);
    }

    /// 取消订阅，语义同 `HomeSubscription::cancel`（Swift 的 Task 取消不会传到 Rust）。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
        drop(super::lock(&self.opening).take());
    }
}

#[cfg(test)]
#[path = "room_lifecycle_tests.rs"]
mod room_lifecycle_tests;

/// 每一批随增量带出去的外壳状态：翻页状态、壳报告的滚动位置、未读数。位置只在壳报告到底时变成
/// [`TimelineScroll::Bottom`]，未读数从那一刻起不再算数（到底了就是 0）。
struct View {
    status: BackPagination,
    scroll: TimelineScroll,
    unread: u32,
    /// 上一批送出去的未读数：同步之后未读数变了才补发一批，一样的数字不重复发。
    sent_unread: u32,
    /// 上一批送出去的加密入口：加密状态变了时入口跟着变才补发一批，没变不重复发。
    notice: Option<KeyNotice>,
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

    async fn refresh_unread(&mut self, room: &ChatRoom) {
        self.unread = u32::try_from(room.unread_messages().await).unwrap_or(u32::MAX);
    }
}

/// 先发当前条目，再逐批转发增量；翻页状态、滚动位置变了也发一批（增量为空）。
/// `select!` 分支的结果：这一批送出去没有，以及这一批里有没有清空。
enum Batch {
    Sent(Result<(), ()>, bool),
}

#[allow(clippy::too_many_lines)] // 一个循环里就是这些分支，拆出去只是把 select 用到的状态搬来搬去。
#[allow(clippy::too_many_arguments)] // 要等的变化就是这几路，再包一层结构体只是把参数搬个家。
async fn forward(
    room: &Arc<ChatRoom>,
    plugins: &crate::plugins::PluginsLink,
    previews: Previews,
    sender: &Outbox,
    scrolled: &mut watch::Receiver<TimelineScroll>,
    mut room_view: watch::Receiver<Option<Arc<crate::pages::RoomPanelView>>>,
    mut chat: watch::Receiver<wonton_protocol::ChatViewModel>,
    mut confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    support: &Support,
) {
    let mut rebuilt_for: Option<wonton_matrix::OwnedEventId> = None;
    'outer: loop {
        // 换了时间线，SDK 里取过的引用也跟着没了，重新取。
        let fetches = QuoteFetches::default();
        let (entries, diffs) = room.entries().await;
        let mut entries: Vector<TimelineEntry> = entries.into_iter().collect();
        let mut read = ReadMark::of(&entries);
        let mut statuses = std::pin::pin!(room.back_pagination_status().await);
        let status = match statuses.next().await {
            Some(status) => back_pagination(status),
            None => BackPagination::Idle,
        };
        let links = LinkTargets::of(&plugins.view());
        // Bot 列表、活动和等回答的卡片变了，AgentRun 条目的显示跟着变：每批都带上现在的情况，变了就重算。
        let bots = BotsView::of(&chat.borrow_and_update(), &sender.confirmations());
        let side_chats = SideChatsView::load(room, bots.supports(support, room)).await;
        sender.set_side_chat(side_chats.header());
        let mut bots = bots.with_side_chats(side_chats);
        let Some(mut view) = start_batch(
            room, &entries, &read, &links, sender, status, &bots, &previews,
        )
        .await
        else {
            return;
        };
        fetches.request(room, &entries);
        ensure_previews(&previews, &entries, plugins);
        // 预览落定了就醒一次：把落定网址所在的条目重发。
        let mut preview_versions = previews.subscribe();
        let mut diffs = std::pin::pin!(diffs);
        // 插件装、卸以后，同一段文字里的插件网址类别会变，要重算。
        let mut plugin_changes = plugins.subscribe();
        // 未读数在同步之后才重新数，比带消息的那一批晚；跟着房间信息重算，按钮上的数字才不会停在旧数上。
        let mut room_info = room.subscribe_to_info_updates();
        // 丢事件的时候常常是“后面没有别的增量了”，光靠增量触发查不出来；每秒看一次缓存有没有漏就够。
        let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            view.refresh_unread(room).await;
            let sent = tokio::select! {
                batch = diffs.next() => {
                    let Some(batch) = batch else { return };
                    apply(room, &mut entries, &mut read, plugins, sender, &mut view, &bots, &previews, batch)
                }
                update = room_info.recv() => {
                    match update {
                        Ok(update) if update.room_id.as_str() == room.room_id().as_str() => {}
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                    if view.scroll == TimelineScroll::Bottom || view.unread == view.sent_unread {
                        continue;
                    }
                    Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                }
                _ = check.tick() => {
                    match room.missed_cached_event(rebuilt_for.as_deref()).await {
                        Some(missed) => {
                            log_rebuild(room, "missed-cached-event", Some(&missed));
                            rebuilt_for = Some(missed.id);
                            Batch::Sent(Ok(()), true)
                        }
                        // 旁聊的未读、状态（“没收到结果”按时间到）、头部也按秒重算。
                        None => match side_chat_batch(room, support, &entries, &read, plugins, sender, &mut view, &mut bots, &previews).await {
                            Some(batch) => batch,
                            None => continue,
                        },
                    }
                }
                changed = room_view.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // 面板、红点或入口状态变了：增量是空的，壳只更新那几个字段。
                    Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                }
                changed = plugin_changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let Some(batch) =
                        relinked_batch(&entries, &read, plugins, sender, &mut view, &bots, &previews)
                    else {
                        continue;
                    };
                    batch
                }
                next = statuses.next() => {
                    let Some(next) = next else { return };
                    let next = back_pagination(next);
                    if next == view.status {
                        continue;
                    }
                    view.status = next;
                    Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                }
                changed = scrolled.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let next = *scrolled.borrow_and_update();
                    if next == view.scroll {
                        continue;
                    }
                    view.scroll = next;
                    view.refresh_unread(room).await;
                    Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                }
                changed = chat.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // 加密状态变了入口跟着变；Bot 列表或活动变了，AgentRun 条目跟着重算（ADR-0082）。
                    let new_bots = BotsView::of(&chat.borrow_and_update(), &sender.confirmations())
                        .with_side_chats(bots.side_chats().clone());
                    let diffs = refreshed_agent_runs(
                        &entries,
                        &read,
                        &links_of(plugins),
                        &bots,
                        &new_bots,
                        &previews,
                    );
                    if diffs.is_empty() && sender.key_notice(&entries) == view.notice {
                        continue;
                    }
                    bots = new_bots;
                    Batch::Sent(send(sender, &entries, diffs, &mut view, None), false)
                }
                changed = confirmations.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // 确认卡片变了（新卡片、回答的进展、到期）：每一批都带现在的列表，壳换输入栏上方那几行；
                    // 等回答的卡片变了，AgentRun 条目跟着重算（ADR-0082）。
                    let new_bots = bots.with_waiting_cards(&sender.confirmations());
                    let diffs = refreshed_agent_runs(
                        &entries,
                        &read,
                        &links_of(plugins),
                        &bots,
                        &new_bots,
                        &previews,
                    );
                    bots = new_bots;
                    Batch::Sent(send(sender, &entries, diffs, &mut view, None), false)
                }
                changed = preview_versions.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // 预览取回来了：落定网址所在的条目重发一遍，别的条目不动。
                    let settled = previews.take_settled();
                    let links = links_of(plugins);
                    let diffs = preview_diffs(&entries, &read, &links, &bots, &previews, &settled);
                    if diffs.is_empty() {
                        continue;
                    }
                    Batch::Sent(send(sender, &entries, diffs, &mut view, None), false)
                }
            };
            match settle(sent, room, &entries, sender, &mut view).await {
                Next::End => return,
                Next::Rebuild => continue 'outer,
                Next::Continue => {
                    fetches.request(room, &entries);
                    ensure_previews(&previews, &entries, plugins);
                }
            }
        }
    }
}

/// 首批：整条时间线发出去，并给出该停在哪。进房间停在已读位置，没有未读时到最新一条；壳把这个落点摆好之前
/// 不报到底。送回失败时返回 `None`。
#[allow(clippy::too_many_arguments)] // 一批要用的输入就是这些，再包一层结构体只是把参数搬个家。
async fn start_batch(
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
    };
    view.refresh_unread(room).await;
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
fn relinked_batch(
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
fn relinked(
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
fn apply(
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
    for diff in batch {
        out.push(timeline_diff(diff.clone(), read, &links, bots, previews));
        diff.apply(entries);
    }
    out.extend(move_read_mark(entries, read, &links, bots, previews));
    // 对账不放在每批之后：它会读一遍整个缓存，批多一点就把时间线的更新通道挤到跟不上（15 秒一次都嫌多）。
    Batch::Sent(send(sender, entries, out, view, None), false)
}

/// 一批送出后的下一步。
enum Next {
    Continue,
    End,
    /// 事件缓存被清空重来：旧的一条接不到清空期间的事件，要从头来。
    Rebuild,
}

async fn settle(
    sent: Batch,
    room: &ChatRoom,
    entries: &Vector<TimelineEntry>,
    sender: &Outbox,
    view: &mut View,
) -> Next {
    match sent {
        Batch::Sent(Err(_), _) => Next::End,
        Batch::Sent(Ok(()), true) => {
            // 告诉壳从头来，换一条时间线，从缓存重新读。房间对象不换，壳因为这个清空往回翻页时还落在同一个房间上。
            if send(sender, entries, vec![TimelineDiff::Clear], view, None).is_err() {
                return Next::End;
            }
            if room.rebuild_timeline().await.is_err() {
                return Next::End;
            }
            Next::Rebuild
        }
        Batch::Sent(Ok(()), false) => Next::Continue,
    }
}

/// 这一批里有 Clear：SDK 重置时间线时会把重读的内容跟在同一批里发过来。把重读的结果折成最小的增删交给壳，
/// 不惊动它的清屏；开头的条目变了、或者一下子少了东西（同步从头来过）才返回 `None`，由调用方清空重建。
#[allow(clippy::too_many_arguments)] // 和 [`apply`] 一样，参数就是批处理要用的那几个。
fn clear_batch(
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
async fn side_chat_batch(
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

/// 记下这次重建时间线的原因，失败时能从日志看出是哪条路。
fn log_rebuild(room: &ChatRoom, reason: &'static str, missed: Option<&MissedEvent>) {
    match missed {
        Some(missed) => tracing::info!(
            room_id = %room.room_id(),
            reason,
            event_id = %missed.id,
            absent = ?missed.absent,
            "rebuilding the timeline"
        ),
        None => tracing::info!(room_id = %room.room_id(), reason, "rebuilding the timeline"),
    }
}

/// “已读”“已送达”换了地方、上方该不该写时间变了的条目，各发一个 `Set`。
fn move_read_mark(
    entries: &Vector<TimelineEntry>,
    read: &mut ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &Previews,
) -> Vec<TimelineDiff> {
    let now = ReadMark::of(entries);
    if now == *read {
        return Vec::new();
    }
    let diffs = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| read.of_entry(&entry.id) != now.of_entry(&entry.id))
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), &now, links, bots, previews),
        })
        .collect();
    *read = now;
    diffs
}

/// Bot 列表、活动或等回答的卡片变了：AgentRun 条目按新的 Bot 情况重算，返回显示变了的。
fn refreshed_agent_runs(
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    before: &BotsView,
    after: &BotsView,
    previews: &Previews,
) -> Vec<TimelineDiff> {
    if before == after {
        return Vec::new();
    }
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            matches!(&entry.kind, EntryKind::Message(message) if message.agent_run.is_some())
        })
        .filter_map(|(index, entry)| {
            let item = timeline_item(entry.clone(), read, links, after, previews);
            (timeline_item(entry.clone(), read, links, before, previews) != item)
                .then_some(TimelineDiff::Set {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    item,
                })
        })
        .collect()
}

/// 认链接要用的插件表：刷新时现取，装了或卸了插件不用重开房间。
fn links_of(plugins: &crate::plugins::PluginsLink) -> LinkTargets {
    LinkTargets::of(&plugins.view())
}

fn send(
    sender: &Outbox,
    entries: &Vector<TimelineEntry>,
    diffs: Vec<TimelineDiff>,
    view: &mut View,
    anchor: Option<String>,
) -> Result<(), ()> {
    view.sent_unread = view.unread;
    view.notice = sender.key_notice(entries);
    sender.send(entries, diffs, view.status, view.viewport(), anchor)
}

/// 本账号已读位置在 `entries` 里的下标：回执指着的事件不在这一批里时为空。
async fn read_index(room: &ChatRoom, entries: &Vector<TimelineEntry>) -> Option<usize> {
    let position = wonton_matrix::entry_id(&room.read_position().await?);
    entries.iter().position(|entry| entry.id == position)
}

/// 首批该停在哪：有已读标记就停它自己；没有时按未读算——位置在这一批里就停它后面第一条，位置不在这里就从末尾
/// 往回数未读条数、数不够一屏就停在最早一条，让主人接着往前翻；没有未读停在最新一条，空房间为空。
fn anchor_of(entries: &Vector<TimelineEntry>, unread: u32, read: Option<usize>) -> Option<String> {
    let last = entries.len().checked_sub(1)?;
    if let Some(marker) = entries
        .iter()
        .find(|entry| matches!(entry.kind, EntryKind::ReadMarker))
    {
        return Some(marker.id.clone());
    }
    if unread == 0 {
        return Some(entries[last].id.clone());
    }
    if let Some(after) = read.and_then(|index| index.checked_add(1)) {
        return Some(entries[after.min(last)].id.clone());
    }
    let wanted = usize::try_from(unread).unwrap_or(usize::MAX);
    let mut counted = 0;
    for entry in entries.iter().rev() {
        if matches!(entry.kind, EntryKind::Message(_)) {
            counted += 1;
            if counted == wanted {
                return Some(entry.id.clone());
            }
        }
    }
    Some(entries[0].id.clone())
}

fn back_pagination(status: BackPaginationStatus) -> BackPagination {
    match status {
        BackPaginationStatus::Idle => BackPagination::Idle,
        BackPaginationStatus::Paginating => BackPagination::Paginating,
        BackPaginationStatus::ReachedStart => BackPagination::ReachedStart,
    }
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
