//! 一直跟着和核心账号的私聊，把等回答的卡片放进 ViewModel（ADR-0016）：主人在别的房间、别的页面时，壳靠它提示有卡片要回答。
//!
//! 打开的房间才有完整的时间线，所以这个私聊单独订阅（`MatrixSession::watch_room`），和主人打开着哪个房间无关。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use wonton_matrix::{MatrixSession, OwnedRoomId, TimelineEntry, Vector};
use wonton_protocol::{ChatViewModel, RoomMembership};

use super::confirmation::{Answers, Cards, RoomCards, core_account};
use super::{Active, CardSuccess};

/// 开始跟一个房间时往回看多少条事件。卡片 10 分钟就过期，同一个插件同时等确认的又有上限，这些够了。
const CARD_LOOKBACK: u16 = 50;

/// 跟着房间列表找出和核心账号的私聊，逐个跟着，把结果汇总进 ViewModel。主人退出过私聊时核心会换一个新的，旧的会从 `m.direct`
/// 去掉，所以列表里通常只有一个。卡片在本会话里见过（亲眼见过它没结束，或这台设备回答过它）变成成功时往
/// 调一次 `succeeded`：装好插件的卡片出现时，已装列表多半还是旧的。
pub(super) async fn follow_cards(
    view: watch::Sender<ChatViewModel>,
    session: Arc<MatrixSession>,
    answers: watch::Sender<Answers>,
    succeeded: CardSuccess,
) {
    let core = core_account(session.user_id());
    let mut rooms = view.subscribe();
    let (sender, mut updates) = mpsc::unbounded_channel();
    let mut followed: HashMap<String, Active> = HashMap::new();
    let mut standing: BTreeMap<String, RoomCards> = BTreeMap::new();
    loop {
        let rows = rooms.borrow_and_update().rooms.clone();
        // 是不是核心的私聊以会话里的 `m.direct` 为准，不以行上的私聊字段认：行跟着每次列表重算重取，
        // `m.direct` 还没同步到、或 Tuwunel 补状态的间隙里，同一房间在相邻两次重算里一会儿算一会儿不算，
        // 拿它当集合的依据，订阅就跟着行抖动进出（`MatrixSession::direct_chat_rooms_with`）。
        // 读不到就不动集合：不知道私聊名单时，进和出都没有依据，等下一轮重算再试。
        let dm_rooms: Option<BTreeSet<String>> = match session.direct_chat_rooms_with(&core).await {
            Ok(rooms) => Some(rooms.into_iter().map(|room| room.to_string()).collect()),
            Err(error) => {
                tracing::warn!("cannot read m.direct; keeping watched rooms unchanged: {error}");
                None
            }
        };
        // 进集合：行已是已加入，且 `m.direct` 把房间标给核心账号。
        let wanted: BTreeSet<String> = dm_rooms
            .as_ref()
            .map(|dm_rooms| {
                rows.iter()
                    .filter(|row| {
                        row.membership == RoomMembership::Joined && dm_rooms.contains(&row.room_id)
                    })
                    .map(|row| row.room_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        // 出集合只认稳定的证据，行上的其他变化不动已跟着的房间：
        // - `m.direct` 不再把房间标给核心：私聊真的换走了，这是收到事件才有的变化；
        // - 行从列表里消失：列表只显示没离开的房间，消失就是离开了。
        // 行上的成员状态不看出：Tuwunel 在加入后的第一次同步里先让事件可见、再补房间的当前状态，这中间
        // 的行把已加入的房间错标成受邀（见 `MatrixSession::until_joined`），等下一次重算就回正。按它退订，
        // 订阅跟着行翻转，每次都取消在飞的同步请求，进而把整个同步顶重启（#370）。
        let gone: Vec<String> = match &dm_rooms {
            Some(dm_rooms) => followed
                .keys()
                .filter(|room| {
                    !dm_rooms.contains(*room) || !rows.iter().any(|row| &row.room_id == *room)
                })
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        for room in gone {
            followed.remove(&room);
            standing.remove(&room);
            if let Ok(room) = room.parse::<OwnedRoomId>() {
                session.unwatch_room(&room).await;
            }
        }
        for room in wanted {
            if followed.contains_key(&room) {
                continue;
            }
            let task = tokio::spawn(follow_room(
                session.clone(),
                room.clone(),
                Cards::new(session.user_id(), answers.subscribe()),
                sender.clone(),
                succeeded.clone(),
            ));
            followed.insert(room, Active { tasks: vec![task] });
        }
        publish(&view, &standing);
        tokio::select! {
            changed = rooms.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            Some((room, cards)) = updates.recv() => {
                standing.insert(room, cards);
            }
        }
    }
}

/// 跟着一个房间，卡片的情况每次可能变化时报一次：来了新事件、这台设备的回答有了进展、有卡片到期。
/// 卡片在本会话里见过（亲眼见过它没结束，或这台设备回答过它）变成成功时调一次 `succeeded`。
async fn follow_room(
    session: Arc<MatrixSession>,
    room_id: String,
    mut cards: Cards,
    out: mpsc::UnboundedSender<(String, RoomCards)>,
    succeeded: CardSuccess,
) {
    let room = match room_id.parse::<OwnedRoomId>() {
        Ok(room) => session.watch_room(&room).await,
        Err(error) => {
            tracing::warn!(room = %room_id, "notification room ID isn't valid: {error}");
            return;
        }
    };
    let room = match room {
        Ok(room) => room,
        Err(error) => {
            tracing::warn!(room = %room_id, "can't follow the notification room, so cards won't be announced: {error}");
            return;
        }
    };
    let (entries, diffs) = room.entries().await;
    let mut entries: Vector<TimelineEntry> = entries.into_iter().collect();
    let mut diffs = std::pin::pin!(diffs);
    let mut newest = None;
    // 本会话里见过、还没结束的卡片：它变成成功时报一次插件 ID。
    let mut watched = HashSet::new();
    // 报过成功结果的卡片：重放和重复编辑不再报。
    let mut reported = HashSet::new();
    loop {
        // 同步只给最新的事件，更早的卡片要往回翻才看得到。缺口是同步带来的，所以最底下来了新事件就翻一次；翻回来的在上面，不会再触发。
        let latest = entries.back().map(|entry| entry.id.clone());
        if latest != newest {
            newest = latest;
            if let Err(error) = room.paginate_backwards(CARD_LOOKBACK).await {
                tracing::warn!(room = %room_id, "can't page back through the notification room, so older cards won't be announced: {error}");
            }
        }
        let shown = cards.view();
        for (card_id, done, plugin_id) in shown.plugin_cards(&entries) {
            if done {
                // 本会话里见过的卡片变成成功才报：亲眼见过它没结束，或这台设备回答过它。
                // 同一批同步里卡片和结果一起到时，亲眼这条路赶不上，回答过的照报。
                // 启动时翻回来的历史结果（两条都不占）不报——那时连上后的取列表本来就会跑。
                // 报过一次就不再报，重放和重复编辑不再触发。
                let witnessed = watched.remove(&card_id) || shown.answered_here(&card_id);
                if witnessed && reported.insert(card_id.clone()) {
                    // 测试钩子开着时决定等放行（见 test_hooks）：settled 在这一步之后才发出去。
                    #[cfg(not(feature = "test-hooks"))]
                    succeeded(&plugin_id);
                    #[cfg(feature = "test-hooks")]
                    succeeded(&plugin_id).await;
                }
            } else {
                watched.insert(card_id);
            }
        }
        if out
            .send((room_id.clone(), shown.standing(&room_id, &entries)))
            .is_err()
        {
            return;
        }
        let next_expiry = shown.next_expiry(&entries);
        tokio::select! {
            batch = diffs.next() => {
                let Some(batch) = batch else { return };
                for diff in batch {
                    diff.apply(&mut entries);
                }
            }
            changed = cards.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            () = tokio::time::sleep(next_expiry.unwrap_or_default()), if next_expiry.is_some() => {}
        }
    }
}

fn publish(view: &watch::Sender<ChatViewModel>, standing: &BTreeMap<String, RoomCards>) {
    let mut pending: Vec<_> = standing
        .values()
        .flat_map(|cards| cards.pending.iter().cloned())
        .collect();
    pending.sort_by_key(|(expires_at_ms, _)| *expires_at_ms);
    let pending: Vec<_> = pending.into_iter().map(|(_, card)| card).collect();
    let settled: Vec<_> = standing
        .values()
        .flat_map(|cards| cards.settled.iter().cloned())
        .collect();
    view.send_if_modified(|view| {
        if view.pending_confirmations == pending && view.settled_confirmations == settled {
            return false;
        }
        view.pending_confirmations.clone_from(&pending);
        view.settled_confirmations.clone_from(&settled);
        true
    });
}
