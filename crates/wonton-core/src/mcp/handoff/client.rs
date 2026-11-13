use std::{
    io,
    sync::{Arc, OnceLock},
    time::Duration,
};

use agent_client_protocol::{
    Agent, Client, ConnectTo, Error, Lines, RawJsonRpcMessage, UntypedMessage,
    schema::ProtocolVersion,
    v1::{
        ConnectionTo,
        schema::{
            CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest,
            RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
            SessionId,
        },
    },
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Notify, mpsc, oneshot},
};
use tokio_websockets::{Message, WebSocketStream};

pub(super) enum Event {
    Opened {
        session: String,
        proceed: oneshot::Sender<bool>,
    },
    PromptSent,
    Text(String),
    Ended {
        stop_reason: String,
    },
}

pub(super) async fn run<S>(
    stream: WebSocketStream<S>,
    meta: Map<String, Value>,
    message: String,
    events: mpsc::UnboundedSender<Event>,
    mut cancel: mpsc::UnboundedReceiver<String>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let cancel_flushed = Arc::new(Notify::new());
    let transport = transport(stream, events.clone(), cancel_flushed.clone());
    let expected_session = Arc::new(OnceLock::<SessionId>::new());
    let update_session = expected_session.clone();
    let update_events = events.clone();
    Client.v1()
        .on_receive_request(
            async |_: RequestPermissionRequest, responder, _| {
                responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
            },
            agent_client_protocol::on_receive_request!(),
        )
        // Untyped notifications preserve forward compatibility with new sessionUpdate variants.
        .on_receive_notification(
            async move |notification: UntypedMessage, _| {
                forward_text(notification, update_session.get(), &update_events)
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(transport, async move |connection| {
            let session = tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(15), open(&connection, meta)) => {
                    result.map_err(|_| failure("ACP setup timed out"))??
                }
                _ = cancel.recv() => return Err(failure("handoff cancelled during ACP setup")),
            };
            expected_session.set(session.clone())
                .map_err(|_| failure("ACP session already opened"))?;
            let (proceed, permission) = oneshot::channel();
            if events.send(Event::Opened { session: session.to_string(), proceed }).is_err() {
                cancel_session(&connection, session, &cancel_flushed).await?;
                return Err(failure("handoff receiver closed before registration"));
            }
            let allowed = tokio::select! {
                biased;
                _ = cancel.recv() => false,
                _ = connection.incoming_closed() => return Err(failure("ACP disconnected before prompt")),
                result = permission => result.unwrap_or(false),
            };
            if !allowed {
                cancel_session(&connection, session, &cancel_flushed).await?;
                return Err(failure("handoff cancelled before prompt"));
            }
            prompt(&connection, session, message, events, cancel, cancel_flushed).await
        })
        .await
        .map_err(|error| error.to_string())
}

fn transport<S>(
    stream: WebSocketStream<S>,
    outgoing_events: mpsc::UnboundedSender<Event>,
    flushed: Arc<Notify>,
) -> impl ConnectTo<Client>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sink, incoming) = stream.split();
    // Observe the SDK's serialized envelopes only to acknowledge transport delivery.
    // All request IDs, dispatch, serialization and response routing belong to the SDK.
    let outgoing = futures_util::sink::unfold(sink, move |mut sink, line: String| {
        let events = outgoing_events.clone();
        let flushed = flushed.clone();
        async move {
            let envelope: RawJsonRpcMessage =
                serde_json::from_str(&line).map_err(io::Error::other)?;
            sink.send(Message::text(line))
                .await
                .map_err(io::Error::other)?;
            match envelope {
                RawJsonRpcMessage::Request(request)
                    if request.method.as_ref() == "session/prompt" =>
                {
                    events
                        .send(Event::PromptSent)
                        .map_err(|_| io::Error::other("handoff receiver closed"))?;
                }
                RawJsonRpcMessage::Notification(notification)
                    if notification.method.as_ref() == "session/cancel" =>
                {
                    flushed.notify_one()
                }
                _ => {}
            }
            Ok::<_, io::Error>(sink)
        }
    });
    let incoming = incoming.filter_map(|frame| async move {
        match frame {
            Ok(frame) if frame.is_text() => {
                Some(frame.as_text().map(str::to_owned).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid ACP websocket text")
                }))
            }
            Ok(frame) if frame.is_binary() => Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "binary ACP websocket frame",
            ))),
            Ok(frame) if frame.is_close() => None,
            Ok(_) => None,
            Err(error) => Some(Err(io::Error::other(error))),
        }
    });
    Lines::new(outgoing, incoming)
}

async fn open(
    connection: &ConnectionTo<Agent>,
    meta: Map<String, Value>,
) -> Result<SessionId, Error> {
    let initialized = connection
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    if initialized.protocol_version != ProtocolVersion::V1 {
        return Err(failure("provider did not negotiate ACP v1"));
    }
    let mut request = NewSessionRequest::new("/");
    request.meta = Some(meta);
    Ok(connection
        .send_request(request)
        .block_task()
        .await?
        .session_id)
}

async fn cancel_session(
    connection: &ConnectionTo<Agent>,
    session: SessionId,
    flushed: &Notify,
) -> Result<(), Error> {
    connection.send_notification(CancelNotification::new(session))?;
    tokio::select! {
        _ = flushed.notified() => Ok(()),
        _ = connection.incoming_closed() => Err(failure("ACP disconnected while cancelling session")),
    }
}

async fn prompt(
    connection: &ConnectionTo<Agent>,
    session: SessionId,
    message: String,
    events: mpsc::UnboundedSender<Event>,
    mut cancel: mpsc::UnboundedReceiver<String>,
    cancel_flushed: Arc<Notify>,
) -> Result<(), Error> {
    let request = PromptRequest::new(session.clone(), vec![ContentBlock::from(message)]);
    let response = connection.send_request(request).block_task();
    tokio::pin!(response);
    let completed = tokio::select! {
        biased;
        result = &mut response => result?,
        _ = cancel.recv() => {
            cancel_session(connection, session, &cancel_flushed).await?;
            // ACP cancellation completes with the prompt response; continue dispatching updates.
            response.await?
        }
    };
    let stop_reason = serde_json::to_value(completed.stop_reason)?
        .as_str()
        .ok_or_else(|| failure("invalid ACP stop reason"))?
        .to_owned();
    events
        .send(Event::Ended { stop_reason })
        .map_err(|_| failure("handoff receiver closed before prompt completion"))
}

fn forward_text(
    notification: UntypedMessage,
    expected_session: Option<&SessionId>,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<(), Error> {
    if notification.method != "session/update" {
        return Ok(());
    }
    let Some(session) = expected_session else {
        return Ok(());
    };
    if notification.params.get("sessionId").and_then(Value::as_str) != Some(session.0.as_ref()) {
        return Ok(());
    }
    let Some(update) = notification.params.get("update") else {
        return Err(Error::invalid_params().data("session/update missing update"));
    };
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("agent_message_chunk") {
        return Ok(());
    }
    let content = update
        .get("content")
        .ok_or_else(|| Error::invalid_params().data("missing chunk content"))?;
    if content.get("type").and_then(Value::as_str) == Some("text") {
        let text = content
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_params().data("missing chunk text"))?;
        events
            .send(Event::Text(text.to_owned()))
            .map_err(|_| failure("handoff receiver closed during prompt"))?;
    }
    Ok(())
}

fn failure(message: &str) -> Error {
    Error::internal_error().data(message)
}
