use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness, TOKEN};

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
