//! Agent 一轮任务的进度（ADR-0082）：把 Bot 发的状态消息合成进时间线，和 Bot 的在线状态对表；
//! "以反应回答"把主人点中的选项经 Matrix 发成 `m.annotation` 反应。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wonton_matrix::{AgentRunStatus, ChatRoom, EntryKind, TimelineEntry, Vector};
use wonton_protocol::{
    AgentRunState, BotActivity, ChatViewModel, ConfirmationRequester, ConfirmationState, CoreError,
    TimelineContent,
};

use super::confirmation::CardsView;
use super::side_chat::{SideChatsView, Support};
use super::{ChatLink, TimelineKey, lock};

/// 合成 AgentRun 条目要用的 Bot 情况：Bot 列表和每个账号的活动来自 [`ChatViewModel`]；这个房间里
/// 等回答的卡片来自房间自己的时间线（ADR-0082 第 4 节：卡片在这个对话里）。Bot 列表、活动或卡片变了，
/// AgentRun 条目的显示跟着变，所以每批都带上一份。
#[derive(Clone, Default, PartialEq)]
pub(super) struct BotsView {
    users: HashSet<String>,
    /// 账号 → 当前活动；没有或为 `None` 都算不确定。
    activity: HashMap<String, Option<BotActivity>>,
    /// Bot 账号 → 这个房间里它等回答的卡片（多张时取最早到期的）。
    waiting_cards: HashMap<String, String>,
    /// 这个房间里旁聊的情况（ADR-0093）：卡片、结果行的状态也跟着 Bot 走，一起带着。
    side_chats: SideChatsView,
}

impl BotsView {
    /// Bot 列表、活动、房间里的等回答卡片，一次算齐。
    pub(super) fn of(
        view: &ChatViewModel,
        entries: &Vector<TimelineEntry>,
        cards: &CardsView,
    ) -> Self {
        Self {
            users: view.bots.iter().map(|bot| bot.user_id.clone()).collect(),
            activity: view
                .bots
                .iter()
                .map(|bot| (bot.user_id.clone(), bot.activity))
                .collect(),
            waiting_cards: waiting_cards_from(entries, cards),
            side_chats: SideChatsView::default(),
        }
    }

    /// 换上重算过的旁聊情况。
    pub(super) fn with_side_chats(&self, side_chats: SideChatsView) -> Self {
        Self {
            side_chats,
            ..self.clone()
        }
    }

    pub(super) fn side_chats(&self) -> &SideChatsView {
        &self.side_chats
    }

    /// 房间里有没有认得的 Bot 声明了支持旁聊（ADR-0093 §1 的信任规则）。
    pub(super) fn supports(&self, support: &Support, room: &ChatRoom) -> bool {
        support.supports(&room.room_id().to_owned(), &self.users)
    }

    /// 房间里的等回答卡片变了（来了、答了、到期了）而 Bot 列表没变时，只重算卡片映射。
    pub(super) fn with_waiting_cards(
        &self,
        entries: &Vector<TimelineEntry>,
        cards: &CardsView,
    ) -> Self {
        Self {
            waiting_cards: waiting_cards_from(entries, cards),
            ..self.clone()
        }
    }

    pub(super) fn is_bot(&self, sender: &str) -> bool {
        self.users.contains(sender)
    }

    /// Bot 离线或活动未知时，还停在 working、waiting 的状态合成成不确定，不转圈（ADR-0082 第 4 节）。
    fn visible(&self, sender: &str, raw: &AgentRunState) -> AgentRunState {
        let online = matches!(
            self.activity.get(sender),
            Some(Some(activity)) if !matches!(activity, BotActivity::Offline)
        );
        match raw {
            AgentRunState::Working | AgentRunState::Waiting if !online => AgentRunState::Uncertain,
            state => state.clone(),
        }
    }

    /// 把状态对象合成成时间线条目；waiting 且这个 Bot 在这个房间有等回答的核心卡片时带上卡片 ID。
    /// `body` 是消息本身的正文（写给不认识这个格式的客户端的话）。
    pub(super) fn content(
        &self,
        status: &AgentRunStatus,
        body: &str,
        sender: &str,
    ) -> TimelineContent {
        TimelineContent::AgentRun {
            run: status.run.clone(),
            state: self.visible(sender, &status.state),
            step: status.step.clone(),
            steps: status.steps,
            progress: status.progress,
            result: status.result.clone(),
            end: status.end.clone(),
            body: body.to_owned(),
            card_id: (status.state == AgentRunState::Waiting)
                .then(|| self.waiting_cards.get(sender).cloned())
                .flatten(),
            choices: status.choices.clone(),
        }
    }
}

/// 这个房间里每个 Bot 等回答的卡片：时间线里的卡片消息按发起者归组，多张时取最早到期的。
/// 答过、到期、发送中的不算（和提示条列表同一个口径）。
fn waiting_cards_from(
    entries: &Vector<TimelineEntry>,
    cards: &CardsView,
) -> HashMap<String, String> {
    let mut found: HashMap<String, (u64, String)> = HashMap::new();
    for entry in entries.iter() {
        let EntryKind::Message(message) = &entry.kind else {
            continue;
        };
        let Some(view) = cards.view_of(message) else {
            continue;
        };
        let card = view.card;
        if view.sending.is_some() || card.state != ConfirmationState::Pending {
            continue;
        }
        let Some(ConfirmationRequester::Bot { user_id, .. }) = &card.requester else {
            continue;
        };
        match found.entry(user_id.clone()) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if card.expires_at_ms < slot.get().0 {
                    slot.insert((card.expires_at_ms, card.id.clone()));
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert((card.expires_at_ms, card.id.clone()));
            }
        }
    }
    found
        .into_iter()
        .map(|(user, (_, id))| (user, id))
        .collect()
}

impl ChatLink {
    /// 以反应回答 Bot 的提问：主人点了状态行上的选项按钮才发，核心把它发成对状态消息的 `m.annotation`
    /// 反应。只对现在还是状态消息、还带这个选项的条目发；失败进 `ChatViewModel::problem`。
    pub(super) fn answer_agent_choice(
        self: &Arc<Self>,
        room_id: &str,
        item_id: &str,
        key: &str,
    ) -> Result<(), CoreError> {
        let (_, room_id) = self.target(room_id)?;
        // 状态行可能在主对话里，也可能在打开着的旁聊里（ADR-0093）：在哪条时间线里找到就在哪里回答。
        let rooms = lock(&self.open).all_in(&room_id);
        if rooms.is_empty() {
            return Err(super::target::not_subscribed(&TimelineKey::main(room_id)));
        }
        let item_id = item_id.to_owned();
        let key = key.to_owned();
        self.spawn_op("chat-op-agent-answer-failed", async move {
            let mut outcome = Ok(());
            for room in rooms {
                outcome = room.answer_agent_choice(&item_id, &key).await;
                if !matches!(outcome, Err(wonton_matrix::Error::InvalidInput(_))) {
                    break;
                }
            }
            outcome
        });
        Ok(())
    }
}
