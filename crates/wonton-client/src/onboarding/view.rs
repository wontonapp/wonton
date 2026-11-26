//! 从状态和记录算出壳要显示的 [`OnboardingViewModel`]：这一步的行、最后一页、提示和默认 Bot 的房间置顶。

use std::collections::HashSet;

use wonton_matrix::UserId;
use wonton_protocol::{
    ChatViewModel, DefaultBotPlugin, DefaultBotVisibility, ModelSettingsPage, Onboarding,
    OnboardingCommit, OnboardingModelStep, OnboardingNeedRow, OnboardingReview, OnboardingStep,
    OnboardingViewModel, StepNav,
};

use super::{LocalState, review, storage};

/// 引导的步骤，按走的顺序。服务器太旧显不出来的步记在 `LocalState::unavailable` 里。
pub(super) const STEPS: [OnboardingStep; 4] = [
    OnboardingStep::Profile,
    OnboardingStep::Model,
    OnboardingStep::Apps,
    OnboardingStep::Review,
];

/// 第一个还显示得出来的步；一步都不剩时为空。
pub(super) fn first_visible(unavailable: &HashSet<OnboardingStep>) -> Option<OnboardingStep> {
    STEPS.into_iter().find(|step| !unavailable.contains(step))
}

/// 主人已经给默认 Bot 的私聊发过消息。发布时据此先把 `prompts_used` 落盘再渲染：收起提问的视图
/// 出去时记录一定已经在盘上（`OnboardingLink::publish`），App 杀在中间也不会再给一次提问。
pub(super) fn prompt_done(
    onboarding: Option<&Onboarding>,
    chat: &ChatViewModel,
    sent_rooms: &HashSet<String>,
) -> bool {
    let Some(bot) = onboarding.and_then(|onboarding| {
        onboarding
            .default_bot_plugin
            .as_ref()
            .filter(|default| default.visibility == Some(DefaultBotVisibility::Visible))
            .and_then(|default| default.bot_user_id.as_deref())
            .filter(|id| <&UserId>::try_from(*id).is_ok())
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
                problem: state.problem.clone().or_else(|| state.bot_problem.clone()),
                tips: tips(records),
                ..OnboardingViewModel::default()
            },
            None,
        );
    };
    let default_bot = onboarding.default_bot_plugin.as_ref();
    let default_bot_visibility = Some(project_default_bot_visibility(state, default_bot));
    let default_bot_user_id = default_bot
        .filter(|_| default_bot_visibility == Some(DefaultBotVisibility::Visible))
        .and_then(|default| default.bot_user_id.clone());
    // 收起只看盘上的 `prompts_used`：发过消息的落盘在 `OnboardingLink::publish` 里、渲染用同一份快照，
    // 收起的视图出去时记录一定已经在盘上。房间行不参与收起（行列表抖动不收起），壳只在 Bot 私聊里显示。
    let chat_prompts = if onboarding.step == OnboardingStep::Finished
        && default_bot_user_id.is_some()
        && !records.prompts_used
    {
        onboarding.prompts.clone()
    } else {
        Vec::new()
    };
    let model = OnboardingModelStep {
        settings: state.model_settings.as_ref().and_then(|settings| {
            settings.web_url.as_ref().map(|web_url| ModelSettingsPage {
                plugin_id: settings.plugin_id.clone(),
                web_url: web_url.clone(),
            })
        }),
        has_configured_models: state
            .model_settings
            .as_ref()
            .is_some_and(|settings| settings.has_configured_models),
        choice: state
            .draft
            .model(state.model_settings.as_ref())
            .and_then(|model| model.choice()),
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
        nav: step.and_then(|step| navigation(state, step)),
        too_old: state.too_old.clone(),
        loading: state.loading,
        problem: state.problem.clone().or_else(|| state.bot_problem.clone()),
        catalog_problem: onboarding.catalog_problem.clone(),
        profile: state.draft.profile_step(state.owner.as_ref()),
        model,
        needs,
        review: OnboardingReview {
            items: state.review.iter().map(review::ReviewEntry::item).collect(),
            commit: state.commit,
        },
        default_bot_user_id: default_bot_user_id.clone(),
        default_bot_visibility,
        chat_prompts,
        tips: tips(records),
    };
    (view, default_bot_user_id)
}

