//! 播放器照核心给的流地址取 HLS、条目封面指向核心的行为测试。夹具和假提供方在 [`super::media_tests`]。

use axum::body::Body;
use axum::http::{StatusCode, header};
use hyper::Method;
use serde_json::Value;
use url::Url;
use wonton_protocol::media::api::{item_artwork_path, session_stream_path};
use wonton_protocol::media::provider::ArtworkQuery;
use wonton_protocol::media::{MediaItemRef, ProviderStream};

use super::media_tests::{Fixture, PROVIDER, fixture};

/// 播放器那样：按上一份列表的地址解析相对地址，取回来。
async fn fetch(f: &Fixture, url: &Url) -> (StatusCode, String) {
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    let (status, _, body) = f.send(Method::GET, &path, &[], Body::empty()).await;
    (status, String::from_utf8(body).unwrap())
}

fn first_uri(playlist: &str) -> &str {
    playlist
        .lines()
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap()
}

#[tokio::test]
async fn an_hls_player_follows_the_relative_playlists_from_the_stream_it_was_given() {
    let f = fixture().await;
    let (id, session) = f.start_session_of("hls").await;
    let stream: ProviderStream = serde_json::from_value(session["plan"]["stream"].clone()).unwrap();
    let ProviderStream::Transcoded(master) = stream else {
        panic!("{stream:?}");
    };
    assert!(!master.contains("token"), "提供方的查询串不下发：{master}");
    let master = Url::parse("http://core.invalid")
        .unwrap()
        .join(&master)
        .unwrap();
    let (status, body) = fetch(&f, &master).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.hits("GET /hls/up-1/master.m3u8?token=abc"), 1);
    let main = master.join(first_uri(&body)).unwrap();
    let (status, body) = fetch(&f, &main).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.hits("GET /hls/up-1/main.m3u8?DeviceId=d1"), 1);
    let segment = main.join(first_uri(&body)).unwrap();
    let (status, body) = fetch(&f, &segment).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "segment"));
    assert_eq!(f.hits("GET /hls/up-1/hls1/main/0.ts?seg=0"), 1);
    // 跑出流地址所在目录的一律拒，不转给提供方。
    let fetched = f.hits("GET /");
    for escape in ["../../uploads", "%2E%2E/master.m3u8", "a//b"] {
        let path = format!("{}/{escape}", session_stream_path(id));
        let (status, _, body) = f.send(Method::GET, &path, &[], Body::empty()).await;
        let failure: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (status, failure["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("protocol")),
            "{escape}"
        );
    }
    assert_eq!(f.hits("GET /"), fetched);
}

#[tokio::test]
async fn listed_items_point_their_artwork_at_the_core() {
    let f = fixture().await;
    let (status, page) = f
        .json(
            Method::GET,
            "/_wonton/api/media/items?kind=video",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items: Vec<MediaItemRef> = serde_json::from_value(page["items"].clone()).unwrap();
    assert!(
        items
            .iter()
            .all(|item| item.source.as_deref() == Some(PROVIDER))
    );
    let artwork = items[0].artwork_url.clone().unwrap();
    assert_eq!(
        artwork,
        item_artwork_path(&items[0], &ArtworkQuery::default())
    );
    assert_eq!(items[1].artwork_url, None);
    let (status, headers, body) = f
        .send(
            Method::GET,
            &format!("{artwork}&type=backdrop&width=640"),
            &[],
            Body::empty(),
        )
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &b"jpeg"[..]));
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(f.hits("GET /items/m1/artwork?type=backdrop&width=640"), 1);
}
