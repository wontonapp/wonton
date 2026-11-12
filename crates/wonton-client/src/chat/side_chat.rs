//! 旁聊（ADR-0093）：开旁聊、房间的支持声明、卡片状态的合成、时间线里旁聊条目的显示。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::{
    ChatRoom, MatrixSession, Message, OwnedEventId, OwnedRoomId, OwnedUserId, ResultLine,
    SIDE_CHAT_SUPPORT_EVENT, SideChatFacts, ThreadActivity, result_line, root_marker,
    title_from_body,
};
use wonton_protocol::{
    CoreError, SideChatCard, SideChatResultState, SideChatState, TimelineContent,
};

use super::view::core_error;
use super::{ChatLink, TimelineKey, invalid, lock, onboarding};
use crate::l10n::{CATALOG, language};

/// Bot 在旁聊里最后一条消息之后等结果行等多久，过了就是“没收到结果”（ADR-0093 §4，按实测再调）。
pub(super) const RESULT_WAIT: Duration = Duration::from_secs(120);

/// 合成卡片的状态（ADR-0093 §4 的表）。`now_ms` 是本机时间：和服务器时间戳差几秒只影响“没收到结果”早晚几秒。
///
/// - 最新一条结果行比 Bot 在旁聊里的最后一条消息新（或旁聊里还没有 Bot 的消息）：按结果行。
/// - 没有支持声明：有过结果行按它，没有是 `Untracked`。
/// - 有支持声明：Bot 最后一条消息过了 [`RESULT_WAIT`] 还没等到新的结果行、也没有在跑的状态，是 `ResultMissing`；
///   没过就还是上一条结果行的状态，没有结果行是 `Open`。
pub(super) fn card_state(
    support: bool,
    result: Option<&ResultLine>,
    activity: Option<&ThreadActivity>,
    now_ms: u64,
) -> SideChatState {
    let last = activity.and_then(|activity| activity.last_reply_ms);
    let running = activity.is_some_and(|activity| activity.running);
    let of = |line: &ResultLine| match line.state {
        Some(SideChatResultState::Done) => SideChatState::Done,
        Some(SideChatResultState::Failed) => SideChatState::Failed,
        Some(SideChatResultState::Ended) => SideChatState::Ended,
        None => SideChatState::Other,
    };
    if let Some(line) = result.filter(|line| last.is_none_or(|last| line.timestamp_ms >= last)) {
        return of(line);
    }
    if !support {
        return result.map_or(SideChatState::Untracked, of);
    }
    let wait = u64::try_from(RESULT_WAIT.as_millis()).unwrap_or(u64::MAX);
    let overdue = last.is_some_and(|last| now_ms >= last.saturating_add(wait));
    if overdue && !running {
        return SideChatState::ResultMissing;
    }
    result.map_or(SideChatState::Open, of)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// 一个旁聊卡片上除了回复数以外的部分。
#[derive(Clone, Debug, PartialEq, Eq)]
struct CardInfo {
    title: String,
    unread: u64,
    replies: u64,
    state: SideChatState,
}

/// 时间线里显示旁聊要用的情况：每个旁聊的标题、未读、状态，以及这条时间线是不是旁聊。每秒重算一次，变了的条目重发。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SideChatsView {
    /// 这条时间线是哪个旁聊的；主对话是 `None`。
    thread: Option<OwnedEventId>,
    cards: HashMap<OwnedEventId, CardInfo>,
}

impl SideChatsView {
    /// 从本机缓存的旁聊情况算出来。
    pub(super) fn of(
        thread: Option<OwnedEventId>,
        facts: &SideChatFacts,
        support: bool,
        now_ms: u64,
    ) -> Self {
        let roots: HashSet<&OwnedEventId> =
            facts.threads.keys().chain(facts.results.keys()).collect();
        let cards = roots
            .into_iter()
            .map(|root| {
                let result = facts.results.get(root);
                let activity = facts.threads.get(root);
                let title = facts
                    .titles
                    .get(root)
                    .or(result.map(|line| &line.title))
                    .cloned()
                    .unwrap_or_default();
                let info = CardInfo {
                    title,
                    unread: activity.map_or(0, |activity| activity.unread),
                    replies: activity.map_or(0, |activity| activity.replies),
                    state: card_state(support, result, activity, now_ms),
                };
                (root.clone(), info)
            })
            .collect();
        Self { thread, cards }
    }

