use std::sync::{Arc, Mutex};

use aws_lc_rs::digest;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_websockets::{Message, ServerBuilder};

use super::test_support::StateData;

pub(super) async fn subscribe(
    state: Arc<Mutex<StateData>>,
    events: broadcast::Sender<Option<String>>,
    mut request: Request<Body>,
) -> Response<Body> {
    let key = request.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
    let mut hash = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    hash.update(key.as_bytes());
    hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept = STANDARD.encode(hash.finish().as_ref());
    let mut receiver = events.subscribe();
    tokio::spawn(async move {
        let upgraded = hyper::upgrade::on(&mut request).await.unwrap();
        let mut stream = ServerBuilder::new().serve(TokioIo::new(upgraded));
        state.lock().unwrap().connections += 1;
        while let Ok(signal) = receiver.recv().await {
            match signal {
                Some(id) => {
                    let message = json!({"Type":"new","Data":{"ID":id}}).to_string();
                    if stream.send(Message::text(message)).await.is_err() {
                        return;
                    }
                }
                None => {
                    let _ = stream.send(Message::close(None, "")).await;
                    state.lock().unwrap().closed += 1;
                    let _ = stream.next().await;
                    return;
                }
            }
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .unwrap()
}
