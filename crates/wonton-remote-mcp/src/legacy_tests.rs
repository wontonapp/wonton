use std::sync::{Arc, Mutex, Weak};

use axum::body::Bytes;
use axum::http::HeaderMap;
use tokio::sync::mpsc;

use super::{OpenError, Session, State};

fn session() -> Arc<Session> {
    let (_sender, receiver) = mpsc::channel(1);
    Arc::new(Session {
        id: "test-session".to_owned(),
        plugin: "app.example.test".to_owned(),
        url: "https://remote.example/mcp".parse().unwrap(),
        endpoint: "https://remote.example/messages".parse().unwrap(),
        credentials: HeaderMap::new(),
        registry: Weak::new(),
        state: Mutex::new(State {
            closed: false,
            reason: "closed",
            pending: Default::default(),
            events: Some(receiver),
            reader: None,
        }),
    })
}

#[tokio::test]
async fn closing_between_open_observation_and_registration_cannot_leave_a_pending_request() {
    let session = session();
    assert!(session.is_open());
    session.close("remote disconnected");
    let request = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call"}"#;
    assert!(matches!(
        session.register(request, false),
        Err(OpenError::Expired)
    ));
    assert!(session.state.lock().unwrap().pending.is_empty());
}

#[tokio::test]
async fn closing_after_registration_finishes_the_waiter_without_a_reply() {
    let session = session();
    let request = Bytes::from_static(br#"{"jsonrpc":"2.0","id":7,"method":"tools/call"}"#);
    let (receiver, _guard) = session
        .register(&request, false)
        .unwrap_or_else(|_| panic!("open session refused registration"))
        .unwrap();
    session.close("remote disconnected");
    assert!(receiver.await.is_err());
    assert!(session.state.lock().unwrap().pending.is_empty());
}
