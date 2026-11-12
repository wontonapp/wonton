//! 从状态和记录算出壳要显示的 [`OnboardingViewModel`]：这一步的行、最后一页、提示和默认 Agent 的房间置顶。

use std::collections::HashSet;

use wonton_protocol::{
    ChatViewModel, Onboarding, OnboardingAgent, OnboardingAgentRow, OnboardingCommit,
    OnboardingModelStep, OnboardingNeedRow, OnboardingReview, OnboardingStep, OnboardingViewModel,
    PluginState, ReplacedAgent, StepNav,
};

use super::{LocalState, review, storage};

pub(super) fn render(
    state: &LocalState,
    records: &storage::Records,
    chat: &ChatViewModel,
    sent_rooms: &HashSet<String>,
) -> (OnboardingViewModel, Option<String>) {
    let Some(onboarding) = state.onboarding.as_ref() else {
        return (
            OnboardingViewModel {
                loading: state.loading,
                problem: state.problem.clone(),
                tips: tips(records),
                ..OnboardingViewModel::default()
            },
            None,
        );
    };
    let selected = state.draft.agent(onboarding);
    let agents = onboarding
        .agents
        .iter()
        .map(|agent| OnboardingAgentRow {
            id: agent.id.clone(),
            title: agent.title.clone(),
            logins: agent
                .logins
                .iter()
                .map(|login| login.title.clone())
                .collect(),
            selected: selected.as_deref() == Some(agent.id.as_str()),
            source: agent
                .status
                .is_none()
                .then(|| agent.source.clone())
                .flatten(),
            blocked: agent.blocked.clone(),
        })
        .collect();
    let selected_agent = selected
        .as_deref()
        .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id));
    let primary_bot = onboarding
        .primary_agent
        .as_deref()
        .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id))
        .and_then(|agent| agent.bot_user_id.clone());
    let chat_prompts = if onboarding.step == OnboardingStep::Finished
        && primary_bot.is_some()
        && !records.prompts_used
        && chat.rooms.iter().any(|room| {
            !sent_rooms.contains(&room.room_id)
                && room.direct_user_id.as_deref() == primary_bot.as_deref()
        }) {
        onboarding.prompts.clone()
    } else {
        Vec::new()
    };
    let model = OnboardingModelStep {
        providers: state.providers.clone(),
        logins: selected_agent.map_or_else(Vec::new, |agent| agent.logins.clone()),
        choice: state
            .draft
            .model(state.providers.as_ref())
            .and_then(|model| model.choice()),
        testing: state.testing,
        problem: state.model_problem.clone(),
    };
    let selected_needs = state.draft.needs(onboarding);
    let needs = onboarding
        .needs
        .iter()
        .map(|need| OnboardingNeedRow {
            id: need.id.clone(),
            title: need.title.clone(),
            selected: selected_needs.contains(&need.id),
            plugins: need.plugins.clone(),
        })
        .collect();
    let step = state.step.unwrap_or(onboarding.step);
    let view = OnboardingViewModel {
        step: Some(step),
        nav: navigation(state, step, selected_agent),
        loading: state.loading,
        problem: state.problem.clone(),
        catalog_problem: onboarding.catalog_problem.clone(),
        profile: state.draft.profile_step(state.owner.as_ref()),
        agents,
        model,
        needs,
        review: OnboardingReview {
            items: state.review.iter().map(review::ReviewEntry::item).collect(),
            commit: state.commit,
        },
        primary_bot_user_id: primary_bot.clone(),
        chat_prompts,
        replaced_agent: replaced_agent(onboarding),
        tips: tips(records),
    };
    (view, primary_bot)
}

fn navigation(
    state: &LocalState,
    step: OnboardingStep,
    selected: Option<&OnboardingAgent>,
) -> Option<StepNav> {
    let not_started = state.commit == OnboardingCommit::NotStarted;
    let (position, can_next) = match step {
        OnboardingStep::Profile => (1, true),
        OnboardingStep::Agent => (2, selected.is_some_and(installable)),
        OnboardingStep::Model => (
            3,
            !state.testing && state.draft.model(state.providers.as_ref()).is_some(),
        ),
        OnboardingStep::Apps => (4, true),
        OnboardingStep::Review => (
            5,
            match state.commit {
                OnboardingCommit::NotStarted => review::can_commit(&state.review),
                OnboardingCommit::Running => false,
                OnboardingCommit::Done => true,
            },
        ),
        OnboardingStep::Finished | OnboardingStep::Other => return None,
    };
    let ready = !state.loading;
    Some(StepNav {
        position,
        total: 5,
        can_back: ready && not_started && position > 1,
        can_skip: ready && not_started && step != OnboardingStep::Review,
        can_next: ready && can_next,
    })
}

