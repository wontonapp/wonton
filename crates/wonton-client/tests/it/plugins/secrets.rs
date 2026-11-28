//! 确认页上填的密钥：随安装请求发出，改一项时走 PUT，值只上行。

use axum::http::Method;
use wonton_client::{Action, PluginTaskState};
use wonton_protocol::{SecretInput, SecretString};

use super::rig::{Canned, Rig, preview};

#[tokio::test]
async fn the_secret_values_the_owner_filled_are_posted_and_updated() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut shown = preview("app.photos");
    shown.secret_inputs = vec![SecretInput {
        name: "token".to_owned(),
        title: "Token".to_owned(),
        description: None,
        url: None,
        required: true,
    }];
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: shown,
                secrets: [("token".to_owned(), SecretString::new("s3cret"))].into(),
                settings: Default::default(),
            },
        })
        .unwrap();
    rig.task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    let sent: serde_json::Value = serde_json::from_str(
        rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0]
            .body
            .as_str(),
    )
    .unwrap();
    assert_eq!(sent["secrets"]["token"], "s3cret");

    let path = "/_wonton/api/plugins/app.photos/secrets/token";
    rig.core.answer(
        Method::PUT,
        path,
        Canned(
            200,
            serde_json::json!({
                "source": null, "id": "app.photos", "title": "x", "summary": "",
                "about": null, "icon": null, "screenshots": [], "tags": [],
                "kind": "app", "version": "1.0.0", "trust": "official",
                "provider": null, "via_index": null, "author": null,
                "homepage": null, "repository": null, "license": null, "stars": null,
                "release_notes": null, "permissions": [], "requirements": [],
                "has_web": false, "installed": null, "blocked": null,
                "lan_webui": null, "secret_inputs": [],
            })
            .to_string(),
        ),
    );
    rig.app
        .set_plugin_secret(
            "app.photos".to_owned(),
            "token".to_owned(),
            Some(SecretString::new("next")),
        )
        .await
        .unwrap();
    let put: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::PUT, path)[0].body).unwrap();
    assert_eq!(put["value"], "next");
}