    /// 读本机缓存再算一遍。
    pub(super) async fn load(room: &ChatRoom, support: bool) -> Self {
        let facts = room.side_chat_facts().await;
        Self::of(
            room.thread_root().map(ToOwned::to_owned),
            &facts,
            support,
            now_ms(),
        )
    }

    /// 旁聊卡；`replies` 是 SDK 的话题汇总给的回复数，没有时用本机缓存里数的。
    fn card(&self, root: &OwnedEventId, replies: Option<u64>, title: &str) -> SideChatCard {
        let info = self.cards.get(root);
        SideChatCard {
            root_event_id: root.to_string(),
            title: info
                .map(|info| info.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| title.to_owned()),
            reply_count: replies.or(info.map(|info| info.replies)).unwrap_or(0),
            unread_count: info.map_or(0, |info| info.unread),
            state: info.map_or(SideChatState::Open, |info| info.state),
        }
    }

    /// 旁聊订阅的头部：标题和状态（主对话是 `None`）。
    pub(super) fn header(&self) -> Option<SideChatCard> {
        let root = self.thread.as_ref()?;
        Some(self.card(root, None, ""))
    }

    /// 这条消息在时间线里按旁聊显示的样子：结果行、带标记的根。别的返回 `None`，照普通消息显示。
    pub(super) fn content(&self, message: &Message) -> Option<TimelineContent> {
        if self.thread.is_some() {
            return None;
        }
        if message.thread_root.is_none()
            && let Some(line) = result_line(&message.custom, message.timestamp_ms)
            && let Some(state) = line.state
        {
            return Some(TimelineContent::SideChatResult {
                root_event_id: line.root.to_string(),
                title: line.title,
                state: match state {
                    SideChatResultState::Done => SideChatState::Done,
                    SideChatResultState::Failed => SideChatState::Failed,
                    SideChatResultState::Ended => SideChatState::Ended,
                },
            });
        }
        let root = message.event_id.as_ref()?;
        let marker = root_marker(&message.custom)?;
        Some(TimelineContent::SideChat {
            card: self.card(root, message.thread_replies, &marker.title),
        })
    }

    /// 有回复的普通消息带的卡片（只在主对话里）。
    pub(super) fn card_of(&self, message: &Message) -> Option<SideChatCard> {
        if self.thread.is_some() {
            return None;
        }
        let replies = message.thread_replies.filter(|replies| *replies > 0)?;
        let root = message.event_id.as_ref()?;
        let title = match &message.body {
            wonton_matrix::MessageBody::Text(body) | wonton_matrix::MessageBody::Notice(body) => {
                title_from_body(body)
            }
            _ => String::new(),
        };
        Some(self.card(root, Some(replies), &title))
    }

    /// 这条消息的显示会不会随旁聊情况变：结果行、根、有回复的消息。
    pub(super) fn affects(message: &Message) -> bool {
        message.thread_replies.is_some()
            || root_marker(&message.custom).is_some()
            || result_line(&message.custom, 0).is_some()
    }
}

/// 各房间的旁聊支持声明（ADR-0093 §1）。存的是通过了前两条信任规则（`state_key` 和发送者相同、发送者是房间成员）
/// 的声明者；“是核心认得的 Bot”在用的时候按当时的 Bot 列表看，Bot 列表晚到也不用重取。
#[derive(Default)]
pub(crate) struct Support {
    declared: Mutex<HashMap<OwnedRoomId, Vec<OwnedUserId>>>,
    /// 取过（或正在取）的房间和第几次取：聊天列表每次更新不重复取；同一个房间先后取了两次时，只认后一次的结果
    /// （先发的可能后回来，带着声明之前的状态）。
    asked: Mutex<HashMap<OwnedRoomId, u64>>,
    changed: watch::Sender<u64>,
}

