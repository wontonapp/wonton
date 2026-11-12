//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use wonton_matrix::{
    BackPaginationStatus, ChatRoom, EntryKind, MissedEvent, TimelineEntry, Vector, VectorDiff,
};
use wonton_protocol::{
    BackPagination, KeyNotice, TimelineDiff, TimelineScroll, TimelineUpdate, TimelineViewport,
};

use super::agent_run::BotsView;
use super::confirmation::{Cards, CardsView};
use super::links::LinkTargets;
use super::menu::{Menus, Outbox};
use super::read_mark::ReadMark;
use super::reply::QuoteFetches;
use super::view::{timeline_diff, timeline_item};

/// 一个房间的时间线订阅。壳循环调用 [`next`](Self::next)，把每一批增量按顺序应用到自己的列表上；不再需要时
/// 调用 [`cancel`](Self::cancel)。第一批是 `Reset`，给出当前的全部条目。
#[derive(uniffi::Object)]
pub struct TimelineSubscription {
    updates: Mutex<mpsc::UnboundedReceiver<TimelineUpdate>>,
    cancelled: watch::Sender<bool>,
    scroll: watch::Sender<TimelineScroll>,
}

impl TimelineSubscription {
    /// 在核心的运行时里开始转发 `room` 的时间线。订阅释放或取消后转发自动停止。
    ///
    /// `room_view` 是 tabs 合成的房间面板视图（ADR-0052）：每一批都带上当前值，变了也发一批空的。`chat` 读加密状态，
    /// 决定解不开的消息旁给什么入口。
    pub(crate) fn start(
        runtime: &Handle,
        room: Arc<ChatRoom>,
        cards: Cards,
        plugins: Arc<crate::plugins::PluginsLink>,
        menus: Arc<std::sync::Mutex<Menus>>,
        room_view: watch::Receiver<Option<Arc<crate::tabs::RoomPanelView>>>,
        chat: watch::Receiver<wonton_protocol::ChatViewModel>,
    ) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancelled, mut stop) = watch::channel(false);
        let (scroll, mut scrolled) = watch::channel(TimelineScroll::Bottom);
        runtime.spawn(async move {
            let outbox = Outbox::new(sender, menus, room_view.clone(), chat.clone());
            tokio::select! {
                () = forward(&room, cards, &plugins, &outbox, &mut scrolled, room_view, chat) => {}
                () = outbox.closed() => {}
                _ = stop.wait_for(|cancelled| *cancelled) => {}
            }
        });
        Arc::new(Self {
            updates: Mutex::new(receiver),
            cancelled,
            scroll,
        })
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
    }
}

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

    fn refresh_unread(&mut self, room: &ChatRoom) {
        self.unread = u32::try_from(room.unread_messages()).unwrap_or(u32::MAX);
    }
}

/// 先发当前条目，再逐批转发增量；翻页状态、滚动位置变了也发一批（增量为空）。确认卡片的显示还取决于这台设备回答的进展和
/// 现在的时间，它们变了时比较前后的显示，变了的卡片发 `Set`，所以这里留着一份 SDK 的条目。
/// `select!` 分支的结果：这一批送出去没有，以及这一批里有没有清空。
enum Batch {
    Sent(Result<(), ()>, bool),
}

/// 卡片显示变了就重发受影响的条目，没变就等下一轮。用在 [`forward`] 的选择分支里，展开成 `continue` 或一批。
macro_rules! refreshed {
    ($entries:ident, $shown:ident, $cards:ident, $read:ident, $plugins:ident, $sender:ident, $view:expr, $bots:expr) => {
        match refreshed_batch(
            &$entries,
            &mut $shown,
            &$cards,
            &$read,
            &links_of($plugins),
            $sender,
            $view,
            &mut $bots,
        ) {
            Some(batch) => batch,
            None => continue,
        }
    };
}

