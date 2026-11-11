use super::*;

#[tokio::test]
async fn real_metrics_response_drives_traffic_and_connection_quota_without_double_count() {
    let fixture = Fixture::new().await;
    let response = fixture.register(&key(1), &fixture.invite().await).await;
    let home = response["home"].as_str().unwrap();
    let token = response["frp_token"].as_str().unwrap();
    *fixture.backend.metrics.lock().await = format!(
        "frp_server_client_counts 1\nfrp_server_traffic_in{{name=\"{home}.matrix\",type=\"https\"}} 600\nfrp_server_traffic_out{{name=\"{home}.matrix\",type=\"https\"}} 400\nfrp_server_connection_counts{{name=\"{home}.matrix\",type=\"https\"}} 100\n"
    );
    fixture.app.poll_metrics().await.unwrap();
    fixture.app.poll_metrics().await.unwrap();
    assert_eq!(
        fixture.app.store.lock().await.data.homes[home].usage_bytes,
        1000
    );
    assert_eq!(
        fixture.hook("NewUserConn", home, token, json!({})).await["reject"],
        true
    );
    *fixture.backend.metrics.lock().await = format!(
        "frp_server_client_counts 1\nfrp_server_traffic_in{{name=\"{home}.matrix\",type=\"https\"}} 5\nfrp_server_traffic_out{{name=\"{home}.matrix\",type=\"https\"}} 7\n"
    );
    fixture.app.poll_metrics().await.unwrap();
    assert_eq!(
        fixture.app.store.lock().await.data.homes[home].usage_bytes,
        1012
    );
    assert_eq!(
        fixture.hook("NewUserConn", home, token, json!({})).await["reject"],
        false
    );
    fixture.app.runtime.lock().await.metrics_at = now() - 66;
    assert_eq!(
        fixture.hook("NewUserConn", home, token, json!({})).await["reject"],
        true
    );
}

#[tokio::test]
async fn acme_proxy_exposes_only_update_and_revokes_suspended_credentials() {
    let fixture = Fixture::new().await;
    let response = fixture.register(&key(1), &fixture.invite().await).await;
    let home = response["home"].as_str().unwrap();
    let account = &response["acme_dns"];
    let update = || {
        fixture
            .app
            .client
            .post(format!("{}/acme/update", fixture.url))
            .header("X-Api-User", account["username"].as_str().unwrap())
            .header("X-Api-Key", account["password"].as_str().unwrap())
            .json(&json!({"subdomain":account["subdomain"],"txt":"test-proof"}))
    };
    assert_eq!(update().send().await.unwrap().status(), 200);
    assert_eq!(
        fixture
            .app
            .client
            .post(format!("{}/acme/register", fixture.url))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    admin::execute(
        &fixture.app,
        Command::Suspend {
            home: home.into(),
            reason: "abuse".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(update().send().await.unwrap().status(), 403);
}