impl Support {
    /// 房间里有没有认得的 Bot 声明了支持。
    pub(crate) fn supports(&self, room_id: &OwnedRoomId, bots: &HashSet<String>) -> bool {
        lock(&self.declared)
            .get(room_id)
            .is_some_and(|users| users.iter().any(|user| bots.contains(user.as_str())))
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// 还没取过的房间取一次。
    pub(crate) fn ensure(
        self: &Arc<Self>,
        runtime: &Handle,
        session: &Arc<MatrixSession>,
        room_id: &OwnedRoomId,
    ) {
        if !lock(&self.asked).contains_key(room_id) {
            self.refresh(runtime, session, room_id);
        }
    }

    /// 向服务器重取一次房间状态，核对声明。取不到时保留上一次的结果，只记日志：没有声明的房间照常聊天，
    /// 只是不出开旁聊的按钮。
    pub(crate) fn refresh(
        self: &Arc<Self>,
        runtime: &Handle,
        session: &Arc<MatrixSession>,
        room_id: &OwnedRoomId,
    ) {
        let generation = {
            let mut asked = lock(&self.asked);
            let generation = asked.get(room_id).map_or(0, |last| last + 1);
            asked.insert(room_id.clone(), generation);
            generation
        };
        let support = self.clone();
        let session = session.clone();
        let room_id = room_id.clone();
        runtime.spawn(async move {
            match session.room_state(&room_id).await {
                Ok(_) if lock(&support.asked).get(&room_id) != Some(&generation) => {}
                Ok(state) => {
                    let users = declarers(&state);
                    let changed =
                        lock(&support.declared).insert(room_id, users.clone()) != Some(users);
                    if changed {
                        support.changed.send_modify(|count| *count += 1);
                    }
                }
                Err(error) => {
                    tracing::info!(%room_id, "couldn't read the side chat declaration: {error}");
                }
            }
        });
    }

    pub(crate) fn clear(&self) {
        lock(&self.declared).clear();
        lock(&self.asked).clear();
    }
}

/// 房间状态里有效的支持声明者：`v` 是 1、`state_key` 和发送者相同、发送者是已加入的成员。
pub(super) fn declarers(state: &[Value]) -> Vec<OwnedUserId> {
    let joined: HashSet<&str> = state
        .iter()
        .filter(|event| {
            event["type"] == "m.room.member" && event["content"]["membership"] == "join"
        })
        .filter_map(|event| event["state_key"].as_str())
        .collect();
    state
        .iter()
        .filter(|event| event["type"] == SIDE_CHAT_SUPPORT_EVENT)
        .filter(|event| event["content"]["v"].as_u64() == Some(1))
        .filter(|event| event["state_key"] == event["sender"])
        .filter_map(|event| event["sender"].as_str())
        .filter(|sender| joined.contains(sender))
        .filter_map(|sender| sender.parse().ok())
        .collect()
}

impl ChatLink {
    /// 开旁聊（ADR-0093 §4）：发根消息、等服务器回事件 ID，再把第一句话放进这个话题的发送队列，返回根的事件 ID。
    /// 根没发出去时按原因返回错误，房间里不留空旁聊。
    pub(crate) async fn start_side_chat(
        &self,
        room_id: &str,
        body: &str,
    ) -> Result<String, CoreError> {
        let body = body.trim();
        if body.is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let (_, room_id) = self.send_target(room_id)?;
        let room = self
            .room(&TimelineKey::main(room_id.clone()))
            .await
            .map_err(|e| core_error(&e))?;
        let title = title_from_body(body);
        let root = room
            .send_side_chat_root(&title)
            .await
            .map_err(|e| core_error(&e))?;
        room.queue_text_in_thread(&root, body)
            .await
            .map_err(|e| core_error(&e))?;
        onboarding::record_sent(&self.sent, room_id.to_string());
        Ok(root.to_string())
    }
}

#[cfg(test)]
#[path = "side_chat_tests.rs"]
mod tests;
