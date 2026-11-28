use serde::{Deserialize, Serialize};

use crate::action_domains::{
    BotAction, BotContactAction, ChatAction, ClientsAction, FamilyAction, FileAction,
    FriendsAction, HomeAction, OnboardingAction, PageAction, PeerAction, PluginAction, PushAction,
    SessionAction, SettingsAction, SharingAction,
};
use crate::agent_chain::AgentAction;
use crate::agent_models_view::AgentModelsAction;

/// 原生壳发给客户端核心的唯一用户操作入口。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "domain", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "passed by value across UniFFI; boxing would allocate and change the exposed contract"
)]
pub enum Action {
    Session {
        action: SessionAction,
    },
    Home {
        action: HomeAction,
    },
    Chat {
        action: ChatAction,
    },
    Bot {
        action: BotAction,
    },
    Push {
        action: PushAction,
    },
    Plugins {
        action: PluginAction,
    },
    Settings {
        action: SettingsAction,
    },
    Onboarding {
        action: OnboardingAction,
    },
    Files {
        action: FileAction,
    },
    Pages {
        action: PageAction,
    },
    Peers {
        action: PeerAction,
    },
    BotContacts {
        action: BotContactAction,
    },
    Family {
        action: FamilyAction,
    },
    Friends {
        action: FriendsAction,
    },
    Sharing {
        action: SharingAction,
    },
    /// 公网访问、入口和访客链接（ADR-0021、ADR-0089、ADR-0090）。
    Clients {
        action: ClientsAction,
    },
    Timeline {
        action: crate::TimelineAction,
    },
    Agent {
        action: AgentAction,
    },
    /// 一个 Bot 的模型页（ADR-0129）。
    AgentModels {
        action: AgentModelsAction,
    },
    /// 媒体与传输（ADR-0113）：播放控制、停止、队列改动。
    Media {
        action: crate::media::MediaAction,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn action_json_uses_domain_and_nested_action_tags() {
        let actions = [
            Action::Session {
                action: SessionAction::SelectSession {
                    session_id: "session-1".into(),
                },
            },
            Action::Settings {
                action: SettingsAction::SetLanguages {
                    languages: vec!["zh-Hans".into(), "en".into()],
                },
            },
            Action::Plugins {
                action: PluginAction::RefreshPlugins,
            },
            Action::Family {
                action: FamilyAction::RefreshRoles,
            },
        ];

        assert_eq!(
            serde_json::to_value(&actions[0]).unwrap(),
            json!({
                "domain": "session",
                "action": {"type": "select_session", "session_id": "session-1"}
            })
        );
        assert_eq!(
            serde_json::to_value(&actions[1]).unwrap(),
            json!({
                "domain": "settings",
                "action": {"type": "set_languages", "languages": ["zh-Hans", "en"]}
            })
        );
        assert_eq!(
            serde_json::to_value(&actions[3]).unwrap(),
            json!({"domain": "family", "action": {"type": "refresh_roles"}})
        );
        assert!(
            serde_json::from_value::<Action>(json!({
                "type": "select_session", "session_id": "session-1"
            }))
            .is_err()
        );

        for action in actions {
            let json = serde_json::to_value(&action).unwrap();
            assert_eq!(serde_json::from_value::<Action>(json).unwrap(), action);
        }
    }
}
