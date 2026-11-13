//! 旁聊（ADR-0093）在 Matrix 一侧要读的东西：根消息和结果行的自定义字段、标题的取法，以及从本机事件缓存里
//! 收集的结果行和每个话题的活动（未读、对方最后一条消息、还在跑的 Agent 状态）。卡片的状态怎么合成由客户端核心定。

use std::collections::HashMap;

use matrix_sdk::{
    Room,
    ruma::{EventId, OwnedEventId, OwnedUserId, UserId},
};
use serde_json::{Map, Value};
use wonton_protocol::{AgentRunState, SideChatResultState, SideChatRoot};

use crate::{ChatRoom, agent_run};

/// 根消息上的自定义字段。
pub const SIDE_CHAT_FIELD: &str = "app.wonton.side_chat";
/// 结果行上的自定义字段。
pub const SIDE_CHAT_RESULT_FIELD: &str = "app.wonton.side_chat_result";
/// 支持声明的状态事件类型，`state_key` 是 Bot 自己的 Matrix ID。
pub const SIDE_CHAT_SUPPORT_EVENT: &str = "app.wonton.bot.side_chats";
/// 标题最多几个字符（按 Unicode 标量算，不按字节）。
const TITLE_CHARS: usize = 20;

/// 别的客户端开的话题没有标题字段时，按正文第一行的前 20 个字符取；App 开旁聊时也用它从第一句话取标题。
pub fn title_from_body(body: &str) -> String {
    body.trim_start()
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(TITLE_CHARS)
        .collect()
}

/// 根消息的标记：`v` 是 1 且有标题才算，别的按普通 `m.notice` 显示（ADR-0093 §7）。
pub fn root_marker(custom: &Map<String, Value>) -> Option<SideChatRoot> {
    let root: SideChatRoot = serde_json::from_value(custom.get(SIDE_CHAT_FIELD)?.clone()).ok()?;
    (root.v == 1).then_some(root)
}

/// 主对话里的一条结果行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultLine {
    pub root: OwnedEventId,
    pub title: String,
    /// 结果行写的状态；不认识时为 `None`（卡片落到 `Other`，这一行按普通提示显示正文）。
    pub state: Option<SideChatResultState>,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
}

/// 读结果行：`v` 是 1、`root`（事件 ID）和 `title` 齐全、`state` 是字符串才算，否则按普通 `m.notice` 显示。
pub fn result_line(custom: &Map<String, Value>, timestamp_ms: u64) -> Option<ResultLine> {
    let field = custom.get(SIDE_CHAT_RESULT_FIELD)?.as_object()?;
    if field.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let root = field.get("root")?.as_str()?.parse().ok()?;
    let title = field.get("title")?.as_str()?.to_owned();
    let state = field.get("state")?.as_str()?;
    Some(ResultLine {
        root,
        title,
        state: serde_json::from_value(Value::from(state)).ok(),
        timestamp_ms,
    })
}

/// 一个话题在本机缓存里的活动。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadActivity {
    /// 按话题已读回执算的未读条数。
    pub unread: u64,
    /// 本机缓存里数得到的回复条数（根不算，编辑不算）。
    pub replies: u64,
    /// 对方（不是本账号）在话题里最后一条消息的时间，不算 Agent 的状态消息（ADR-0082）。
    pub last_reply_ms: Option<u64>,
    /// 话题里还有停在 `working`、`waiting` 的状态消息。
    pub running: bool,
}

/// 一个房间里旁聊的情况：结果行（每个旁聊只留最新一条）、各话题的活动和标题。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideChatFacts {
    pub results: HashMap<OwnedEventId, ResultLine>,
    pub threads: HashMap<OwnedEventId, ThreadActivity>,
    /// 话题根的标题：根上有标记就用标记里的，没有按根的正文取。根不在本机缓存里时没有。
    pub titles: HashMap<OwnedEventId, String>,
}

/// 原始事件里要用的几个字段；不是 `m.room.message` 的不要。
pub(crate) struct RawEvent {
    pub(crate) event_id: Option<OwnedEventId>,
    sender: Option<OwnedUserId>,
    timestamp_ms: u64,
    pub(crate) content: Map<String, Value>,
}

