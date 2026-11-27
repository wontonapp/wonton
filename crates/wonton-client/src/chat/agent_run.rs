//! Agent 一轮任务的进度（ADR-0082）：把 Bot 发的状态消息合成进时间线，和 Bot 的在线状态对表；
//! "以反应回答"把主人点中的选项经 Matrix 发成 `m.annotation` 反应。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wonton_matrix::{AgentRunStatus, ChatRoom};
use wonton_protocol::{
    AgentRunState, BotActivity, ChatViewModel, ConfirmationRequester, ConfirmationState,
    ConfirmationView, CoreError, TimelineContent,
};

use super::side_chat::{SideChatsView, Support};
use super::{ChatLink, TimelineKey, lock};

/// 合成 AgentRun 条目要用的 Bot 情况：Bot 列表和每个账号的活动来自 [`ChatViewModel`]；这个房间里
/// 等回答的卡片来自「等你确认」列表按 `origin.room_id` 过滤后的房间卡片（ADR-0016：卡片不在时间线里）。
/// Bot 列表、活动或卡片变了，AgentRun 条目的显示跟着变，所以每批都带上一份。
#[derive(Clone, Default, PartialEq)]
pub(super) struct BotsView {
    /// 插件的已安装清单拿到没有：没拿到时 `users` 是空的，是「不认识任何 Bot」，不是「一个 Bot 都没有」。
    ready: bool,
    users: HashSet<String>,
    /// 账号 → 当前活动；没有或为 `None` 都算不确定。
    activity: HashMap<String, Option<BotActivity>>,
    /// Bot 账号 → 这个房间里它等回答的卡片（多张时取最早到期的）。
    waiting_cards: HashMap<String, String>,
    /// 这个房间里旁聊的情况（ADR-0093）：卡片、结果行的状态也跟着 Bot 走，一起带着。
    side_chats: SideChatsView,
}

impl BotsView {
    /// Bot 列表、活动、这个房间等回答的卡片，一次算齐。`ready` 是插件的已安装清单拿到没有。
    pub(super) fn of(view: &ChatViewModel, cards: &[ConfirmationView], ready: bool) -> Self {
        Self {
            ready,
            users: view.bots.iter().map(|bot| bot.user_id.clone()).collect(),
            activity: view
                .bots
                .iter()
                .map(|bot| (bot.user_id.clone(), bot.activity))
                .collect(),
            waiting_cards: waiting_cards_from(cards),
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

    /// 这个房间等回答的卡片变了（来了、答了、到期了）而 Bot 列表没变时，只重算卡片映射。
    pub(super) fn with_waiting_cards(&self, cards: &[ConfirmationView]) -> Self {
        Self {
            waiting_cards: waiting_cards_from(cards),
            ..self.clone()
        }
    }

    pub(super) fn is_bot(&self, sender: &str) -> bool {
        self.users.contains(sender)
    }

    /// 插件的已安装清单拿到没有：没拿到时 `users` 是空的，是「不认识任何 Bot」，不是「一个 Bot 都没有」。
    pub(super) fn ready(&self) -> bool {
        self.ready
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
    /// `body` 是消息本身的正文（写给不认识这个格式的客户端的话）；`timestamp_ms` 是这条状态消息
    /// 自己的发送时间（编辑不改），壳在 `working` 行用它算已经等了多久。
    pub(super) fn content(
        &self,
        status: &AgentRunStatus,
        body: &str,
        sender: &str,
        sender_name: &str,
        timestamp_ms: u64,
    ) -> TimelineContent {
        TimelineContent::AgentRun {
            run: status.run.clone(),
            trace: Vec::new(),
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
            sender_name: sender_name.to_owned(),
            timestamp_ms,
            unread_anchor: false,
        }
    }
}

/// 这个房间里每个 Bot 等回答的卡片：房间的确认卡片按发起者归组，多张时取最早到期的。
/// 答过、到期、发送中的不算（和提示条列表同一个口径）。
fn waiting_cards_from(cards: &[ConfirmationView]) -> HashMap<String, String> {
    let mut found: HashMap<String, (u64, String)> = HashMap::new();
    for view in cards {
        let card = &view.card;
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
    /// 以反应回答 Bot 的选项（ADR-0109 第 8 节）：主人点了消息下面或状态行上的按钮才发，核心把它发成对那条
    /// 消息的 `m.annotation` 反应。只有这条不在这份时间线里才换下一条，选项没有了、不是问我的、已经答过都就地拒绝；失败进 `ChatViewModel::problem`。
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
                // 只有“不在这份时间线里”才换一份再找；已经答过、不在等待这类拒绝就地生效。
                if !matches!(outcome, Err(wonton_matrix::Error::NotInTimeline(_))) {
                    break;
                }
            }
            outcome
        });
        Ok(())
    }
}
