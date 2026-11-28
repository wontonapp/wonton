//! 成员移除收回他的媒体：提供方收不回就停在这一步、账号不停，重试时接着做。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hyper::{Method, StatusCode};
use serde_json::json;
use wonton_protocol::media::{MediaKind, MediaOperation};

use super::Bed;
use crate::http::HttpClient;
use crate::media::Media;
use crate::media::provider::credential_name;
use crate::media::test_support::{FakeDirectory, candidate, caps, serve};

const PROVIDER: &str = "app.example.media";

#[tokio::test]
async fn removal_waits_for_the_media_credential_to_be_revoked() {
    let down = Arc::new(AtomicBool::new(true));
    let upstream = down.clone();
    let router = caps(&["playback"], &["video"]).route(
        "/revoke",
        axum::routing::post(move || {
            let down = upstream.load(Ordering::SeqCst);
            async move {
                if down {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::NO_CONTENT
                }
            }
        }),
    );
    let (endpoint, _seen, _served) = serve(router).await;
    let entry = candidate(PROVIDER, &[MediaOperation::Playback], &[MediaKind::Video]);
    let (directory, book) = FakeDirectory::new(vec![(entry, endpoint)]);
    let mut bed = Bed::start().await;
    let media = Media::with_directory(
        &bed.dir.path().join("media"),
        Arc::new(directory),
        HttpClient::new(),
    )
    .unwrap();
    bed.reopen_with(Some(Arc::new(media))).await;
    let (id, _) = bed.pending("alice", false);
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let alice = "@alice:test";
    let media = bed.gateway.media.clone().unwrap();
    media
        .providers
        .route_to(
            alice,
            |_| true,
            PROVIDER,
            &MediaOperation::Playback,
            Some(&MediaKind::Video),
        )
        .await
        .unwrap();

    let reply = bed
        .call(
            Method::POST,
            &wonton_protocol::member_removal_path("alice"),
            json!({"delete_home": false, "keep_bot_data": false}),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert!(!bed.fake.state().user(alice).deactivated);
    assert!(book.revoked.lock().unwrap().is_empty());

    down.store(false, Ordering::SeqCst);
    bed.gateway.sync_members().await.unwrap();
    assert!(bed.fake.state().user(alice).deactivated);
    assert_eq!(
        *book.revoked.lock().unwrap(),
        [(PROVIDER.to_owned(), credential_name(alice))]
    );
    assert!(bed.gateway.members.lifecycle("alice").removal.is_none());
}
