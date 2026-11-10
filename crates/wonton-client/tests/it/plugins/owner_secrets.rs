use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, CoreError, OwnerSecretFailure, OwnerSecretsView};

use super::rig::{Canned, Rig, WAIT, core_error};

const ID: &str = "app.secrets.demo";
const LIST: &str = "/_wonton/api/plugins/app.secrets.demo/secrets";
const VALUE: &str = "/_wonton/api/plugins/app.secrets.demo/secrets/admin_password";

fn act(rig: &Rig, action: Action) {
    rig.app.dispatch(action).unwrap();
}

fn list(rig: &Rig) {
    rig.core.answer(
        Method::GET,
        LIST,
        Canned(
            200,
            r#"{"secrets":[{"name":"admin_password","label":"Admin password"}]}"#.into(),
        ),
    );
    act(
        rig,
        Action::ListOwnerSecrets {
            plugin_id: ID.into(),
        },
    );
}

fn show(rig: &Rig) {
    act(
        rig,
        Action::ShowOwnerSecret {
            plugin_id: ID.into(),
            name: "admin_password".into(),
        },
    );
}

async fn settled(rig: &Rig) -> OwnerSecretsView {
    rig.wait_for(|view| {
        view.owner_secrets
            .as_ref()
            .is_some_and(|group| !group.loading && group.secrets.iter().all(|row| !row.loading))
    })
    .await
    .owner_secrets
    .unwrap()
}

async fn arrived(rig: &Rig, path: &str, count: usize) {
    let deadline = Instant::now() + WAIT;
    while rig.core.seen(&Method::GET, path).len() < count {
        assert!(Instant::now() < deadline, "missing request {path} #{count}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn disclosure_fetches_only_on_demand_and_reopening_never_reuses_values() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    assert!(rig.app.plugins().owner_secrets.is_none());
    assert!(rig.core.seen(&Method::GET, LIST).is_empty());
    assert!(rig.core.seen(&Method::GET, VALUE).is_empty());

    list(&rig);
    let group = settled(&rig).await;
    assert!(!group.secrets[0].expanded);
    assert!(group.secrets[0].value.is_none());
    assert!(rig.core.seen(&Method::GET, VALUE).is_empty());

    rig.core.answer(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"first"}"#.into()),
    );
    show(&rig);
    assert_eq!(
        settled(&rig).await.secrets[0]
            .value
            .as_ref()
            .unwrap()
            .expose(),
        "first"
    );
    act(
        &rig,
        Action::HideOwnerSecret {
            plugin_id: ID.into(),
            name: "admin_password".into(),
        },
    );
    assert!(
        rig.app.plugins().owner_secrets.unwrap().secrets[0]
            .value
            .is_none()
    );
    rig.core.answer(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"rotated"}"#.into()),
    );
    show(&rig);
    assert_eq!(
        settled(&rig).await.secrets[0]
            .value
            .as_ref()
            .unwrap()
            .expose(),
        "rotated"
    );

    act(
        &rig,
        Action::DismissOwnerSecrets {
            plugin_id: ID.into(),
        },
    );
    assert!(rig.app.plugins().owner_secrets.is_none());
    list(&rig);
    assert!(settled(&rig).await.secrets[0].value.is_none());
    assert_eq!(rig.core.seen(&Method::GET, LIST).len(), 2);
    assert_eq!(rig.core.seen(&Method::GET, VALUE).len(), 2);
    let error = rig.app.dispatch(Action::ShowOwnerSecret {
        plugin_id: ID.into(),
        name: "hidden_key".into(),
    });
    assert!(matches!(error, Err(CoreError::InvalidInput { .. })));
    assert!(
        rig.core
            .seen(
                &Method::GET,
                "/_wonton/api/plugins/app.secrets.demo/secrets/hidden_key"
            )
            .is_empty()
    );
}

