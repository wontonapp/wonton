use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness, TOKEN};
use crate::plugin::names::container_name;

#[tokio::test]
async fn failed_install_remains_queryable_until_explicitly_cleared() {
    let home = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let headers = [("authorization", bearer.as_str())];
    let package = "services:\n  app:\n    image: nginx\nx-wonton:\n  spec: '0.5'\n  id: app.rollback\n  version: 1.0.0\n  kind: app\n  title: {en: Rollback}\n  description: {en: Rollback}\n  permissions: []\n";
    home.docker
        .fail_pulls(Some("registry rejected installation"));
    let response = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins",
            &headers,
            Body::from(package),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_GATEWAY,
        "{}",
        response.text()
    );
    assert_eq!(response.error_kind(), Some("image_pull_failed"));
    let original = response.json()["error"]["message"].clone();
    let detail = home
        .get(CORE, "/_wonton/api/plugins/app.rollback", &headers)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let result = detail.json()["installed"]["install_failure"].clone();
    assert_eq!(result["kind"], "image_pull_failed");
    assert_eq!(result["message"], original);
    assert_eq!(result["stage"], "pulling_images");
    assert_eq!(result["cleanup_errors"], json!([]));
    assert!(result.get("retryable").is_none());
    let retry = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/app.rollback/retry",
            &headers,
            Body::empty(),
        )
        .await;
    assert_eq!(retry.status, StatusCode::CONFLICT, "{}", retry.text());
    assert_eq!(retry.error_kind(), Some("conflict"));
    let list = home.get(CORE, "/_wonton/api/plugins", &headers).await;
    assert_eq!(list.status, StatusCode::OK);
    assert!(
        list.json()["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|plugin| {
                plugin["id"] == "app.rollback" && plugin["install_failure"] == result
            })
    );
    let cleared = home
        .send(
            Method::DELETE,
            CORE,
            "/_wonton/api/plugins/app.rollback",
            &headers,
            Body::empty(),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text());
    let missing = home
        .get(CORE, "/_wonton/api/plugins/app.rollback", &headers)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn confirmed_install_can_be_retried_after_rollback() {
    let home = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let headers = [("authorization", bearer.as_str())];
    let package = "services:\n  app:\n    image: nginx\nx-wonton:\n  spec: '0.5'\n  id: app.rollback\n  version: 1.0.0\n  kind: app\n  title: {en: Rollback}\n  description: {en: Rollback}\n  permissions: []\n";
    let preview = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &headers,
            Body::from(package),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text());
    let preview = preview.json();
    let request = json!({"source": preview["source"], "digest": preview["digest"], "plan": preview["plan"], "grant": []});
    let json_headers = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/json"),
    ];
    home.docker
        .fail_pulls(Some("registry rejected installation"));
    let failed = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins",
            &json_headers,
            Body::from(request.to_string()),
        )
        .await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY, "{}", failed.text());
    assert_eq!(failed.error_kind(), Some("image_pull_failed"));
    let detail = home
        .get(CORE, "/_wonton/api/plugins/app.rollback", &headers)
        .await;
    assert_eq!(
        detail.json()["installed"]["install_failure"]["retryable"],
        true
    );

    home.docker.fail_pulls(None);
    let retry = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/app.rollback/retry",
            &headers,
            Body::empty(),
        )
        .await;
    assert_eq!(retry.status, StatusCode::NO_CONTENT, "{}", retry.text());
    let detail = home
        .get(CORE, "/_wonton/api/plugins/app.rollback", &headers)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    assert!(detail.json()["installed"]["install_failure"].is_null());
}

/// 服务到时限还在启动：504 和单独的类别，报告点名服务。截图里的“稍后再试”说不清是慢还是坏（#682）。
#[tokio::test]
async fn a_service_still_starting_at_the_deadline_answers_gateway_timeout() {
    let home = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let headers = [("authorization", bearer.as_str())];
    // 测试的安装期限是 5 秒；健康检查的判定窗（1 秒 × 1 次）比它短，时限就按 5 秒。
    let package = "services:\n  app:\n    image: nginx\n    healthcheck:\n      test: ['CMD', 'true']\n      interval: 1s\n      retries: 1\nx-wonton:\n  spec: '0.5'\n  id: app.timeout\n  version: 1.0.0\n  kind: app\n  title: {en: Timeout}\n  description: {en: Timeout}\n  permissions: []\n";
    home.docker.set_state(
        &container_name("app-timeout", "app"),
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
    );
    let response = home
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins",
            &headers,
            Body::from(package),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::GATEWAY_TIMEOUT,
        "{}",
        response.text()
    );
    assert_eq!(response.error_kind(), Some("health_timeout"));
    let message = response.json()["error"]["message"].clone();
    let message = message.as_str().unwrap();
    assert!(
        message.contains(&container_name("app-timeout", "app")),
        "{message}"
    );
    assert!(message.contains("of 5 s"), "{message}");
    let detail = home
        .get(CORE, "/_wonton/api/plugins/app.timeout", &headers)
        .await;
    let failure = detail.json()["installed"]["install_failure"].clone();
    assert_eq!(failure["kind"], "health_timeout");
    // 直装这条链不带重试要的原包（见 install_checked：retry 传 None）。
    assert!(failure.get("retryable").is_none());
}