impl RawEvent {
    pub(crate) fn parse(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("m.room.message") {
            return None;
        }
        let id = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            event_id: id("event_id").and_then(|id| id.parse().ok()),
            sender: id("sender").and_then(|id| id.parse().ok()),
            timestamp_ms: value
                .get("origin_server_ts")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            content: value
                .get("content")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn relation(&self) -> Option<(String, OwnedEventId)> {
        let relates = self.content.get("m.relates_to")?.as_object()?;
        let kind = relates.get("rel_type")?.as_str()?.to_owned();
        let target = relates.get("event_id")?.as_str()?.parse().ok()?;
        Some((kind, target))
    }

    pub(crate) fn thread_root(&self) -> Option<OwnedEventId> {
        self.relation()
            .filter(|(kind, _)| kind == "m.thread")
            .map(|(_, root)| root)
    }

    fn body(&self) -> &str {
        self.content
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }
}

impl ChatRoom {
    /// 从本机事件缓存收集这个房间的旁聊情况，只读本机、不等网络。缓存里没有的（更早、没翻到的）不知道。
    pub async fn side_chat_facts(&self) -> SideChatFacts {
        let Ok((cache, _handles)) = self.room.event_cache().await else {
            return SideChatFacts::default();
        };
        let events = cache.events().await.unwrap_or_default();
        let parsed: Vec<RawEvent> = events
            .iter()
            .filter_map(|event| RawEvent::parse(event.raw().json().get()))
            .collect();
        let (mut facts, roots) = facts_of(&parsed);
        for root in roots {
            let activity = self.thread_activity(&root).await;
            facts.threads.insert(root, activity);
        }
        facts
    }

    /// 一个话题在本机缓存里的活动（未读、对方最后一条、还在跑的状态）。缓存还没有这个话题时都是空的。
    pub async fn thread_activity(&self, root: &EventId) -> ThreadActivity {
        let client = self.room.client();
        let Ok((thread, _handles)) = client.event_cache().thread(self.room.room_id(), root).await
        else {
            return ThreadActivity::default();
        };
        let unread = thread.num_unread_messages().await.unwrap_or_default();
        let events = match thread.subscribe().await {
            Ok((events, _subscriber)) => events,
            Err(_) => Vec::new(),
        };
        let parsed: Vec<RawEvent> = events
            .iter()
            .filter_map(|event| RawEvent::parse(event.raw().json().get()))
            .collect();
        ThreadActivity {
            unread,
            ..activity_of(&parsed, self.room.own_user_id())
        }
    }
}

impl crate::MatrixSession {
    /// 向服务器取房间的全部状态事件（原始 JSON，含 `sender`、`state_key`）。旁聊的支持声明是自定义状态，不在
    /// Sliding Sync 的 `required_state` 里，本机没有，要问服务器（ADR-0093 §1）。
    pub async fn room_state(
        &self,
        room_id: &matrix_sdk::ruma::RoomId,
    ) -> crate::Result<Vec<Value>> {
        let response = self
            .client
            .send(
                matrix_sdk::ruma::api::client::state::get_state_events::v3::Request::new(
                    room_id.to_owned(),
                ),
            )
            .await?;
        Ok(response
            .room_state
            .iter()
            .filter_map(|raw| serde_json::to_value(raw).ok())
            .collect())
    }
}

/// 聊天列表预览里的旁聊（ADR-0093 §4）：房间最新一条是旁聊的根、结果行或话题里的消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatestSideChat {
    pub title: String,
    /// 最新一条是结果行时是它。
    pub result: Option<ResultLine>,
}

/// 房间最新一条（SDK 算的，话题里的也算）如果属于旁聊，给出旁聊的标题和结果行。话题里的消息要按根取标题，
/// 根不在本机缓存里时不算，按普通消息预览。
pub(crate) async fn latest_side_chat(room: &Room) -> Option<LatestSideChat> {
    let matrix_sdk_base::latest_event::LatestEventValue::Remote(event) = (**room).latest_event()
    else {
        return None;
    };
    let latest = RawEvent::parse(event.raw().json().get())?;
    if let Some(root) = latest.thread_root() {
        let (cache, _handles) = room.event_cache().await.ok()?;
        let root = cache.find_event(&root).await.ok()??;
        let root = RawEvent::parse(root.raw().json().get())?;
        let title = root_marker(&root.content)
            .map_or_else(|| title_from_body(root.body()), |marker| marker.title);
        return Some(LatestSideChat {
            title,
            result: None,
        });
    }
    if let Some(line) = result_line(&latest.content, latest.timestamp_ms) {
        return Some(LatestSideChat {
            title: line.title.clone(),
            result: Some(line),
        });
    }
    root_marker(&latest.content).map(|marker| LatestSideChat {
        title: marker.title,
        result: None,
    })
}