#[tokio::test]
async fn a_hidden_inflight_value_cannot_reappear_or_satisfy_the_next_disclosure() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    list(&rig);
    settled(&rig).await;
    rig.core.hold(Method::GET, VALUE);
    show(&rig);
    arrived(&rig, VALUE, 1).await;
    act(
        &rig,
        Action::HideOwnerSecret {
            plugin_id: ID.into(),
            name: "admin_password".into(),
        },
    );
    let hidden = &rig.app.plugins().owner_secrets.unwrap().secrets[0];
    assert!(!hidden.expanded && !hidden.loading && hidden.value.is_none());
    rig.core.answer_always(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"fresh"}"#.into()),
    );
    show(&rig);
    arrived(&rig, VALUE, 2).await;
    rig.core.release(Method::GET, VALUE);
    let shown = settled(&rig).await;
    assert_eq!(shown.secrets[0].value.as_ref().unwrap().expose(), "fresh");
    act(&rig, Action::AppBackgrounded);
    assert!(rig.app.plugins().owner_secrets.is_none());
}

#[tokio::test]
async fn dismissal_and_relaunch_forget_pending_lists_and_displayed_values() {
    let mut rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    rig.core.hold(Method::GET, LIST);
    list(&rig);
    arrived(&rig, LIST, 1).await;
    act(
        &rig,
        Action::DismissOwnerSecrets {
            plugin_id: ID.into(),
        },
    );
    assert!(rig.app.plugins().owner_secrets.is_none());
    rig.core.release(Method::GET, LIST);
    list(&rig);
    settled(&rig).await;
    rig.core.answer(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"transient"}"#.into()),
    );
    show(&rig);
    assert_eq!(
        settled(&rig).await.secrets[0]
            .value
            .as_ref()
            .unwrap()
            .expose(),
        "transient"
    );
    rig.reopen();
    assert!(rig.app.plugins().owner_secrets.is_none());
    rig.connected().await;
    assert!(rig.app.plugins().owner_secrets.is_none());
}

#[tokio::test]
async fn another_server_cannot_inherit_an_open_secret() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    list(&rig);
    settled(&rig).await;
    rig.core.answer(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"old-server"}"#.into()),
    );
    show(&rig);
    settled(&rig).await;
    let next = super::rig::FakeHome::start(vec![rig.device()]).await;
    rig.connect_to(next.ticket());
    rig.wait_for(|view| view.owner_secrets.is_none()).await;
    assert!(next.core.seen(&Method::GET, LIST).is_empty());
    assert!(next.core.seen(&Method::GET, VALUE).is_empty());
}

#[tokio::test]
async fn errors_preserve_server_categories_and_never_retain_a_previous_value() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    list(&rig);
    settled(&rig).await;
    rig.core.answer(
        Method::GET,
        VALUE,
        Canned(200, r#"{"value":"previous"}"#.into()),
    );
    show(&rig);
    settled(&rig).await;
    for (status, kind, expected) in [
        (401, "unauthenticated", OwnerSecretFailure::Unauthenticated),
        (403, "cross_site", OwnerSecretFailure::CrossSite),
        (403, "owner_only", OwnerSecretFailure::OwnerOnly),
        (404, "not_installed", OwnerSecretFailure::NotInstalled),
        (404, "not_found", OwnerSecretFailure::NotFound),
        (503, "not_ready", OwnerSecretFailure::NotReady),
    ] {
        rig.core
            .answer(Method::GET, VALUE, core_error(status, kind, "detail"));
        show(&rig);
        let group = settled(&rig).await;
        assert!(group.secrets[0].value.is_none());
        assert_eq!(group.secrets[0].problem.as_ref().unwrap().reason, expected);
    }
    rig.core
        .answer(Method::GET, LIST, core_error(403, "owner_only", "detail"));
    act(
        &rig,
        Action::ListOwnerSecrets {
            plugin_id: ID.into(),
        },
    );
    let denied = settled(&rig).await;
    assert!(denied.secrets.is_empty());
    assert_eq!(
        denied.problem.unwrap().reason,
        OwnerSecretFailure::OwnerOnly
    );
}