#[allow(clippy::too_many_lines)] // 一个循环里就是这些分支，拆出去只是把 select 用到的状态搬来搬去。
async fn forward(
    room: &Arc<ChatRoom>,
    mut cards: Cards,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    scrolled: &mut watch::Receiver<TimelineScroll>,
    mut room_view: watch::Receiver<Option<Arc<crate::tabs::RoomPanelView>>>,
    mut chat: watch::Receiver<wonton_protocol::ChatViewModel>,
) {
    let mut rebuilt_for: Option<wonton_matrix::OwnedEventId> = None;
    'outer: loop {
        // 换了时间线，SDK 里取过的引用也跟着没了，重新取。
        let fetches = QuoteFetches::default();
        let (entries, diffs) = room.entries().await;
        let mut entries: Vector<TimelineEntry> = entries.into_iter().collect();
        let mut shown = cards.view();
        let mut read = ReadMark::of(&entries);
        let mut statuses = std::pin::pin!(room.back_pagination_status().await);
        let status = match statuses.next().await {
            Some(status) => back_pagination(status),
            None => BackPagination::Idle,
        };
        let links = LinkTargets::of(&plugins.view());
        // Bot 列表、活动和等回答的卡片变了，AgentRun 条目的显示跟着变：每批都带上现在的情况，变了就重算。
        let mut bots = BotsView::of(&chat.borrow_and_update(), &entries, &shown);
        let Some(mut view) =
            start_batch(room, &entries, &shown, &read, &links, sender, status, &bots).await
        else {
            return;
        };
        fetches.request(room, &entries);
        let mut diffs = std::pin::pin!(diffs);
        // 插件装、卸以后，同一段文字里的插件网址类别会变，要重算。
        let mut plugin_changes = plugins.subscribe();
        // 未读数在同步之后才重新数，比带消息的那一批晚；跟着房间信息重算，按钮上的数字才不会停在旧数上。
        let mut room_info = room.subscribe_to_info_updates();
        // 丢事件的时候常常是“后面没有别的增量了”，光靠增量触发查不出来；每秒看一次缓存有没有漏就够。
        let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            view.refresh_unread(room);
            let next_expiry = shown.next_expiry(&entries);
            let sent = tokio::select! {
                batch = diffs.next() => {
                    let Some(batch) = batch else { return };
                    apply(room, &mut entries, &mut shown, &cards, &mut read, plugins, sender, &mut view, &mut bots, batch)
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
                        None => continue,
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
                    let Some(batch) = relinked_batch(&entries, &cards, &read, plugins, sender, &mut view, &bots) else {
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
                    view.refresh_unread(room);
                    Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                }
                changed = chat.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // 加密状态变了入口跟着变；Bot 列表、活动或等回答的卡片变了，AgentRun 条目跟着重算（ADR-0082）。
                    let new_bots = BotsView::of(&chat.borrow_and_update(), &entries, &shown);
                    let diffs = refreshed_agent_runs(
                        &entries,
                        &shown,
                        &read,
                        &links_of(plugins),
                        &bots,
                        &new_bots,
                    );
                    if diffs.is_empty() && sender.key_notice(&entries) == view.notice {
                        continue;
                    }
                    bots = new_bots;
                    Batch::Sent(send(sender, &entries, diffs, &mut view, None), false)
                }
                changed = cards.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    refreshed!(entries, shown, cards, read, plugins, sender, &mut view, bots)
                }
                () = tokio::time::sleep(next_expiry.unwrap_or_default()), if next_expiry.is_some() => {
                    refreshed!(entries, shown, cards, read, plugins, sender, &mut view, bots)
                }
            };
            match settle(sent, room, &entries, sender, &mut view).await {
                Next::End => return,
                Next::Rebuild => continue 'outer,
                Next::Continue => fetches.request(room, &entries),
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
    shown: &CardsView,
    read: &ReadMark,
    links: &LinkTargets,
    sender: &Outbox,
    status: BackPagination,
    bots: &BotsView,
) -> Option<View> {
    let mut view = View {
        status,
        scroll: TimelineScroll::Bottom,
        unread: 0,
        sent_unread: 0,
        notice: None,
    };
    view.refresh_unread(room);
    let position = read_index(room, entries).await;
    let anchor = anchor_of(entries, view.unread, position);
    if anchor.as_deref() != entries.last().map(|entry| entry.id.as_str()) {
        view.scroll = TimelineScroll::Away;
    }
    let reset = TimelineDiff::Reset {
        items: entries
            .iter()
            .map(|entry| timeline_item(entry.clone(), shown, read, links, bots))
            .collect(),
    };
    send(sender, entries, vec![reset], &mut view, anchor).ok()?;
    Some(view)
}

/// 插件表变了：整条时间线按现在的插件表重画一遍；没有条目时返回 `None`。
fn relinked_batch(
    entries: &Vector<TimelineEntry>,
    cards: &Cards,
    read: &ReadMark,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    view: &mut View,
    bots: &BotsView,
) -> Option<Batch> {
    let links = LinkTargets::of(&plugins.view());
    let shown = cards.view();
    let diffs = relinked(entries, &shown, read, &links, bots);
    if diffs.is_empty() {
        return None;
    }
    Some(Batch::Sent(send(sender, entries, diffs, view, None), false))
}

