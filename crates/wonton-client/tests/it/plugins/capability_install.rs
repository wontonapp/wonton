//! 缺必需能力的安装（ADR-0053）：确认页写明会一起装的提供方，安装请求把它们的完整决定带上。

use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, PluginTaskState};
use wonton_protocol::{
    CapabilityProvider, InstallPreview, MissingCapability, PluginAction, PluginFailure,
};

use super::rig::{Canned, Rig, WAIT, preview};

const PROVIDER_SOURCE: &str = "github:owner/app.downloader@v2.0.0";

fn consumer_preview() -> InstallPreview {
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![MissingCapability {
        capability: "torrent_client".to_owned(),
        third_party: false,
        providers: vec![CapabilityProvider {
            id: "app.downloader".to_owned(),
            title: "下载器".to_owned(),
            source: PROVIDER_SOURCE.to_owned(),
            high_privilege: true,
        }],
    }];
    consumer
}

fn provider_preview() -> InstallPreview {
    let mut provider = preview("app.downloader");
    provider.source = Some(PROVIDER_SOURCE.to_owned());
    provider.permissions[0].permission = "storage.share.downloads:write".to_owned();
    provider
}

fn answer_previews(rig: &Rig, consumer: &InstallPreview, provider: &InstallPreview) {
    for body in [consumer, provider] {
        rig.core.answer(
            Method::POST,
            "/_wonton/api/plugins/preview",
            Canned(200, serde_json::to_string(body).unwrap()),
        );
    }
}

fn install(rig: &Rig, preview: InstallPreview) {
    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::InstallPlugin {
                preview,
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
}

#[tokio::test]
async fn the_confirm_page_names_the_provider_that_will_be_installed_with_it() {
    let rig = Rig::new().await;
    rig.connected().await;
    answer_previews(&rig, &consumer_preview(), &provider_preview());

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    let line = shown
        .notices
        .iter()
        .find(|line| line.text.contains("下载器"))
        .expect("确认页要写明会一起装提供方");
    assert!(line.emphasis, "高权限提供方要醒目");
    assert!(line.text.contains("torrent_client"), "{line:?}");
}

#[tokio::test]
async fn install_carries_the_provider_decision_of_the_listed_source() {
    let rig = Rig::new().await;
    rig.connected().await;
    answer_previews(&rig, &consumer_preview(), &provider_preview());
    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();

    install(&rig, shown);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["source"], "github:owner/app.photos@v1.0.0");
    assert_eq!(
        sent["install_together"],
        serde_json::json!([{
            "id": "app.downloader",
            "request": {
                "source": PROVIDER_SOURCE,
                "digest": "sha256:abc",
                "plan": "opaque-install-plan",
                "grant": ["storage.share.downloads:write"],
                "enabled_permissions": [],
                "secrets": {},
                "settings": {},
                "devices": [],
                "lan_webui": false,
                "install_together": [],
                "connect_plugins": [],
            },
        }])
    );
}

#[tokio::test]
async fn a_required_capability_with_no_provider_stops_before_the_request() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![MissingCapability {
        capability: "torrent_client".to_owned(),
        third_party: false,
        providers: Vec::new(),
    }];
    answer_previews(&rig, &consumer, &provider_preview());

    install(&rig, consumer);
    let state = rig
        .task_within("app.photos", Duration::from_secs(WAIT.as_secs()), |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert!(
        matches!(
            state,
            PluginTaskState::Failed {
                reason: PluginFailure::RecordUnusable,
                ref message,
            } if message.contains("torrent_client")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}
