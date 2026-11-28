//! OIDC 登录与同意页只认精确的 Bot 客户端。

use super::*;

#[tokio::test]
async fn oidc_consent_uses_exact_bot_client_and_rechecks_bot_existence() {
    let accepted = Arc::new(Mutex::new(Vec::<Value>::new()));
    let h = Harness::start(Some(fake_hydra(accepted.clone()).await)).await;
    setup(&h).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let label = bot_web_label(&bot("mia"));
    let result = h
        .get(
            CORE,
            &format!("/_wonton/oidc/consent?consent_challenge={label}"),
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(result.status, StatusCode::SEE_OTHER, "{}", result.text());
    assert_eq!(
        accepted.lock().unwrap()[0]["session"]["id_token"]["groups"],
        json!(["user", "plugin_admin", "admin"])
    );
    h.manager
        .bot_records_for_test(&PluginId::parse("app.whoami").unwrap(), &[]);
    let result = h
        .get(
            CORE,
            &format!("/_wonton/oidc/consent?consent_challenge={label}"),
            &[("cookie", &core)],
        )
        .await;
    assert_failure(&result, StatusCode::NOT_FOUND, "not_found");
    assert_eq!(accepted.lock().unwrap().len(), 1);
}

async fn bot_hydra(accepted: Arc<Mutex<Vec<Value>>>) -> std::net::SocketAddr {
    use axum::extract::Query;
    use axum::routing::{get, put};
    use axum::{Json, Router};
    use std::collections::HashMap;
    let login = get(|Query(query): Query<HashMap<String, String>>| async move {
        Json(
            json!({"skip": false, "subject": "", "client": {"client_id": query["login_challenge"]}}),
        )
    });
    let accept = put(move |Json(body): Json<Value>| {
        let accepted = accepted.clone();
        async move {
            accepted.lock().unwrap().push(body);
            Json(json!({"redirect_to": "http://auth.wonton.localhost/after-login"}))
        }
    });
    let (listener, addr) = super::super::fakes::bind().await;
    super::super::fakes::serve(
        listener,
        Router::new()
            .route("/admin/oauth2/auth/requests/login", login)
            .route("/admin/oauth2/auth/requests/login/accept", accept),
    )
    .await;
    addr
}

#[tokio::test]
async fn oidc_login_checks_exact_client_family_and_target_before_accepting() {
    let accepted = Arc::new(Mutex::new(Vec::<Value>::new()));
    let h = Harness::start(Some(bot_hydra(accepted.clone()).await)).await;
    setup(&h).await;
    let device = member(&h);
    let label = bot_web_label(&bot("mia"));
    let path = format!("/_wonton/oidc/login?login_challenge={label}");
    let denied = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_failure(&denied, StatusCode::FORBIDDEN, "not_shared");
    assert!(accepted.lock().unwrap().is_empty());
    open_whoami(&h);
    let allowed = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_eq!(allowed.status, StatusCode::SEE_OTHER, "{}", allowed.text());
    assert_eq!(accepted.lock().unwrap()[0]["subject"], "mia");
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    h.manager
        .bot_records_for_test(&PluginId::parse("app.whoami").unwrap(), &[]);
    for client in [label, "app.whoami".into(), bot_web_label(&bot("unknown"))] {
        let reply = h
            .get(
                CORE,
                &format!("/_wonton/oidc/login?login_challenge={client}"),
                &[("cookie", &core)],
            )
            .await;
        assert_failure(&reply, StatusCode::NOT_FOUND, "not_found");
    }
    assert_eq!(accepted.lock().unwrap().len(), 1);
}