/// 整条时间线按给定的插件表重画：链接的类别可能变。
fn relinked(
    entries: &Vector<TimelineEntry>,
    shown: &CardsView,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
) -> Vec<TimelineDiff> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), shown, read, links, bots),
        })
        .collect()
}

/// 一批增量：清空批折成增删，其余逐条翻译后发出。
#[allow(clippy::too_many_arguments)] // 一批要用的输入就是这些，再包一层结构体只是把参数搬个家。
fn apply(
    room: &ChatRoom,
    entries: &mut Vector<TimelineEntry>,
    shown: &mut CardsView,
    cards: &Cards,
    read: &mut ReadMark,
    plugins: &crate::plugins::PluginsLink,
    sender: &Outbox,
    view: &mut View,
    bots: &mut BotsView,
    batch: Vec<VectorDiff<TimelineEntry>>,
) -> Batch {
    if batch.iter().any(|diff| matches!(diff, VectorDiff::Clear)) {
        return clear_batch(
            room,
            entries,
            batch,
            shown,
            read,
            &links_of(plugins),
            sender,
            view,
            bots,
        );
    }
    let links = LinkTargets::of(&plugins.view());
    let mut out = refresh(entries, shown, cards, read, &links, bots);
    for diff in batch {
        out.push(timeline_diff(diff.clone(), shown, read, &links, bots));
        diff.apply(entries);
    }
    // 这一批带来了新的卡片或回答：房间里的等回答卡片变了，AgentRun 条目跟着重算。
    let new_bots = bots.with_waiting_cards(entries, shown);
    out.extend(refreshed_agent_runs(
        entries, shown, read, &links, bots, &new_bots,
    ));
    *bots = new_bots;
    out.extend(move_read_mark(entries, shown, read, &links, bots));
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
    shown: &CardsView,
    read: &mut ReadMark,
    links: &LinkTargets,
    sender: &Outbox,
    view: &mut View,
    bots: &mut BotsView,
) -> Batch {
    let replaced = reset_diffs(entries, &batch, shown, read, links, bots);
    tracing::info!(room_id = %room.room_id(), reason = "sdk-clear", silent = replaced.is_some(), "timeline got a Clear");
    let Some(mut out) = replaced else {
        return Batch::Sent(
            send(sender, entries, vec![TimelineDiff::Clear], view, None),
            true,
        );
    };
    out.extend(move_read_mark(entries, shown, read, links, bots));
    let new_bots = bots.with_waiting_cards(entries, shown);
    out.extend(refreshed_agent_runs(
        entries, shown, read, links, bots, &new_bots,
    ));
    *bots = new_bots;
    Batch::Sent(send(sender, entries, out, view, None), false)
}

/// 把一次重置折成增删：返回 `Some` 时已经换上新的条目，`None` 表示这次重置动到了开头或丢了条目，要清空重建。
/// 广播频道跟不上时重发的就是同一批事件（只多不少、开头逐条相同），这样壳不会被清屏；同步从头来过时会从最新
/// 一段重来，开头对不上。
fn reset_diffs(
    entries: &mut Vector<TimelineEntry>,
    batch: &[VectorDiff<TimelineEntry>],
    shown: &CardsView,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
) -> Option<Vec<TimelineDiff>> {
    let mut replacement = entries.clone();
    for diff in batch {
        diff.clone().apply(&mut replacement);
    }
    let render = |items: &Vector<TimelineEntry>| {
        items
            .iter()
            .map(|entry| timeline_item(entry.clone(), shown, read, links, bots))
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

/// 卡片显示变了时要发的 `Set`；没变时返回 `None`，这一批不发。
#[allow(clippy::too_many_arguments)] // 同 [`apply`]，参数就是批处理要用的那几个。
fn refreshed_batch(
    entries: &Vector<TimelineEntry>,
    shown: &mut CardsView,
    cards: &Cards,
    read: &ReadMark,
    links: &LinkTargets,
    sender: &Outbox,
    view: &mut View,
    bots: &mut BotsView,
) -> Option<Batch> {
    let mut diffs = refresh(entries, shown, cards, read, links, bots);
    // 回答有了进展（卡片结束）：房间里的等回答卡片变了，AgentRun 条目跟着重算。
    let new_bots = bots.with_waiting_cards(entries, shown);
    diffs.extend(refreshed_agent_runs(
        entries, shown, read, links, bots, &new_bots,
    ));
    *bots = new_bots;
    if diffs.is_empty() {
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

/// 换成卡片现在的显示，返回显示变了的卡片。
fn refresh(
    entries: &Vector<TimelineEntry>,
    shown: &mut CardsView,
    cards: &Cards,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
) -> Vec<TimelineDiff> {
    let now = cards.view();
    let mut diffs = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if !now.is_card(entry) {
            continue;
        }
        let before = timeline_item(entry.clone(), shown, read, links, bots);
        let after = timeline_item(entry.clone(), &now, read, links, bots);
        if before != after {
            diffs.push(TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item: after,
            });
        }
    }
    *shown = now;
    diffs
}

/// “已读”换了地方时，换下来和换上去的两条各发一个 `Set`。
fn move_read_mark(
    entries: &Vector<TimelineEntry>,
    cards: &CardsView,
    read: &mut ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
) -> Vec<TimelineDiff> {
    let now = ReadMark::of(entries);
    if now == *read {
        return Vec::new();
    }
    let diffs = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| read.marks(&entry.id) || now.marks(&entry.id))
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), cards, &now, links, bots),
        })
        .collect();
    *read = now;
    diffs
}

