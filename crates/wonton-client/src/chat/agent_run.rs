//! Agent 一轮任务的进度（ADR-0082）：把 Bot 发的状态消息合成进时间线，和 Bot 的在线状态对表；
//! "以反应回答"把主人点中的选项经 Matrix 发成 `m.annotation` 反应。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wonton_matrix::AgentRunStatus;
use wonton_protocol::{
    AgentRunState, BotActivity, ChatViewModel, ConfirmationRequester, CoreError, TimelineContent,
};

use super::ChatLink;

/// 合成 AgentRun 条目要用的 Bot 情况，来自 [`ChatViewModel`]，按房间取：Bot 列表、每个账号的活动、
/// 这个房间里等回答的卡片。Bot 列表或活动变了，AgentRun 条目的显示跟着变，所以每批都带上一份。
#[derive(Clone, Default, PartialEq)]
pub(super) struct BotsView {
    users: HashSet<String>,
    /// 账号 → 当前活动；没有或为 `None` 都算不确定。
    activity: HashMap<String, Option<BotActivity>>,
    /// Bot 账号 → 这个房间里它等回答的卡片（最早到期的在前，取第一张）。
    waiting_cards: HashMap<String, String>,
}

impl BotsView {
    pub(super) fn of(view: &ChatViewModel, room_id: &str) -> Self {
        Self {
            users: view.bots.iter().map(|bot| bot.user_id.clone()).collect(),
            activity: view
                .bots
                .iter()
                .map(|bot| (bot.user_id.clone(), bot.activity))
                .collect(),
            waiting_cards: view
                .pending_confirmations
                .iter()
                // 列表按最早到期排前；同一个 Bot 有几张等回答的卡片时取最早的那张。
                .fold(HashMap::new(), |mut cards, card| {
                    match (&card.requester, &card.room_id) {
                        (Some(ConfirmationRequester::Bot { user_id, .. }), Some(card_room))
                            if card_room == room_id =>
                        {
                            cards
                                .entry(user_id.clone())
                                .or_insert_with(|| card.id.clone());
                        }
                        _ => {}
                    }
                    cards
                }),
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
