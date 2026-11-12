use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use super::*;
use crate::Action;

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        serde_json::from_value::<T>(serde_json::to_value(value).unwrap()).unwrap(),
        *value
    );
}

fn problem(reason: ShareFailure) -> ShareProblem {
    ShareProblem {
        reason,
        http_status: Some(403),
        identity: None,
    }
}

fn plugin() -> SharedPlugin {
    SharedPlugin {
        plugin_id: "app.snake".into(),
        title: "Snake".into(),
        web_url: "http://app-snake.home.example.org/".into(),
        display: TabDisplay::Standard,
        state: TabState::Ready,
    }
}

#[test]
fn grants_preserve_person_group_and_optional_expiry() {
    for recipient in [
        ShareRecipient::Friend {
            user_id: "@bob:matrix.org".into(),
        },
        ShareRecipient::Group {
            group_id: "players".into(),
        },
    ] {
        for expires_at_ms in [None, Some(1_793_000_000_000)] {
            let grant = ShareGrant {
                id: "grant-1".into(),
                plugin_id: "app.snake".into(),
                recipient: recipient.clone(),
                expires_at_ms,
            };
            let shares = PluginShares {
                per_user_accounts: false,
                friend_count: 2,
                grants: vec![grant],
            };
            round_trip(&shares);
            let update = ShareGrantUpdate {
                plugin_id: "app.snake".into(),
                recipient: recipient.clone(),
                expires_at_ms,
                acknowledge_risk: true,
            };
            round_trip(&update);
            round_trip(&Action::CreatePluginShare {
                grant: update.clone(),
            });
            round_trip(&Action::UpdatePluginShare {
                id: "grant-1".into(),
                grant: update,
            });
        }
    }
    round_trip(&Action::RevokePluginShare {
        id: "grant-1".into(),
    });
}

#[test]
fn sources_and_app_failures_remain_distinct() {
    let view = SharedAppsViewModel {
        sources: vec![
            SharedAppsSource {
                owner_user_id: "@alice:example.org".into(),
                owner_display_name: Some("Alice".into()),
                server_name: "example.org".into(),
                state: SharedAppsState::Ready {
                    proxy: crate::ProxySettings {
                        host: "127.0.0.1".into(),
                        port: 19001,
                        username: "share".into(),
                        password: "connection-secret".into(),
                        domain: "home.example.org".into(),
                    },
                },
                apps: vec![SharedAppRow {
                    plugin: plugin(),
                    problem: Some(problem(ShareFailure::Revoked)),
                }],
            },
            SharedAppsSource {
                owner_user_id: "@carol:other.example.org".into(),
                owner_display_name: None,
                server_name: "other.example.org".into(),
                state: SharedAppsState::Failed {
                    problem: problem(ShareFailure::Unreachable),
                },
                apps: vec![],
            },
        ],
    };
    round_trip(&view);
    round_trip(&SharedAppsState::Loading);
    for reason in [
        ShareFailure::Unreachable,
        ShareFailure::Required,
        ShareFailure::Revoked,
        ShareFailure::Expired,
        ShareFailure::PluginNotRunning,
        ShareFailure::Other,
    ] {
        round_trip(&problem(reason));
    }
    for identity in [
        ShareIdentityFailure::InvalidToken,
        ShareIdentityFailure::DiscoveryFailed,
        ShareIdentityFailure::UserInfoFailed,
        ShareIdentityFailure::ServerNameMismatch,
    ] {
        round_trip(&ShareProblem {
            reason: ShareFailure::Identity,
            http_status: Some(401),
            identity: Some(identity),
        });
    }
    round_trip(&Action::RefreshSharedApps {
        owner_user_id: None,
    });
    round_trip(&Action::RefreshSharedApps {
        owner_user_id: Some("@alice:example.org".into()),
    });
    round_trip(&Action::OpenSharedApp {
        owner_user_id: "@alice:example.org".into(),
        plugin_id: "app.snake".into(),
    });
}

#[test]
fn a_shared_page_requires_its_source_and_a_ready_source_requires_its_proxy() {
    let page: crate::TabRef = serde_json::from_value(json!({
        "type": "shared_app",
        "owner_user_id": "@alice:example.org",
        "plugin_id": "app.snake"
    }))
    .unwrap();
    assert_eq!(
        page,
        crate::TabRef::SharedApp {
            owner_user_id: "@alice:example.org".into(),
            plugin_id: "app.snake".into(),
        }
    );
    assert!(
        serde_json::from_value::<crate::TabRef>(json!({
            "type": "shared_app", "plugin_id": "app.snake"
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<SharedAppsState>(json!({"state": "ready"})).is_err());
    let source: SharedAppsState = serde_json::from_value(json!({
        "state": "ready",
        "proxy": {
            "host": "127.0.0.1", "port": 19001,
            "username": "share", "password": "connection-secret",
            "domain": "home.example.org"
        }
    }))
    .unwrap();
    assert!(!format!("{source:?}").contains("connection-secret"));
}

#[test]
fn unknown_share_values_do_not_break_lists() {
    assert_eq!(
        serde_json::from_value::<ShareRecipient>(json!({"type":"organization", "id":"ignored"}))
            .unwrap(),
        ShareRecipient::Other
    );
    assert_eq!(
        serde_json::from_value::<ShareFailure>(json!("disabled")).unwrap(),
        ShareFailure::Other
    );
    assert_eq!(
        serde_json::from_value::<ShareIdentityFailure>(json!("issuer_offline")).unwrap(),
        ShareIdentityFailure::Other
    );
    assert_eq!(
        serde_json::from_value::<SharedAppsState>(json!({"state":"reconnecting"})).unwrap(),
        SharedAppsState::Other
    );
    assert_eq!(
        serde_json::from_value::<AppsAction>(json!("new_button")).unwrap(),
        AppsAction::Other
    );
    for action in [AppsAction::Store, AppsAction::AddSource, AppsAction::Other] {
        round_trip(&action);
    }
    let mut app = serde_json::to_value(plugin()).unwrap();
    app["state"] = json!({"state":"maintenance", "estimate_ms": 1000});
    let app: SharedPlugin = serde_json::from_value(app).unwrap();
    assert_eq!(app.state, TabState::Unknown);
}
