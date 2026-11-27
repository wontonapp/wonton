//! 原生浏览（影视的 `catalog`、相册的 `library`）经核心转给媒体源的行为测试。夹具和假提供方在
//! [`super::media_tests`]。

use axum::body::Body;
use axum::http::StatusCode;
use hyper::Method;
use serde_json::{Value, json};

use super::media_tests::fixture;

#[tokio::test]
async fn catalog_requests_reach_the_provider_as_sent_without_the_source() {
    let f = fixture().await;
    let (status, _, body) = f
        .send(
            Method::GET,
            "/_wonton/api/media/catalog/entries/s%2F1?source=app.example.media&sort=title",
            &[],
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["entry"]["id"],
        "s/1"
    );
    assert_eq!(f.hits("GET /catalog/entries/s%2F1?sort=title"), 1);
    let (status, result) = f
        .json(
            Method::POST,
            "/_wonton/api/media/catalog/actions",
            json!({"action": "mark_played", "ids": ["e1", "e2"]}),
        )
        .await;
    assert_eq!(
        (status, result["done"].clone()),
        (StatusCode::OK, json!(["e1", "e2"]))
    );
    let before = f.hits("GET /");
    for escape in ["../tasks", "entries/%2E%2E", "entries//x"] {
        let (status, _, _) = f
            .send(
                Method::GET,
                &format!("/_wonton/api/media/catalog/{escape}"),
                &[],
                Body::empty(),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{escape}");
    }
    assert_eq!(f.hits("GET /"), before);
    // 相册没有提供方声明 `library`：明确说没有，不转给别家。
    let (status, _, body) = f
        .send(
            Method::GET,
            "/_wonton/api/media/library/timeline",
            &[],
            Body::empty(),
        )
        .await;
    let failure: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("plugin_unavailable"))
    );
}

#[tokio::test]
async fn the_browse_roots_and_an_empty_source_follow_the_usual_rules() {
    let f = fixture().await;
    // 根路径带不带结尾的 `/` 都进媒体路由：这里没有提供方声明 `library`，回失败信封，不落到核心的兜底页。
    for path in ["/_wonton/api/media/library", "/_wonton/api/media/library/"] {
        let (status, _, body) = f.send(Method::GET, path, &[], Body::empty()).await;
        let failure: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (status, failure["code"].as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, Some("plugin_unavailable")),
            "{path}"
        );
    }
    // `source=` 空着当没带：按默认规则选到唯一那家，转过去的查询串里也没有它。
    let (status, _, _) = f
        .send(
            Method::GET,
            "/_wonton/api/media/catalog/entries/abc?source=",
            &[],
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.hits("GET /catalog/entries/abc"), 1);
}
