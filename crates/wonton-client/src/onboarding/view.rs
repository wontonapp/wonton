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

/// 主人已经给默认 Agent 的私聊发过消息。发布时据此先把 `prompts_used` 落盘再渲染：收起提问的视图
/// 出去时记录一定已经在盘上（`OnboardingLink::publish`），App 杀在中间也不会再给一次提问。
pub(super) fn prompt_done(
    onboarding: Option<&Onboarding>,
    chat: &ChatViewModel,
    sent_rooms: &HashSet<String>,
) -> bool {
    let Some(bot) = onboarding.and_then(|onboarding| {
        onboarding
            .primary_agent
            .as_deref()
            .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id))
            .and_then(|agent| agent.bot_user_id.as_deref())
    }) else {
        return false;
    };
    chat.rooms.iter().any(|room| {
        sent_rooms.contains(&room.room_id) && room.direct_user_id.as_deref() == Some(bot)
    })
}

pub(super) fn render(
    state: &LocalState,
    records: &storage::Records,
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
    // 引导流程里显示这次记下的选择；流程外（设置里的“默认 Agent”）显示服务器上的现值，
    // 不让上次没走完的草稿冒充当前设置。
    let in_flow = matches!(
        state.step,
        Some(step) if !matches!(step, OnboardingStep::Finished | OnboardingStep::Other)
    );
    let selected = if in_flow {
        state.draft.agent(onboarding)
    } else {
        onboarding.primary_agent.clone()
    };
    let agents = agent_rows(onboarding, selected.as_deref());
    let selected_agent = selected
        .as_deref()
        .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id));
    let primary_bot = onboarding
        .primary_agent
        .as_deref()
        .and_then(|id| onboarding.agents.iter().find(|agent| agent.id == id))
        .and_then(|agent| agent.bot_user_id.clone());
    // 收起只看盘上的 `prompts_used`：发过消息的落盘在 `OnboardingLink::publish` 里、渲染用同一份快照，
    // 收起的视图出去时记录一定已经在盘上。房间行不参与收起（行列表抖动不收起），壳只在 Bot 私聊里显示。
    let chat_prompts = if onboarding.step == OnboardingStep::Finished
        && primary_bot.is_some()
        && !records.prompts_used
    {
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
    // `step` 为空就是“不在引导里”（关掉重看、还没取到），不再兜底回第一步，壳好回到设置。
    let step = state
        .too_old
        .is_none()
        .then(|| match state.step {
            Some(step) if !state.unavailable.contains(&step) => Some(step),
            Some(_) => first_visible(&state.unavailable),
            None => None,
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
        can_close: ready && not_started && state.revisit,
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
        ChatAccount, ChatSync, DeviceVerification, EncryptionStatus, PushStatus, RecoveryNextStep,
        RecoveryStatus, RoomRow,
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
                can_reset: false,
                recovery_next_step: RecoveryNextStep::None,
                verification_flow: None,
                key_handoff: None,
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
                side_chats: false,
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

    fn render_with(records: &storage::Records) -> OnboardingViewModel {
        let state = LocalState {
            onboarding: Some(onboarding()),
            ..LocalState::default()
        };
        render(&state, records).0
    }

    #[test]
    fn prompts_show_until_prompts_used_is_recorded() {
        // 收起只看盘上的记录（落盘在 publish 里、渲染用同一份快照），房间行不参与。
        let records = storage::Records::default();
        let view = render_with(&records);
        assert_eq!(view.chat_prompts, ["问我能做什么"]);

        let used = storage::Records {
            prompts_used: true,
            ..storage::Records::default()
        };
        let view = render_with(&used);
        assert!(view.chat_prompts.is_empty());
    }

    #[test]
    fn a_message_to_the_bot_is_what_gets_prompts_used_recorded() {
        // 收起提问只由 publish 落盘的 `prompts_used` 决定；房间记进发过消息本身不收起，
        // 行列表抖动（行短暂缺失）也不收起，发过消息的发布到了才落盘、才收。
        let bot_chat = chat("!room:home", Some("@hermes:home"));
        assert!(!prompt_done(
            Some(&onboarding()),
            &bot_chat,
            &HashSet::new()
        ));
        let sent: HashSet<String> = ["!room:home".to_owned()].into();
        assert!(prompt_done(Some(&onboarding()), &bot_chat, &sent));

        // 别的房间发过消息不算。
        let other: HashSet<String> = ["!other:home".to_owned()].into();
        assert!(!prompt_done(Some(&onboarding()), &bot_chat, &other));
        // 私聊标记缺失的行不算。
        let plain = chat("!room:home", None);
        assert!(!prompt_done(Some(&onboarding()), &plain, &sent));
    }

    #[test]
    fn prompts_come_back_after_a_server_switch() {
        // 换服务器后 storage.sync_server 清掉提问记录；发过消息的记录没了，候选提问恢复。
        let records = storage::Records {
            prompts_used: true,
            prompt_rooms: vec!["!room:home".to_owned()],
            prompt_server_id: Some("old".to_owned()),
            ..storage::Records::default()
        };
        let view = render_with(&records);
        assert!(view.chat_prompts.is_empty());

        let reset = storage::Records {
            prompt_server_id: Some("new".to_owned()),
            ..storage::Records::default()
        };
        let view = render_with(&reset);
        assert_eq!(view.chat_prompts, ["问我能做什么"]);
    }
}
