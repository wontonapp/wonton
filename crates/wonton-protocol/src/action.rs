use serde::{Deserialize, Serialize};

use crate::action_domains::{
    BotAction, BotContactAction, ChatAction, FamilyAction, FileAction, FriendsAction, HomeAction,
    OnboardingAction, PageAction, PeerAction, PluginAction, PushAction, SessionAction,
    SettingsAction, SharingAction,
};
use crate::agent_chain::AgentAction;

/// 原生壳发给客户端核心的唯一用户操作入口。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "domain", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "passed by value across UniFFI; boxing would allocate and change the exposed contract"
)]
pub enum Action {
    Session { action: SessionAction },
    Home { action: HomeAction },
    Chat { action: ChatAction },
    Bot { action: BotAction },
    Push { action: PushAction },
    Plugins { action: PluginAction },
    Settings { action: SettingsAction },
    Onboarding { action: OnboardingAction },
    Files { action: FileAction },
    Pages { action: PageAction },
    Peers { action: PeerAction },
    BotContacts { action: BotContactAction },
    Family { action: FamilyAction },
    Friends { action: FriendsAction },
    Sharing { action: SharingAction },
    Timeline { action: crate::TimelineAction },
    Agent { action: AgentAction },
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
