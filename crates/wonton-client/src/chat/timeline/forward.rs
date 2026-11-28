//! 订阅的转发循环：逐路等来 SDK 增量、房间信息、面板、插件表、加密状态、翻页、位置、确认卡片、预览、
//! 输入状态和跳转，折成批次发出去；首批和清空重建也在这里收口。

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;

use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use wonton_matrix::{ChatRoom, TimelineEntry, Vector};
use wonton_protocol::{BackPagination, ConfirmationView, TimelineDiff, TimelineScroll};

use super::super::agent_run::BotsView;
use super::super::bots;
use super::super::jump::Live;
use super::super::links::LinkTargets;
use super::super::menu::Outbox;
use super::super::previews::Previews;
use super::super::read_mark::ReadMark;
use super::super::reply::QuoteFetches;
use super::super::side_chat::{SideChatsView, Support};
use super::super::timeline_previews::{ensure_previews, preview_diffs};
use super::diffs::{View, apply, relinked_batch, send, side_chat_batch, start_batch};
use super::push_body;
use super::rebuild::{back_pagination, log_rebuild};
use super::recompute::{links_of, refreshed_agent_runs};

/// 先发当前条目，再逐批转发增量；翻页状态、滚动位置变了也发一批（增量为空）。
/// `select!` 分支的结果：这一批送出去没有，以及这一批里有没有清空。
pub(super) enum Batch {
    Sent(Result<(), ()>, bool),
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

#[allow(clippy::too_many_lines)] // 一个循环里就是这些分支，拆出去只是把 select 用到的状态搬来搬去。
#[allow(clippy::too_many_arguments)] // 要等的变化就是这几路，再包一层结构体只是把参数搬个家。
pub(super) async fn forward(
    room: &Arc<ChatRoom>,
    plugins: &crate::plugins::PluginsLink,
    previews: Previews,
    sender: &Outbox,
    scrolled: &mut watch::Receiver<TimelineScroll>,
    mut room_view: watch::Receiver<Option<Arc<crate::pages::RoomPanelView>>>,
    mut chat: watch::Receiver<wonton_protocol::ChatViewModel>,
    mut confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    support: &Support,
    ledger: &bots::Bots,
    live: &Live,
    session: &Arc<wonton_matrix::MatrixSession>,
    push_previews: &crate::push::PushPreviews,
    thread_root: Option<&wonton_matrix::OwnedEventId>,
) {
    let runtime = Handle::current();
    let mut rebuilt_for: Option<wonton_matrix::OwnedEventId> = None;
    'outer: loop {
        // 换了时间线，SDK 里取过的引用也跟着没了，重新取。
        let fetches = QuoteFetches::default();
        let (entries, diffs) = room.entries().await;
        let mut entries: Vector<TimelineEntry> = entries.into_iter().collect();
        let mut previewed = HashMap::new();
        // 先订阅再补预览：补完之后才到的推送靠 watch 的版本变化叫醒下面那一支；反过来的话，
        // 补和订阅之间到的推送会漏掉（订阅只从当前版本往后算），这一条预览就一直补不进来。
        let mut push_preview_changes = push_previews.subscribe();
        push_body::merge(
            &mut entries,
            push_previews,
            room.room_id(),
            thread_root.map(|root| root.as_str()),
            session.user_id(),
            &mut previewed,
        );
        let mut read = ReadMark::of(&entries);
        let mut statuses = std::pin::pin!(room.back_pagination_status().await);
        let status = match statuses.next().await {
            Some(status) => back_pagination(status),
            None => BackPagination::Idle,
        };
        let links = LinkTargets::of(&plugins.view());
        // Bot 列表、活动和等回答的卡片变了，AgentRun 条目的显示跟着变：每批都带上现在的情况，变了就重算。
        // 就绪先单独取：下面的 borrow_and_update 读锁活到语句结束，锁内再锁账本会和 reconcile 的
        // 「先账本、后写 chat」反序，Bot 在打字时就会死锁。
        let ready = ledger.ready();
        let bots = BotsView::of(&chat.borrow_and_update(), &sender.confirmations(), ready);
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
        // 正在输入、命令表和跳转请求（ADR-0109）。
        let mut typing_changes = live.typing.subscribe();
        let mut table_changes = live.tables.subscribe();
        let mut jump_changes = live.jumps.pending();
        // 重建期间挂上的跳转请求收不到那次唤醒，订阅之后先补一步。
        let mut edit_changes = live.edit.subscribe();
        live.jumps.tick(&runtime, room, &entries);
        live.tables.refresh(&runtime, session);
        // 插件装、卸以后，同一段文字里的插件网址类别会变，要重算。
        let mut plugin_changes = plugins.subscribe();
        // 未读数在同步之后才重新数，比带消息的那一批晚；跟着房间信息重算，按钮上的数字才不会停在旧数上。
        let mut room_info = room.subscribe_to_info_updates();
        // 丢事件的时候常常是“后面没有别的增量了”，光靠增量触发查不出来；每秒看一次缓存有没有漏就够。
        let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            view.refresh_unread(room, &entries, &bots).await;
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
                        view.refresh_unread(room, &entries, &bots).await;
                        let unread_moved =
                            view.scroll != TimelineScroll::Bottom && view.unread != view.sent_unread;
                        if !unread_moved && view.call == view.sent_call && !view.bots_changed() {
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
                            None => {
                                // 命令表不在同步的必需状态里，收到新表不会叫醒这里：按秒对一次表（内部限了间隔）。
                                live.tables.refresh(&runtime, session);
                                // 成员进出不产生时间线条目，在场的 AI 按秒对一次表：变了补发一批空的。
                                if view.bots_changed() {
                                    Batch::Sent(
                                        send(sender, &entries, Vec::new(), &mut view, None),
                                        false,
                                    )
                                } else {
                                    match side_chat_batch(room, support, &entries, &read, plugins, sender, &mut view, &mut bots, &previews).await {
                                        Some(batch) => batch,
                                        None => continue,
                                    }
                                }
                            },
                        }
                    }
                    changed = room_view.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // 面板、红点或 widget 入口变了：增量是空的，壳只更新那几个字段。
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                    changed = plugin_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // 插件表变了：链接的类别跟着变；「清单已就绪」也在这里翻面。装了插件但一个 Bot
                        // 都没有时 Bot 列表不变、chat 不醒，就靠这一路把就绪带进名单，变了补发一批空的。
                        // 就绪先单独取（原因见首批那段）：不在 chat 的读锁里锁账本。
                        let ready = ledger.ready();
                        let new_bots =
                            BotsView::of(&chat.borrow_and_update(), &sender.confirmations(), ready)
                                .with_side_chats(bots.side_chats().clone());
                        view.refresh_unread(room, &entries, &new_bots).await;
                        let relinked =
                            relinked_batch(&entries, &read, plugins, sender, &mut view, &new_bots, &previews);
                        bots = new_bots;
                        if let Some(batch) = relinked {
                            batch
                        } else if view.bots_changed() {
                            Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                        } else {
                            continue;
                        }
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
                        view.refresh_unread(room, &entries, &bots).await;
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                    changed = chat.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // 加密状态变了入口跟着变；Bot 列表或活动变了，AgentRun 条目跟着重算（ADR-0082）。
                        // 就绪先单独取（原因见首批那段）：不在 chat 的读锁里锁账本。
                        let ready = ledger.ready();
                        let new_bots =
                            BotsView::of(&chat.borrow_and_update(), &sender.confirmations(), ready)
                                .with_side_chats(bots.side_chats().clone());
                        let diffs = refreshed_agent_runs(
                            &entries,
                            &read,
                            &links_of(plugins),
                            &bots,
                            &new_bots,
                            &previews,
                        );
                        view.refresh_unread(room, &entries, &new_bots).await;
                        // 名单先存下来再判断跳不跳：改动已经被 borrow_and_update 消费掉，不存就丢了，
                        // 之后的批次会拿旧名单算（diff 为空和 bots_changed 不成立时尤其会走到 continue）。
                        bots = new_bots;
                        if diffs.is_empty()
                            && sender.key_notice(&entries) == view.notice
                            && !view.bots_changed()
                        {
                            continue;
                        }
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
                    changed = push_preview_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // 打开着的房间来了新推送：预览直接补在末尾，壳点开就看到。
                        let applied =
                            push_body::merge(
                &mut entries,
                push_previews,
                room.room_id(),
                thread_root.map(|root| root.as_str()),
                session.user_id(),
                &mut previewed,
            );
                        if applied.is_empty() {
                            continue;
                        }
                        let items = applied
                            .iter()
                            .map(|entry| super::super::view::timeline_item(entry.clone(), &read, &links, &bots, &previews))
                            .collect();
                        Batch::Sent(send(sender, &entries, vec![TimelineDiff::Append { items }], &mut view, None), false)
                    }
                    changed = typing_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // 一样的名单不重复发。
                        if !live.typing.take_changed() {
                            continue;
                        }
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                    changed = table_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        if !live.tables.take_changed() {
                            continue;
                        }
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                    changed = edit_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        if !live.edit.take_changed() {
                            continue;
                        }
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                    changed = jump_changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        live.jumps.tick(&runtime, room, &entries);
                        Batch::Sent(send(sender, &entries, Vec::new(), &mut view, None), false)
                    }
                };
            match settle(sent, room, &entries, sender, &mut view).await {
                Next::End => return,
                Next::Rebuild => continue 'outer,
                Next::Continue => {
                    fetches.request(room, &entries);
                    ensure_previews(&previews, &entries, plugins);
                    // 服务器的同一条消息到了：预览换成真正的事件，条目也拿掉。
                    let removed = push_body::settle(
                        &mut entries,
                        push_previews,
                        room.room_id(),
                        thread_root.map(|root| root.as_str()),
                        &mut previewed,
                    );
                    if !removed.is_empty() {
                        let diffs = removed
                            .into_iter()
                            .map(|index| TimelineDiff::Remove { index })
                            .collect();
                        if send(sender, &entries, diffs, &mut view, None).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}