/// 房间里所有话题的未读加起来（按话题回执算）。话题根从本机缓存里的话题回复收集，缓存里没有的话题不知道。
pub(crate) async fn threads_unread(room: &Room) -> u64 {
    let Ok((cache, _handles)) = room.event_cache().await else {
        return 0;
    };
    let mut roots: Vec<OwnedEventId> = Vec::new();
    let events = cache.events().await.unwrap_or_default();
    for event in &events {
        if let Some(root) = RawEvent::parse(event.raw().json().get()).and_then(|e| e.thread_root())
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    let client = room.client();
    let event_cache = client.event_cache();
    let mut unread = 0;
    for root in roots {
        if let Ok((thread, _handles)) = event_cache.thread(room.room_id(), &root).await {
            unread += thread.num_unread_messages().await.unwrap_or_default();
        }
    }
    unread
}

/// 等本机的话题缓存把刚发的话题回执算进去（同步回来才算），最多等几秒。话题回执不算房间的变化，房间列表的未读
/// 不会自己重算：调用方等这一步之后再叫房间列表重算，数字才是新的。
pub(crate) async fn until_thread_read(room: &Room, root: &EventId) {
    let client = room.client();
    let Ok((thread, _handles)) = client.event_cache().thread(room.room_id(), root).await else {
        return;
    };
    let Ok(mut info) = thread.subscribe_to_thread_info().await else {
        return;
    };
    let read = async {
        let mut current = info.get().await;
        while current.read_receipts.num_unread > 0 {
            match info.next().await {
                Some(next) => current = next,
                None => return,
            }
        }
    };
    // 服务器一直不回这条回执时不卡住调用方：未读数等下一次同步再对。
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), read).await;
}

/// 房间缓存里的结果行和标题，以及出现过的话题根（有回复的、带标记的根）。缓存从旧到新，后来的结果行盖掉先来的。
pub(crate) fn facts_of(events: &[RawEvent]) -> (SideChatFacts, Vec<OwnedEventId>) {
    let mut facts = SideChatFacts::default();
    let mut roots: Vec<OwnedEventId> = Vec::new();
    for event in events {
        if let Some(root) = event.thread_root() {
            if !roots.contains(&root) {
                roots.push(root);
            }
            continue;
        }
        if let Some(line) = result_line(&event.content, event.timestamp_ms) {
            facts.results.insert(line.root.clone(), line);
        }
        if let (Some(id), Some(marker)) = (&event.event_id, root_marker(&event.content)) {
            facts.titles.insert(id.clone(), marker.title);
            if !roots.contains(id) {
                roots.push(id.clone());
            }
        }
    }
    for event in events {
        let Some(id) = &event.event_id else { continue };
        if roots.contains(id) && !facts.titles.contains_key(id) {
            facts
                .titles
                .insert(id.clone(), title_from_body(event.body()));
        }
    }
    (facts, roots)
}

/// 找对方最后一条消息、看状态消息还有没有在跑。状态消息靠编辑更新，按原消息记最新的状态。
pub(crate) fn activity_of(events: &[RawEvent], own: &UserId) -> ThreadActivity {
    let mut statuses: HashMap<OwnedEventId, AgentRunState> = HashMap::new();
    let mut activity = ThreadActivity::default();
    for event in events {
        if let Some((kind, original)) = event.relation()
            && kind == "m.replace"
        {
            let new_content = event
                .content
                .get("m.new_content")
                .and_then(Value::as_object);
            if let Some(status) = new_content.and_then(agent_run::parse) {
                statuses.insert(original, status.state);
            }
            continue;
        }
        activity.replies += 1;
        if let Some(status) = agent_run::parse(&event.content) {
            if let Some(id) = &event.event_id {
                statuses.entry(id.clone()).or_insert(status.state);
            }
            continue;
        }
        if event.sender.as_deref() != Some(own) {
            activity.last_reply_ms = activity.last_reply_ms.max(Some(event.timestamp_ms));
        }
    }
    activity.running = statuses
        .values()
        .any(|state| matches!(state, AgentRunState::Working | AgentRunState::Waiting));
    activity
}

#[cfg(test)]
#[path = "side_chat_tests.rs"]
mod tests;
