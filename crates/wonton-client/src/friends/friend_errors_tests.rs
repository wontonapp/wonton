//! 用本地 HTTP 响应经过公开 Matrix 错误边界，再观察好友 ViewModel；不把未知上游失败当作地址或邀请状态。
//! 资料 API 自带短重试，避免为验证 HTTP 分类等待建房 API 的长退避；生产重试策略不改。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::sync::watch;
use wonton_matrix::{Error, MatrixSession, OwnedRoomId, OwnedUserId, SessionConfig, StoreConfig};
use wonton_protocol::{FriendFailure, FriendProblem, FriendsState, PushMethod};

use super::FriendsLink;
use crate::chat::{ChatChannels, ChatLink};

const OWNER: &str = "@owner:example.org";
const FRIEND: &str = "@friend:example.org";
const ROOM: &str = "!joined:example.org";

struct Homeserver {
    url: String,
    created: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Homeserver {
    async fn start(status: u16, errcode: Option<&'static str>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let created = Arc::new(AtomicUsize::new(0));
        let observed = created.clone();
        let app = Router::new()
            .route(
                "/_matrix/client/versions",
                get(|| async {
                    Json(json!({
                        "versions": ["v1.11", "v1.15"],
                        "unstable_features": {"org.matrix.simplified_msc3575": true}
                    }))
                }),
            )
            .route(
                "/_matrix/client/v3/profile/{user}",
                get(move || async move {
                    let status = StatusCode::from_u16(status).unwrap();
                    match errcode {
                        Some(errcode) => (
                            status,
                            Json(json!({"errcode": errcode, "error": "unavailable"})),
                        )
                            .into_response(),
                        None => (status, "proxy unavailable").into_response(),
                    }
                }),
            )
            .route(
                "/_matrix/client/v3/sync",
                get(|| async { Json(joined_room()) }),
            )
            .route(
                "/_matrix/client/v3/createRoom",
                post(move || {
                    observed.fetch_add(1, Ordering::SeqCst);
                    async { Json(json!({"room_id": ROOM})) }
                }),
            )
            .fallback(|| async {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"errcode": "M_UNRECOGNIZED", "error": "not in this fixture"})),
                )
            });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, created, task }
    }

    async fn session(&self) -> MatrixSession {
        let saved = serde_json::from_value(json!({
            "user_id": OWNER,
            "device_id": "FRIEND_ERRORS",
            "access_token": "isolated-test-token"
        }))
        .unwrap();
        MatrixSession::restore(
            &SessionConfig {
                homeserver_url: self.url.clone(),
                device_name: "friend-errors".to_owned(),
                store: StoreConfig::InMemory,
                proxy: None,
            },
            saved,
        )
        .await
        .unwrap()
    }
}

impl Drop for Homeserver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn joined_room() -> Value {
    json!({
        "next_batch": "one",
        "rooms": {"join": {ROOM: {
            "state": {"events": [{
                "type": "m.room.create", "state_key": "", "event_id": "$create",
                "sender": OWNER, "origin_server_ts": 1,
                "content": {"creator": OWNER, "room_version": "10"}
            }, {
                "type": "m.room.member", "state_key": OWNER, "event_id": "$owner",
                "sender": OWNER, "origin_server_ts": 2,
                "content": {"membership": "join"}
            }]},
            "timeline": {"limited": false, "prev_batch": "zero", "events": []}
        }}}
    })
}

fn friends() -> (Arc<FriendsLink>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Handle::current();
    let (_, method) = watch::channel(PushMethod::default());
    let home = crate::home::HomeLink::new(
        runtime.clone(),
        crate::sharing::hub::bind(runtime.clone()).unwrap(),
        crate::home::Storage::open(dir.path(), dir.path()).unwrap(),
        crate::home::Network::DirectOnly,
        None,
        method.clone(),
    )
    .unwrap();
    let plugins =
        crate::plugins::PluginsLink::new(runtime.clone(), home.clone(), dir.path(), None, false);
    #[cfg(not(feature = "test-hooks"))]
    let card_success: crate::chat::CardSuccess = Arc::new(|_| {});
    #[cfg(feature = "test-hooks")]
    let card_success: crate::chat::CardSuccess = Arc::new(|_| Box::pin(async {}));
    let chat = ChatLink::new(
        runtime.clone(),
        home.clone(),
        "friend-errors",
        dir.path(),
        card_success,
        plugins,
        ChatChannels {
            view: None,
            confirmations: None,
            method,
            lock_screen: watch::channel(crate::lock_screen::Hidden::default()).1,
            app_lock: crate::app_lock::AppLockLink::new(dir.path()),
            session_out: watch::Sender::new(None),
            external: false,
            push_previews: Arc::new(crate::push::PushPreviews::open(dir.path())),
        },
    )
    .unwrap();
    let link = FriendsLink::new(runtime, home, chat, dir.path(), None).unwrap();
    (link, dir)
}

