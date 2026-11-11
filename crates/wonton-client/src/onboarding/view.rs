//! 从状态和记录算出壳要显示的 [`OnboardingViewModel`]：这一步的行、最后一页、提示和默认 Agent 的房间置顶。

use std::collections::HashSet;

use wonton_protocol::{
    ChatViewModel, Onboarding, OnboardingAgent, OnboardingAgentRow, OnboardingCommit,
    OnboardingModelStep, OnboardingNeedRow, OnboardingReview, OnboardingStep, OnboardingViewModel,
    PluginState, ReplacedAgent, StepNav,
};

use super::{LocalState, review, storage};

/// 引导的步骤，按走的顺序。服务器太旧显不出来的步记在 `LocalState::unavailable` 里。
pub(super) const STEPS: [OnboardingStep; 5] = [
    OnboardingStep::Profile,
    OnboardingStep::Agent,
    OnboardingStep::Model,
    OnboardingStep::Apps,
    OnboardingStep::Review,
];

/// 第一个还显示得出来的步；一步都不剩时为空。
pub(super) fn first_visible(unavailable: &HashSet<OnboardingStep>) -> Option<OnboardingStep> {
    STEPS.into_iter().find(|step| !unavailable.contains(step))
}

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
    let agents = agent_rows(onboarding, selected.as_deref());
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
    let needs = need_rows(state, onboarding);
    // 服务器太旧的步跳过：正在显示的那步刚变不可用时换到下一个，一步都不剩时整段不显示。
    // 整段都打不开（服务器比引导接口还旧）时一步都不显示，只留升级提示。
    let step = state
        .too_old
        .is_none()
        .then(|| {
            state
                .step
                .filter(|step| !state.unavailable.contains(step))
                .or_else(|| first_visible(&state.unavailable))
        })
        .flatten();
    let view = OnboardingViewModel {
        step,
        nav: step.and_then(|step| navigation(state, step, selected_agent)),
        too_old: state.too_old.clone(),
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

fn agent_rows(onboarding: &Onboarding, selected: Option<&str>) -> Vec<OnboardingAgentRow> {
    onboarding
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
            selected: selected == Some(agent.id.as_str()),
            source: agent
                .status
                .is_none()
                .then(|| agent.source.clone())
                .flatten(),
            blocked: agent.blocked.clone(),
        })
        .collect()
}

fn need_rows(state: &LocalState, onboarding: &Onboarding) -> Vec<OnboardingNeedRow> {
    let selected = state.draft.needs(onboarding);
    onboarding
        .needs
        .iter()
        .map(|need| OnboardingNeedRow {
            id: need.id.clone(),
            title: need.title.clone(),
            selected: selected.contains(&need.id),
            plugins: need.plugins.clone(),
        })
        .collect()
}

fn navigation(
    state: &LocalState,
    step: OnboardingStep,
    selected: Option<&OnboardingAgent>,
) -> Option<StepNav> {
    let not_started = state.commit == OnboardingCommit::NotStarted;
    // 第几步按还显示得出来的步算：踢掉的步不占号。
    let mut visible = STEPS
        .iter()
        .filter(|step| !state.unavailable.contains(step));
    let position = visible.position(|candidate| *candidate == step)? as u32 + 1;
    let total = u32::try_from(visible.count()).unwrap_or(u32::MAX) + position;
    let can_next = match step {
        OnboardingStep::Profile | OnboardingStep::Apps => true,
        OnboardingStep::Agent => selected.is_some_and(installable),
        OnboardingStep::Model => {
            !state.testing && state.draft.model(state.providers.as_ref()).is_some()
        }
        OnboardingStep::Review => match state.commit {
            OnboardingCommit::NotStarted => review::can_commit(&state.review),
            OnboardingCommit::Running => false,
            OnboardingCommit::Done => true,
        },
        OnboardingStep::Finished | OnboardingStep::Other => return None,
    };
    let ready = !state.loading;
    Some(StepNav {
        position,
        total,
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
            chat_layout_default: Default::default(),
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
                can_verify_with_other_device: false,
                verification_flow: None,
            },
            installing_bots: Vec::new(),
            rooms: vec![RoomRow {
                layout: Default::default(),
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
            background_notifications: Vec::new(),
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