fn project_default_bot_visibility(
    state: &LocalState,
    default_bot: Option<&DefaultBotPlugin>,
) -> DefaultBotVisibility {
    // 只在 Bot 身份本身待核实（静默重取在飞、上次取失败）时收起；整页刷新（loading）不掩盖
    // 最新一份已核实的身份，否则完成引导后 Bot 一直点不了，等旧 GET 的每条测试也钉不住。
    if state.bot_refreshing || state.bot_problem.is_some() {
        DefaultBotVisibility::Unknown
    } else {
        match default_bot.map(|default| (default.visibility, default.bot_user_id.as_deref())) {
            Some((Some(DefaultBotVisibility::Visible), Some(id)))
                if <&UserId>::try_from(id).is_ok() =>
            {
                DefaultBotVisibility::Visible
            }
            // 核心给了身份、但这个人不能用也是确定的“不可见”：别退成“还在核实”。
            None | Some((Some(DefaultBotVisibility::Absent), _)) => DefaultBotVisibility::Absent,
            _ => DefaultBotVisibility::Unknown,
        }
    }
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

fn navigation(state: &LocalState, step: OnboardingStep) -> Option<StepNav> {
    let not_started = state.commit == OnboardingCommit::NotStarted;
    // 第几步按还显示得出来的步算：踢掉的步不占号。
    let mut visible = STEPS
        .iter()
        .filter(|step| !state.unavailable.contains(step));
    let position = visible.position(|candidate| *candidate == step)? as u32 + 1;
    let total = u32::try_from(visible.count()).unwrap_or(u32::MAX) + position;
    let can_next = match step {
        OnboardingStep::Profile | OnboardingStep::Apps => true,
        OnboardingStep::Model => state.draft.model(state.model_settings.as_ref()).is_some(),
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
            default_bot_plugin: Some(DefaultBotPlugin {
                id: "hermes".to_owned(),
                bot_user_id: Some("@hermes:home".to_owned()),
                visibility: Some(DefaultBotVisibility::Visible),
            }),
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
            shared_contacts: Vec::new(),
            rooms: vec![RoomRow {
                unread_mentions: 0,
                layout: Default::default(),
                call_active: false,
                room_id: room_id.to_owned(),
                name: "Hermes".to_owned(),
                is_direct: true,
                direct_user_id: bot.map(str::to_owned),
                plugin: None,
                bot_chat: None,
                bot: None,
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
            own_private_bot: None,
            own_private_bot_problem: None,
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

    #[test]
    fn pending_identity_refresh_does_not_offer_or_forget_the_previous_bot() {
        let mut state = LocalState {
            onboarding: Some(onboarding()),
            bot_refreshing: true,
            ..LocalState::default()
        };
        let pending = render(&state, &storage::Records::default()).0;
        assert_eq!(
            pending.default_bot_visibility,
            Some(DefaultBotVisibility::Unknown)
        );
        assert_eq!(pending.default_bot_user_id, None);
        assert!(pending.chat_prompts.is_empty());

        state.bot_refreshing = false;
        let verified = render(&state, &storage::Records::default()).0;
        assert_eq!(
            verified.default_bot_user_id.as_deref(),
            Some("@hermes:home")
        );
        assert!(!verified.chat_prompts.is_empty());
    }

    #[test]
    fn invalid_or_contradictory_default_identity_never_becomes_a_chat_target() {
        for (visibility, id) in [
            (Some(DefaultBotVisibility::Visible), Some("not-a-matrix-id")),
            (Some(DefaultBotVisibility::Visible), None),
            (None, Some("@private:home")),
        ] {
            let mut data = onboarding();
            let default = data.default_bot_plugin.as_mut().unwrap();
            default.visibility = visibility;
            default.bot_user_id = id.map(str::to_owned);
            let state = LocalState {
                onboarding: Some(data),
                ..LocalState::default()
            };
            let view = render(&state, &storage::Records::default()).0;
            assert_eq!(
                view.default_bot_visibility,
                Some(DefaultBotVisibility::Unknown)
            );
            assert!(view.default_bot_user_id.is_none());
            assert!(view.chat_prompts.is_empty());
        }
    }

    #[test]
    fn absent_default_bot_keeps_its_identity_but_is_not_a_chat_target() {
        let mut data = onboarding();
        let default = data.default_bot_plugin.as_mut().unwrap();
        default.visibility = Some(DefaultBotVisibility::Absent);
        default.bot_user_id = Some("@private:home".into());
        let state = LocalState {
            onboarding: Some(data),
            ..LocalState::default()
        };
        let view = render(&state, &storage::Records::default()).0;
        assert_eq!(
            view.default_bot_visibility,
            Some(DefaultBotVisibility::Absent)
        );
        assert!(view.default_bot_user_id.is_none());
        assert!(view.chat_prompts.is_empty());
    }
}