async fn profile_error(session: &MatrixSession) -> Error {
    let user: OwnedUserId = FRIEND.parse().unwrap();
    tokio::time::timeout(Duration::from_secs(30), session.fetch_user_profile(&user))
        .await
        .expect("the isolated profile endpoint must finish")
        .unwrap_err()
}

fn problem_after(friends: &FriendsLink, error: Error) -> FriendProblem {
    let mut updates = friends.subscribe();
    friends.fail(error);
    let view = updates.borrow_and_update();
    let FriendsState::Failed { problem } = &view.state else {
        panic!("expected a visible friend failure: {view:?}");
    };
    problem.clone()
}

#[tokio::test]
async fn valid_addresses_with_request_or_service_errors_keep_an_uncertain_friend_failure() {
    let (friends, _dir) = friends();
    for status in [400, 502, 503, 504] {
        let server = Homeserver::start(status, Some("M_UNKNOWN")).await;
        let session = server.session().await;
        let problem = problem_after(&friends, profile_error(&session).await);
        assert_eq!(problem.reason, FriendFailure::Upstream, "HTTP {status}");
        assert_eq!(
            problem.http_status, None,
            "discarded status must not be guessed"
        );
        assert_eq!(problem.matrix_errcode, None);
    }
}

#[tokio::test]
async fn an_unavailable_profile_is_not_an_invalid_friend_address() {
    let (friends, _dir) = friends();
    let server = Homeserver::start(404, Some("M_NOT_FOUND")).await;
    let session = server.session().await;
    let user: OwnedUserId = FRIEND.parse().unwrap();
    let error = session.invite_friend(&user).await.unwrap_err();
    assert!(matches!(error, Error::Upstream(_)), "{error:?}");
    let problem = problem_after(&friends, error);
    assert_eq!(problem.reason, FriendFailure::Upstream);
    assert_eq!(server.created.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_known_non_invited_room_is_distinct_from_an_unavailable_service() {
    let (friends, _dir) = friends();
    let server = Homeserver::start(503, Some("M_UNKNOWN")).await;
    let session = server.session().await;
    let unavailable = profile_error(&session).await;
    assert!(matches!(unavailable, Error::NotReady(_)), "{unavailable:?}");
    assert_eq!(
        problem_after(&friends, unavailable).reason,
        FriendFailure::Upstream
    );

    session
        .client()
        .sync_once(Default::default())
        .await
        .unwrap();
    let room: OwnedRoomId = ROOM.parse().unwrap();
    let handled = session.reject_invite(&room).await.unwrap_err();
    assert!(matches!(handled, Error::NotInvited(_)), "{handled:?}");
    assert_eq!(
        problem_after(&friends, handled).reason,
        FriendFailure::RequestNotFound
    );
}

#[tokio::test]
async fn transport_refusal_and_session_errors_remain_distinct_in_the_friend_view() {
    let (friends, _dir) = friends();
    for (status, errcode, expected) in [
        (403, None, FriendFailure::TransportUnavailable),
        (403, Some("M_FORBIDDEN"), FriendFailure::Forbidden),
        (401, Some("M_UNKNOWN_TOKEN"), FriendFailure::Unauthorized),
    ] {
        let server = Homeserver::start(status, errcode).await;
        let session = server.session().await;
        let problem = problem_after(&friends, profile_error(&session).await);
        assert_eq!(problem.reason, expected, "HTTP {status}, {errcode:?}");
        if expected == FriendFailure::Forbidden {
            assert_eq!(problem.http_status, Some(403));
            assert_eq!(problem.matrix_errcode.as_deref(), Some("M_FORBIDDEN"));
        } else {
            assert_eq!(problem.http_status, None);
        }
    }
}
