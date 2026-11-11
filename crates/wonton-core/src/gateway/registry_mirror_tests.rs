use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::tests::{CORE, Harness, TOKEN};
use crate::plugin::storefront::StoreConfig;

const PATH: &str = "/_wonton/api/registry-mirror";

async fn harness() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.json"),
        r#"{"format":1,"registry_mirrors":{"docker.io":["mirror.example:5443"],"ghcr.io":["ghcr.example"]}}"#,
    )
    .unwrap();
    let harness = Harness::start_with_store(
        None,
        StoreConfig {
            index_dir: Some(dir.path().to_owned()),
            community: false,
        },
    )
    .await;
    (harness, dir)
}

#[tokio::test]
async fn registry_api_settings_persist_and_can_be_disabled() {
    let (h, _index) = harness().await;
    let auth = format!("Bearer {TOKEN}");
    let headers = [("authorization", auth.as_str())];
    let saved = h
        .send(
            Method::PUT,
            CORE,
            PATH,
            &headers,
            Body::from(json!({"mirror":"custom.example:5000"}).to_string()),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(saved.json()["mirror"], "custom.example:5000");
    assert_eq!(saved.json()["choices"], json!(["mirror.example:5443"]));
    let persisted = h.get_after_restart(CORE, PATH, &headers).await;
    assert_eq!(persisted.json()["mirror"], "custom.example:5000");
    let off = h
        .send(
            Method::PUT,
            CORE,
            PATH,
            &headers,
            Body::from(r#"{"mirror":null}"#),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK);
    assert_eq!(
        h.get(CORE, PATH, &headers).await.json()["mirror"],
        Value::Null
    );
}

#[tokio::test]
async fn registry_api_requires_authentication_and_rejects_non_authorities() {
    let (h, _index) = harness().await;
    for (method, path, body) in [
        (Method::GET, PATH.to_owned(), "{}"),
        (
            Method::PUT,
            PATH.to_owned(),
            r#"{"mirror":"mirror.example"}"#,
        ),
        (Method::POST, format!("{PATH}/test"), r#"{"mirrors":[]}"#),
    ] {
        let reply = h.send(method, CORE, &path, &[], Body::from(body)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let auth = format!("Bearer {TOKEN}");
    let headers = [("authorization", auth.as_str())];
    for address in [
        "",
        "https://mirror.example",
        "mirror.example/path",
        "u:p@mirror.example",
        "mirror.example:70000",
        "mirror.example?x",
        "mirror.example#x",
    ] {
        let reply = h
            .send(
                Method::PUT,
                CORE,
                PATH,
                &headers,
                Body::from(json!({"mirror":address}).to_string()),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{address}: {}",
            reply.text()
        );
        assert_eq!(reply.json()["error"]["kind"], "invalid_input");
        let reply = h
            .send(
                Method::POST,
                CORE,
                &format!("{PATH}/test"),
                &headers,
                Body::from(json!({"mirrors":[address]}).to_string()),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        h.get(CORE, PATH, &headers).await.json()["mirror"],
        Value::Null
    );
}

#[tokio::test]
async fn registry_api_daemon_proxy_skips_hub_and_clears_recommendation() {
    let (h, _index) = harness().await;
    h.docker.set_https_proxy("http://proxy.example:7899");
    let auth = format!("Bearer {TOKEN}");
    let headers = [("authorization", auth.as_str())];
    let result = h
        .send(
            Method::POST,
            CORE,
            &format!("{PATH}/test"),
            &headers,
            Body::from(r#"{"mirrors":[]}"#),
        )
        .await;
    assert_eq!(result.status, StatusCode::OK, "{}", result.text());
    assert_eq!(result.json()["docker_hub"], Value::Null);
    let settings = h.get(CORE, PATH, &headers).await.json();
    assert_eq!(settings["docker_hub"], Value::Null);
    assert_eq!(settings["recommended"], false);
}

#[tokio::test]
async fn registry_api_paired_device_reaches_validation_without_owner_token() {
    use super::tunnel_tests::{Home, Phone};
    use http_body_util::BodyExt;
    let home = Home::start(None).await;
    let key = iroh::SecretKey::generate();
    let body = json!({"endpoint_id":key.public().to_string()}).to_string();
    assert_eq!(
        home.manage(Method::POST, "/_wonton/api/devices", &body)
            .await,
        StatusCode::CREATED
    );
    let phone = Phone::start(key, &home).await;
    // 非法地址在读取官方索引前失败，证明隧道身份获准而未进入外网请求。
    let request = phone
        .request(Method::PUT, CORE, PATH)
        .body(Body::from(r#"{"mirror":"https://invalid.example"}"#))
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["kind"], "invalid_input");
}
