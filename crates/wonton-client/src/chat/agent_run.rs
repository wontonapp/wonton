//! Agent 一轮任务的进度（ADR-0082）：把 Bot 发的状态消息合成进时间线，和 Bot 的在线状态对表；
//! "以反应回答"把主人点中的选项经 Matrix 发成 `m.annotation` 反应。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wonton_matrix::AgentRunStatus;
use wonton_protocol::{
    AgentRunState, BotActivity, ChatViewModel, ConfirmationRequester, ConfirmationState,
    ConfirmationView, CoreError, TimelineContent,
};

use super::ChatLink;

/// 合成 AgentRun 条目要用的 Bot 情况：Bot 列表和每个账号的活动来自 [`ChatViewModel`]；这个房间里
/// 等回答的卡片来自「等你确认」列表按 `origin.room_id` 过滤后的房间卡片（ADR-0016：卡片不在时间线里）。
/// Bot 列表、活动或卡片变了，AgentRun 条目的显示跟着变，所以每批都带上一份。
#[derive(Clone, Default, PartialEq)]
pub(super) struct BotsView {
    users: HashSet<String>,
    /// 账号 → 当前活动；没有或为 `None` 都算不确定。
    activity: HashMap<String, Option<BotActivity>>,
    /// Bot 账号 → 这个房间里它等回答的卡片（多张时取最早到期的）。
    waiting_cards: HashMap<String, String>,
}

impl BotsView {
    /// Bot 列表、活动、这个房间等回答的卡片，一次算齐。
    pub(super) fn of(view: &ChatViewModel, cards: &[ConfirmationView]) -> Self {
        Self {
            users: view.bots.iter().map(|bot| bot.user_id.clone()).collect(),
            activity: view
                .bots
                .iter()
                .map(|bot| (bot.user_id.clone(), bot.activity))
                .collect(),
            waiting_cards: waiting_cards_from(cards),
        }
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
    /// 以反应回答 Bot 的提问：主人点了状态行上的选项按钮才发，核心把它发成对状态消息的 `m.annotation`
    /// 反应。只对现在还是状态消息、还带这个选项的条目发；失败进 `ChatViewModel::problem`。
    pub(super) fn answer_agent_choice(
        self: &Arc<Self>,
        room_id: &str,
        item_id: &str,
        key: &str,
    ) -> Result<(), CoreError> {
        let room = self.opened(room_id)?;
        let item_id = item_id.to_owned();
        let key = key.to_owned();
        self.spawn_op("chat-op-agent-answer-failed", async move {
            room.answer_agent_choice(&item_id, &key).await
        });
        Ok(())
    }
}
