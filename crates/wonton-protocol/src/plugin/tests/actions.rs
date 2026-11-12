//! 客户端动作的 JSON 往返：每个动作都要能原样读写。

use super::*;

#[test]
fn plugin_actions_round_trip_through_json() {
    for action in [
        Action::SetLanguages {
            languages: vec!["zh-Hans-CN".into(), "en".into()],
        },
        Action::RefreshPlugins,
        Action::StartPlugin {
            plugin_id: "app.samba".into(),
        },
        Action::RefreshStore { force: true },
        Action::InstallPlugin {
            preview: preview(),
            secrets: [("token".to_owned(), crate::SecretString::new("s3cret"))].into(),
        },
        Action::UpgradePlugin {
            preview: InstallPreview {
                upgrade_from: Some("0.9.0".into()),
                ..preview()
            },
            secrets: Default::default(),
        },
        Action::StartTakeover {
            id: "app.memos".into(),
        },
        Action::EndTakeover {
            id: "app.memos".into(),
        },
        Action::UninstallPlugin {
            id: "app.samba".into(),
            delete_data: true,
        },
        Action::DismissPluginTask {
            id: "app.samba".into(),
        },
    ] {
        assert_eq!(round_trip(&action), action);
    }
}