fn tips(records: &storage::Records) -> wonton_protocol::OnboardingTips {
    wonton_protocol::OnboardingTips {
        confirmation_card: !records.confirmation_card,
        app_installed: !records.app_installed,
    }
}

/// `blocked` 说的是这台主机装不了它；已经装着的（`status` 有值）不用再装，不受它拦。
pub(super) fn installable(agent: &OnboardingAgent) -> bool {
    agent.blocked.is_none() || agent.status.is_some()
}

fn replaced_agent(onboarding: &Onboarding) -> Option<ReplacedAgent> {
    if onboarding.replaced_agent_asked {
        return None;
    }
    // 列表第一项是官方推荐排最前的；它已经装着、而默认 Agent 是另一个且在运行，才问要不要卸掉它。
    let first = onboarding.agents.first()?;
    if first.status.is_none() || onboarding.primary_agent.as_deref() == Some(first.id.as_str()) {
        return None;
    }
    let primary = onboarding
        .agents
        .iter()
        .find(|agent| Some(agent.id.as_str()) == onboarding.primary_agent.as_deref())?;
    if !matches!(primary.status, Some(PluginState::Running { .. })) {
        return None;
    }
    Some(ReplacedAgent {
        id: first.id.clone(),
        title: first.title.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{
        ChatAccount, ChatSync, DeviceVerification, EncryptionStatus, PushStatus, RecoveryStatus,
        RoomRow,
    };

    fn onboarding() -> Onboarding {
        Onboarding {
            step: OnboardingStep::Finished,
            primary_agent: Some("hermes".to_owned()),
            replaced_agent_asked: true,
            agents: vec![OnboardingAgent {
                id: "hermes".to_owned(),
                title: "Hermes".to_owned(),
                source: None,
                status: Some(PluginState::Running {
                    health: wonton_protocol::PluginHealth::Healthy,
                }),
                bot_user_id: Some("@hermes:home".to_owned()),
                logins: Vec::new(),
                blocked: None,
            }],
            needs: Vec::new(),
            prompts: vec!["问我能做什么".to_owned()],
            catalog_problem: None,
        }
    }

    fn chat(room_id: &str, bot: Option<&str>) -> ChatViewModel {
        ChatViewModel {
            account: ChatAccount::SignedIn {
                user_id: "@owner:home".to_owned(),
                device_id: "DEVICE".to_owned(),
            },
            sync: ChatSync::Running,
            encryption: EncryptionStatus {
                verification: DeviceVerification::Unknown,
                recovery: RecoveryStatus::Unknown,
                recovery_key: None,
                backup_reminder: None,
            },
            rooms: vec![RoomRow {
                room_id: room_id.to_owned(),
                name: "Hermes".to_owned(),
                is_direct: true,
                direct_user_id: bot.map(str::to_owned),
                is_encrypted: true,
                is_read_only: false,
                membership: wonton_protocol::RoomMembership::Joined,
                preview: None,
                unread_count: 0,
            }],
            problem: None,
            push: PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            bots: Vec::new(),
        }
    }

    fn render_with(
        records: &storage::Records,
        chat: &ChatViewModel,
        sent_rooms: &HashSet<String>,
    ) -> OnboardingViewModel {
        let state = LocalState {
            onboarding: Some(onboarding()),
            ..LocalState::default()
        };
        render(&state, records, chat, sent_rooms).0
    }

    #[test]
    fn prompts_show_until_a_message_is_sent_to_the_bot() {
        let records = storage::Records::default();
        let chat = chat("!room:home", Some("@hermes:home"));
        let view = render_with(&records, &chat, &HashSet::new());
        assert_eq!(view.chat_prompts, ["问我能做什么"]);

        let sent: HashSet<String> = ["!room:home".to_owned()].into();
        let view = render_with(&records, &chat, &sent);
        assert!(view.chat_prompts.is_empty());
    }

    #[test]
    fn prompts_come_back_after_a_server_switch() {
        // 换服务器后 storage.sync_server 清掉提问记录；房间是旧的、发过消息的记录没了，候选提问恢复。
        let records = storage::Records {
            prompts_used: true,
            prompt_rooms: vec!["!room:home".to_owned()],
            prompt_server_id: Some("old".to_owned()),
            ..storage::Records::default()
        };
        let chat = chat("!room:home", Some("@hermes:home"));
        let sent: HashSet<String> = ["!room:home".to_owned()].into();
        let view = render_with(&records, &chat, &sent);
        assert!(view.chat_prompts.is_empty());

        let reset = storage::Records {
            prompt_server_id: Some("new".to_owned()),
            ..storage::Records::default()
        };
        let view = render_with(&reset, &chat, &HashSet::new());
        assert_eq!(view.chat_prompts, ["问我能做什么"]);
    }
}
