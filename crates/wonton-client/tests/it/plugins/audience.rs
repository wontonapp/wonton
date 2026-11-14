//! 原生壳只把服务器给的角色选择交回客户端核心。

use axum::http::Method;
use wonton_client::{
    Action, Gate, PluginAudienceChoice, PluginAudienceNotice, PluginAudienceView,
    PluginInstallAction, PluginTaskState, plugin_audience_notice, preview_install_action,
    selected_plugin_audience, store_install_action,
};

use super::rig::{Rig, item, preview};

fn audience() -> PluginAudienceView {
    PluginAudienceView {
        choices: vec![
            PluginAudienceChoice {
                id: "all".into(),
                name: "All apps".into(),
                template: None,
                selected: false,
                required: true,
            },
            PluginAudienceChoice {
                id: "family".into(),
                name: "Family".into(),
                template: None,
                selected: true,
                required: false,
            },
            PluginAudienceChoice {
                id: "friends".into(),
                name: "Friends".into(),
                template: None,
                selected: false,
                required: false,
            },
        ],
        notice: Some(PluginAudienceNotice::SharedData),
    }
}

#[tokio::test]
async fn install_serializes_chosen_roles_and_preserves_required_roles() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut shown = preview("app.photos");
    shown.audience = Some(audience());
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: shown,
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    assert_eq!(
        rig.task("app.photos", |state| *state != PluginTaskState::Running)
            .await,
        PluginTaskState::Succeeded
    );
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(
        sent["audience"]["role_ids"],
        serde_json::json!(["all", "family"])
    );
}

#[test]
fn admin_request_is_available_without_installable_but_real_blocks_win() {
    let mut store = item("app.photos", "Photos", &[]);
    store.installable = false;
    store.install_gate = Gate::AskAdmin;
    let mut detail = super::detail::detail();
    detail.installed = None;
    detail.installable = false;
    detail.install_gate = Gate::AskAdmin;
    assert_eq!(
        wonton_client::detail_install_action(detail.clone()),
        PluginInstallAction::RequestAdmin
    );
    detail.blocked = Some("unsupported architecture".into());
    assert_eq!(
        wonton_client::detail_install_action(detail),
        PluginInstallAction::Blocked
    );
    assert_eq!(
        store_install_action(store.clone()),
        PluginInstallAction::RequestAdmin
    );
    store.blocked = Some("unsupported architecture".into());
    assert_eq!(store_install_action(store), PluginInstallAction::Blocked);
    let mut shown = preview("app.photos");
    shown.install_gate = Gate::AskAdmin;
    assert_eq!(
        preview_install_action(shown.clone()),
        PluginInstallAction::RequestAdmin
    );
    shown.install_gate = Gate::Other;
    assert_eq!(preview_install_action(shown), PluginInstallAction::Blocked);
}

#[test]
fn no_editable_audience_does_not_send_owner_grants_and_still_warns() {
    let hidden = PluginAudienceView {
        choices: Vec::new(),
        notice: Some(PluginAudienceNotice::SharedData),
    };
    assert!(selected_plugin_audience(hidden.clone()).is_none());
    assert_eq!(
        plugin_audience_notice(hidden),
        Some(PluginAudienceNotice::SharedData)
    );
    let mut unselected = audience();
    for choice in &mut unselected.choices {
        choice.required = false;
        choice.selected = false;
    }
    assert_eq!(plugin_audience_notice(unselected), None);
}

#[tokio::test]
async fn audience_api_updates_then_returns_refreshed_detail() {
    let rig = Rig::new().await;
    rig.connected().await;
    let selected = selected_plugin_audience(audience()).unwrap();
    let path = "/_wonton/api/plugins/app.photos/audience";
    let response = super::rig::Canned(200, serde_json::to_string(&selected).unwrap());
    rig.core.answer(Method::GET, path, response.clone());
    assert_eq!(
        rig.app.plugin_audience("app.photos".into()).await.unwrap(),
        selected
    );
    rig.core.answer(Method::PUT, path, response);
    let mut detail = super::detail::detail();
    detail.audience = Some(audience());
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins/app.photos",
        super::rig::Canned(200, serde_json::to_string(&detail).unwrap()),
    );
    let updated = rig
        .app
        .update_plugin_audience("app.photos".into(), selected.clone())
        .await
        .unwrap();
    assert_eq!(updated.audience, detail.audience);
    let sent: wonton_client::PluginAudience =
        serde_json::from_str(&rig.core.seen(&Method::PUT, path)[0].body).unwrap();
    assert_eq!(sent, selected);
}