/// Bot 列表、活动或等回答的卡片变了：AgentRun 条目按新的 Bot 情况重算，返回显示变了的。
fn refreshed_agent_runs(
    entries: &Vector<TimelineEntry>,
    shown: &CardsView,
    read: &ReadMark,
    links: &LinkTargets,
    before: &BotsView,
    after: &BotsView,
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
            let item = timeline_item(entry.clone(), shown, read, links, after);
            (timeline_item(entry.clone(), shown, read, links, before) != item)
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
mod tests {
    use super::*;
    use wonton_matrix::{Message, MessageBody, OwnedUserId, SendStatus};
    use wonton_protocol::{MessageContent, RichLinkKind, TimelineContent};

    fn text_entry(id: &str, body: &str) -> TimelineEntry {
        TimelineEntry {
            id: id.to_owned(),
            kind: wonton_matrix::EntryKind::Message(Box::new(Message {
                event_id: None,
                sender: "@bot:wonton.localhost".parse().unwrap(),
                sender_name: "Bot".to_owned(),
                is_own: false,
                timestamp_ms: 0,
                status: SendStatus::Sent,
                body: MessageBody::Text(body.to_owned()),
                custom: Default::default(),
                mentions: Vec::new(),
                agent_run: None,
                thread_root: None,
                edited: false,
                read_by: Vec::new(),
                reply: None,
                group: None,
                can_redact: false,
            })),
        }
    }

    fn kind(diff: &TimelineDiff) -> RichLinkKind {
        let TimelineDiff::Set { item, .. } = diff else {
            panic!("{diff:?}");
        };
        let TimelineContent::Message { message } = &item.content else {
            panic!("文字还是消息");
        };
        let MessageContent::Text { blocks, .. } = &message.body else {
            panic!("文字还是文字");
        };
        blocks
            .iter()
            .flat_map(|block| &block.spans)
            .find_map(|span| span.link.as_ref().map(|link| link.kind.clone()))
            .expect("链接还在")
    }

    /// 行为：先把插件网址渲染成外部网页，插件装好以后同一段文字重算成插件页。
    #[test]
    fn an_arriving_plugin_reclassifies_links() {
        let owner: OwnedUserId = "@owner:wonton.localhost".parse().unwrap();
        let (_, answers) = watch::channel(Default::default());
        let cards = Cards::new(&owner, answers).view();
        let read = ReadMark::default();
        let entries: Vector<TimelineEntry> = vec![text_entry(
            "1",
            "打开 http://app-grafana.home.example:18480/",
        )]
        .into_iter()
        .collect();

        let before = relinked(
            &entries,
            &cards,
            &read,
            &LinkTargets::default(),
            &BotsView::default(),
        );
        assert_eq!(kind(&before[0]), RichLinkKind::External);

        let links = super::super::links::test_targets(&[(
            "app.grafana",
            "http://app-grafana.home.example:18480/",
        )]);
        let after = relinked(&entries, &cards, &read, &links, &BotsView::default());
        assert_eq!(
            kind(&after[0]),
            RichLinkKind::Plugin {
                plugin_id: "app.grafana".to_owned()
            }
        );
    }
}
