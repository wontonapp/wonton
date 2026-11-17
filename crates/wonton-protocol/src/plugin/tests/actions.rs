//! 客户端动作的 JSON 往返：每个动作都要能原样读写。

use super::*;
use crate::{PluginAction, SettingsAction};

#[test]
fn plugin_actions_round_trip_through_json() {
    for action in [
        Action::Settings {
            action: SettingsAction::SetLanguages {
                languages: vec!["zh-Hans-CN".into(), "en".into()],
            },
        },
        Action::Plugins {
            action: PluginAction::RefreshPlugins,
        },
        Action::Plugins {
            action: PluginAction::StartPlugin {
                plugin_id: "app.samba".into(),
            },
        },
        Action::Plugins {
            action: PluginAction::RefreshStore { force: true },
        },
        Action::Plugins {
            action: PluginAction::InstallPlugin {
                preview: preview(),
                secrets: [("token".to_owned(), crate::SecretString::new("s3cret"))].into(),
                settings: Default::default(),
            },
        },
        Action::Plugins {
            action: PluginAction::UpgradePlugin {
                preview: InstallPreview {
                    upgrade_from: Some("0.9.0".into()),
                    ..preview()
                },
                secrets: [("region".to_owned(), crate::SecretString::new("eu-west"))].into(),
                settings: Default::default(),
            },
        },
        Action::Plugins {
            action: PluginAction::StartTakeover {
                id: "app.memos".into(),
            },
        },
        Action::Plugins {
            action: PluginAction::EndTakeover {
                id: "app.memos".into(),
            },
        },
        Action::Plugins {
            action: PluginAction::UninstallPlugin {
                id: "app.samba".into(),
                delete_data: true,
            },
        },
        Action::Plugins {
            action: PluginAction::DismissPluginTask {
                id: "app.samba".into(),
            },
        },
    ] {
        assert_eq!(round_trip(&action), action);
    }
}
