use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tokio::task::JoinHandle;
use tokio_websockets::{Message, ServerBuilder};

use super::*;

const PASSWORD: &str = "test-mailpit-password";

#[derive(Default)]
struct StateData {
    ids: Vec<String>,
    pages: Vec<(usize, usize)>,
    deletions: Vec<Value>,
    attachment_size: usize,
    fail: bool,
    malformed_event: bool,
    error_close: bool,
    drop_connection: bool,
}

type FakeState = Arc<Mutex<StateData>>;

struct Fake {
    authority: String,
    state: FakeState,
    task: JoinHandle<()>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fake {
    async fn start() -> Self {
        let state = FakeState::default();
        let app = Router::new()
            .route("/api/v1/messages", get(list).delete(delete))
            .route("/api/v1/message/{id}", get(message))
            .route("/api/v1/message/{id}/headers", get(headers))
            .route("/api/v1/message/{id}/part/{part}", get(attachment))
            .route("/api/events", get(events))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let authority = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            authority,
            state,
            task,
        }
    }

    fn api(&self) -> Api {
        Api::new(&self.authority, PASSWORD, HttpClient::new()).unwrap()
    }
}

fn authorized(headers: &HeaderMap) -> bool {
    let expected = format!("Basic {}", STANDARD.encode(format!("core:{PASSWORD}")));
    headers
        .get(header::AUTHORIZATION)
        .is_some_and(|value| value == expected.as_str())
}

async fn list(
    State(state): State<FakeState>,
    Query(query): Query<HashMap<String, usize>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut state = state.lock().unwrap();
    if state.fail {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let start = query["start"];
    let limit = query["limit"];
    state.pages.push((start, limit));
    let messages: Vec<_> = state
        .ids
        .iter()
        .skip(start)
        .take(limit)
        .map(|id| json!({"ID": id}))
        .collect();
    Json(json!({"total": state.ids.len(), "start": start, "messages": messages})).into_response()
}

async fn delete(State(state): State<FakeState>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut state = state.lock().unwrap();
    if state.fail {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let body: Value = serde_json::from_slice(&body).unwrap();
    state.deletions.push(body);
    "ok".into_response()
}

async fn message(Path(id): Path<String>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if id == "missing" {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Exercise responses beyond HttpClient::json's unrelated 1 MiB limit.
    Json(json!({"ID": id, "Text": "x".repeat((1 << 20) + 1)})).into_response()
}

async fn headers(Path(id): Path<String>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"Received": [id]})).into_response()
}

