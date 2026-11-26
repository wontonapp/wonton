use super::*;

#[tokio::test]
async fn system_identity_is_limited_to_operator_allowlist() {
    let token = "system-secret";
    let fixture = Fixture::with_system(Some(System {
        domains: vec!["push.homes.test".into(), "site.example".into()],
        token_hash: crate::store::hash(token.as_bytes()),
    }))
    .await;
    assert_eq!(
        fixture.hook("Login", "system", token, json!({})).await["reject"],
        false
    );
    let https = json!({"proxy_type":"https","proxy_name":"system.push","custom_domains":["push.homes.test"]});
    let admitted = fixture.hook("NewProxy", "system", token, https).await;
    assert_eq!(admitted["reject"], false);
    assert_eq!(admitted["content"]["bandwidth_limit"], "20MB");
    let external = json!({"proxy_type":"http","proxy_name":"system.omarchy","custom_domains":["site.example"]});
    assert_eq!(
        fixture.hook("NewProxy", "system", token, external).await["reject"],
        false
    );
    let multi = json!({"proxy_type":"https","proxy_name":"system.multi","custom_domains":["push.homes.test","site.example"]});
    assert_eq!(
        fixture.hook("NewProxy", "system", token, multi).await["reject"],
        false
    );
    let unknown =
        json!({"proxy_type":"https","proxy_name":"system.bad","custom_domains":["bad.homes.test"]});
    assert_eq!(
        fixture.hook("NewProxy", "system", token, unknown).await["reject"],
        true
    );
    assert_eq!(
        fixture.hook("Login", "system", "wrong", json!({})).await["reject"],
        true
    );
}

#[tokio::test]
async fn system_identity_is_rejected_when_not_configured() {
    let fixture = Fixture::new().await;
    let rejected = fixture.hook("Login", "system", "any", json!({})).await;
    assert_eq!(rejected["reject"], true);
    assert_eq!(rejected["reject_reason"], "system access not configured");
}

#[tokio::test]
async fn empty_custom_domains_are_ignored_for_system_and_home() {
    let system_fixture = Fixture::with_system(Some(System {
        domains: vec!["push.homes.test".into()],
        token_hash: crate::store::hash(b"sys"),
    }))
    .await;
    let system = "sys";
    let accepted = system_fixture
        .hook(
            "NewProxy",
            "system",
            system,
            json!({"proxy_type":"https","proxy_name":"system.empty","custom_domains":["push.homes.test", ""]}),
        )
        .await;
    assert_eq!(accepted["reject"], false);
    let all_empty = system_fixture
        .hook(
            "NewProxy",
            "system",
            system,
            json!({"proxy_type":"https","proxy_name":"system.blank","custom_domains":[""]}),
        )
        .await;
    assert_eq!(all_empty["reject"], true);

    let home_fixture = Fixture::new().await;
    let response = home_fixture
        .register(&key(1), &home_fixture.invite().await)
        .await;
    let home = response["home"].as_str().unwrap();
    let token = response["frp_token"].as_str().unwrap();
    let accepted = home_fixture
        .hook(
            "NewProxy",
            home,
            token,
            json!({"proxy_type":"https","proxy_name":format!("{home}.empty"),"custom_domains":[format!("{home}.homes.test"), ""]}),
        )
        .await;
    assert_eq!(accepted["reject"], false);
    let all_empty = home_fixture
        .hook(
            "NewProxy",
            home,
            token,
            json!({"proxy_type":"https","proxy_name":format!("{home}.blank"),"custom_domains":[""]}),
        )
        .await;
    assert_eq!(all_empty["reject"], true);
}

#[tokio::test]
async fn stale_relay_metrics_do_not_take_system_vhosts_down() {
    let fixture = Fixture::with_system(Some(System {
        domains: vec!["push.homes.test".into()],
        token_hash: crate::store::hash(b"sys"),
    }))
    .await;
    fixture.app.runtime.lock().await.metrics_at = now().saturating_sub(120);
    let ping = fixture.hook("Ping", "system", "sys", json!({})).await;
    assert_eq!(ping["reject"], false);
    let proxy = json!({"proxy_type":"https","proxy_name":"system.push","custom_domains":["push.homes.test"]});
    assert_eq!(
        fixture.hook("NewProxy", "system", "sys", proxy).await["reject"],
        false
    );

    let home_fixture = Fixture::new().await;
    let response = home_fixture
        .register(&key(1), &home_fixture.invite().await)
        .await;
    let home = response["home"].as_str().unwrap();
    let token = response["frp_token"].as_str().unwrap();
    home_fixture.app.runtime.lock().await.metrics_at = now().saturating_sub(120);
    let proxy = json!({"proxy_type":"https","proxy_name":format!("{home}.matrix"),"custom_domains":[format!("{home}.homes.test")]});
    assert_eq!(
        home_fixture.hook("NewProxy", home, token, proxy).await["reject"],
        true
    );
}