async fn attachment(
    State(state): State<FakeState>,
    Path((id, part)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if id != "message/id?" || part != "part/id?" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let size = state.lock().unwrap().attachment_size;
    // Chunked: the collection limit must work without trusting Content-Length.
    let chunks = futures_util::stream::iter((0..size).step_by(64 * 1024).map(move |offset| {
        Ok::<_, std::io::Error>(Bytes::from(vec![7; (size - offset).min(64 * 1024)]))
    }));
    Body::from_stream(chunks).into_response()
}

async fn events(State(state): State<FakeState>, mut request: Request<Body>) -> Response {
    if !authorized(request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let key = request.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
    let mut hash = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    hash.update(key.as_bytes());
    hash.update(WEBSOCKET_GUID);
    let accept = STANDARD.encode(hash.finish().as_ref());
    let (malformed, error_close, drop_connection) = {
        let state = state.lock().unwrap();
        (
            state.malformed_event,
            state.error_close,
            state.drop_connection,
        )
    };
    tokio::spawn(async move {
        let upgraded = hyper::upgrade::on(&mut request).await.unwrap();
        let mut stream = ServerBuilder::new().serve(TokioIo::new(upgraded));
        if drop_connection {
            return;
        }
        if malformed {
            stream
                .send(Message::text(r#"{"Type":"new","Data":{}}"#))
                .await
                .unwrap();
            return;
        }
        if error_close {
            stream
                .send(Message::close(
                    Some(tokio_websockets::CloseCode::INTERNAL_SERVER_ERROR),
                    "",
                ))
                .await
                .unwrap();
            let _ = stream.next().await;
            return;
        }
        stream
            .send(Message::ping(Bytes::from_static(b"mailpit")))
            .await
            .unwrap();
        let pong = stream.next().await.unwrap().unwrap();
        assert!(pong.is_pong());
        assert_eq!(&pong.as_payload()[..], b"mailpit");
        stream
            .send(Message::text(r#"{"Type":"stats","Data":{"Total":1}}"#))
            .await
            .unwrap();
        stream
            .send(Message::text(r#"{"Type":"new","Data":{"ID":"new-id"}}"#))
            .await
            .unwrap();
        stream.send(Message::close(None, "")).await.unwrap();
        let reply = stream.next().await.unwrap().unwrap();
        assert!(reply.is_close());
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn lists_every_page_without_requesting_past_the_actual_total() {
    let fake = Fake::start().await;
    let expected: Vec<_> = (0..203).map(|id| format!("id-{id}")).collect();
    fake.state.lock().unwrap().ids = expected.clone();
    assert_eq!(fake.api().list().await.unwrap(), expected);
    assert_eq!(
        fake.state.lock().unwrap().pages,
        [(0, 100), (100, 100), (200, 3)]
    );
}

#[tokio::test]
async fn empty_mailbox_stops_after_first_page() {
    let fake = Fake::start().await;
    assert!(fake.api().list().await.unwrap().is_empty());
    assert_eq!(fake.state.lock().unwrap().pages, [(0, 100)]);
}

#[tokio::test]
async fn basic_auth_is_required_for_http_and_websocket() {
    let fake = Fake::start().await;
    let api = Api::new(&fake.authority, "wrong", HttpClient::new()).unwrap();
    assert!(api.authorization.is_sensitive());
    assert!(matches!(
        api.list().await,
        Err(Error::Status(StatusCode::UNAUTHORIZED))
    ));
    assert!(matches!(
        api.subscribe().await,
        Err(Error::Status(StatusCode::UNAUTHORIZED))
    ));
    assert!(fake.state.lock().unwrap().pages.is_empty());
}

#[tokio::test]
async fn deletes_exactly_one_id_and_never_uses_delete_all() {
    let fake = Fake::start().await;
    let api = fake.api();
    assert!(api.delete("").await.is_err());
    api.delete("one/id\"?").await.unwrap();
    assert_eq!(
        fake.state.lock().unwrap().deletions,
        [json!({"IDs": ["one/id\"?"]})]
    );
}

#[tokio::test]
async fn failed_api_requests_do_not_report_success() {
    let fake = Fake::start().await;
    fake.state.lock().unwrap().fail = true;
    let api = fake.api();
    assert!(matches!(
        api.list().await,
        Err(Error::Status(StatusCode::SERVICE_UNAVAILABLE))
    ));
    assert!(matches!(
        api.delete("one").await,
        Err(Error::Status(StatusCode::SERVICE_UNAVAILABLE))
    ));
    assert!(matches!(api.message("missing").await, Err(Error::NotFound)));
    assert!(fake.state.lock().unwrap().deletions.is_empty());
}

#[tokio::test]
async fn encodes_path_segments_and_accepts_large_json() {
    let fake = Fake::start().await;
    let api = fake.api();
    let message = api.message("one/id?").await.unwrap();
    assert_eq!(message["ID"], "one/id?");
    assert_eq!(message["Text"].as_str().unwrap().len(), (1 << 20) + 1);
    assert_eq!(
        api.headers("one/id?").await.unwrap(),
        json!({"Received": ["one/id?"]})
    );
}

#[tokio::test]
async fn attachment_accepts_exactly_twenty_mib_and_rejects_one_more_byte() {
    let fake = Fake::start().await;
    let api = fake.api();
    assert_eq!(MAX_ATTACHMENT_BYTES, 20 * 1024 * 1024);
    fake.state.lock().unwrap().attachment_size = MAX_ATTACHMENT_BYTES;
    let bytes = api.attachment("message/id?", "part/id?").await.unwrap();
    assert_eq!(bytes.len(), MAX_ATTACHMENT_BYTES);
    assert!(bytes.iter().all(|byte| *byte == 7));
    fake.state.lock().unwrap().attachment_size = MAX_ATTACHMENT_BYTES + 1;
    assert!(matches!(
        api.attachment("message/id?", "part/id?").await,
        Err(Error::Body(_))
    ));
}

#[tokio::test]
async fn websocket_handles_ping_skips_other_events_and_acknowledges_close() {
    let fake = Fake::start().await;
    let mut events = fake.api().subscribe().await.unwrap();
    let id = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id.as_deref(), Some("new-id"));
    assert!(events.next().await.unwrap().is_none());
    assert!(events.next().await.unwrap().is_none());
}

#[test]
fn rejects_invalid_websocket_upgrade() {
    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_ACCEPT, "invalid")
        .body(())
        .unwrap();
    assert!(matches!(
        validate_upgrade(&response, "key"),
        Err(Error::Upgrade(_))
    ));
}

#[tokio::test]
async fn malformed_new_event_is_an_error_not_a_silent_skip() {
    let fake = Fake::start().await;
    fake.state.lock().unwrap().malformed_event = true;
    let mut events = fake.api().subscribe().await.unwrap();
    assert!(matches!(events.next().await, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn websocket_error_close_is_not_a_clean_end() {
    let fake = Fake::start().await;
    fake.state.lock().unwrap().error_close = true;
    let mut events = fake.api().subscribe().await.unwrap();
    assert!(matches!(events.next().await, Err(Error::Closed(1011))));
}

#[tokio::test]
async fn unexpected_websocket_disconnect_is_not_silently_ignored() {
    let fake = Fake::start().await;
    fake.state.lock().unwrap().drop_connection = true;
    let mut events = fake.api().subscribe().await.unwrap();
    assert!(matches!(
        events.next().await,
        Err(Error::UnexpectedEnd | Error::Websocket(_))
    ));
}
