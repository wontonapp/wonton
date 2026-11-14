//! Credential-bearing diagnostics are sanitized before rmcp can log or return them.

use std::borrow::Cow;
use std::io;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use rmcp::model::{ErrorData, JsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{StreamableHttpError, StreamableHttpPostResponse};
use sse_stream::{Error as SseError, Sse};

use super::bounded_http::HttpError;
use crate::plugin::remote_redaction::Redaction;

#[derive(Clone, Default)]
pub(super) struct Diagnostics(pub Option<Redaction>);

#[derive(serde::Deserialize)]
struct DiagnosticFrame {
    error: Option<ErrorData>,
}

impl Diagnostics {
    pub fn message(&self, mut message: ServerJsonRpcMessage) -> ServerJsonRpcMessage {
        if let Some(redaction) = &self.0
            && let JsonRpcMessage::Error(error) = &mut message
        {
            redaction.error(&mut error.error);
        }
        message
    }

    pub fn words<'a>(&self, body: &'a [u8], limit: usize) -> Cow<'a, str> {
        let Some(redaction) = &self.0 else {
            return String::from_utf8_lossy(&body[..body.len().min(limit)]);
        };
        let text = String::from_utf8_lossy(body);
        let mut text = redaction.text(&text).into_owned();
        let mut end = text.len().min(limit);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        Cow::Owned(text)
    }

    pub fn post(
        &self,
        reply: Result<StreamableHttpPostResponse, HttpError>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        if self.0.is_none() {
            return reply;
        }
        match reply {
            Ok(StreamableHttpPostResponse::Json(message, session)) => Ok(
                StreamableHttpPostResponse::Json(self.message(message), session),
            ),
            Ok(StreamableHttpPostResponse::Sse(events, session)) => Ok(
                StreamableHttpPostResponse::Sse(self.events(events), session),
            ),
            Ok(StreamableHttpPostResponse::Accepted) => Ok(StreamableHttpPostResponse::Accepted),
            Ok(_) => Err(StreamableHttpError::UnexpectedServerResponse(
                "unsupported remote MCP HTTP response kind".into(),
            )),
            Err(error) => Err(self.error(error)),
        }
    }

    pub fn events(
        &self,
        events: BoxStream<'static, Result<Sse, SseError>>,
    ) -> BoxStream<'static, Result<Sse, SseError>> {
        self.map_events(events, false)
    }

    pub fn get_events(
        &self,
        events: BoxStream<'static, Result<Sse, SseError>>,
    ) -> BoxStream<'static, Result<Sse, SseError>> {
        self.map_events(events, true)
    }

    fn map_events(
        &self,
        events: BoxStream<'static, Result<Sse, SseError>>,
        sdk_bounded: bool,
    ) -> BoxStream<'static, Result<Sse, SseError>> {
        let Some(redaction) = self.0.clone() else {
            return events;
        };
        events
            .map(move |event| {
                // rmcp 3.8's private EventTooLarge is source-less, unlike its Source
                // variant. Its text contains only our configured limit, not remote data.
                // Preserve that exact object: rmcp downcasts it to stop reconnecting.
                let mut event = event.map_err(|error| match &error {
                    SseError::Body(body) if sdk_bounded && body.source().is_none() => error,
                    _ => sse_error(&redaction, error),
                })?;
                if let Some(data) = &mut event.data {
                    redact_frame(&redaction, data);
                }
                Ok(event)
            })
            .boxed()
    }

    pub fn error(&self, error: HttpError) -> HttpError {
        let Some(redaction) = &self.0 else {
            return error;
        };
        match error {
            StreamableHttpError::AuthRequired(mut challenge) => {
                if let Cow::Owned(text) = redaction.text(&challenge.www_authenticate_header) {
                    challenge.www_authenticate_header = text;
                }
                StreamableHttpError::AuthRequired(challenge)
            }
            StreamableHttpError::InsufficientScope(mut challenge) => {
                if let Cow::Owned(text) = redaction.text(&challenge.www_authenticate_header) {
                    challenge.www_authenticate_header = text;
                }
                if let Some(scope) = &mut challenge.required_scope
                    && let Cow::Owned(text) = redaction.text(scope)
                {
                    *scope = text;
                }
                StreamableHttpError::InsufficientScope(challenge)
            }
            StreamableHttpError::UnexpectedServerResponse(text) => {
                StreamableHttpError::UnexpectedServerResponse(
                    redaction.text(&text).into_owned().into(),
                )
            }
            StreamableHttpError::UnexpectedContentType(content_type) => {
                StreamableHttpError::UnexpectedContentType(
                    content_type.map(|value| redaction.text(&value).into_owned()),
                )
            }
            StreamableHttpError::Io(error) => {
                let text = error.to_string();
                match redaction.text(&text) {
                    Cow::Borrowed(_) => StreamableHttpError::Io(error),
                    Cow::Owned(text) => StreamableHttpError::Io(io::Error::new(error.kind(), text)),
                }
            }
            StreamableHttpError::Sse(error) => {
                StreamableHttpError::Sse(sse_error(redaction, error))
            }
            StreamableHttpError::Client(error) => StreamableHttpError::Client(error.without_url()),
            StreamableHttpError::ReinitializationFailed(mut error) => {
                redaction.error(&mut error);
                StreamableHttpError::ReinitializationFailed(error)
            }
            other => other,
        }
    }
}

fn sse_error(redaction: &Redaction, error: SseError) -> SseError {
    match error {
        SseError::Body(error) => match redaction.text(&error.to_string()) {
            Cow::Borrowed(_) => SseError::Body(error),
            Cow::Owned(text) => SseError::Body(Box::new(io::Error::other(text))),
        },
        other => other,
    }
}

fn redact_frame(redaction: &Redaction, data: &mut String) {
    // Serde structs also accept sequences. Check the root before the cheap peek
    // so a malformed array can never reach object mutation.
    if data.trim_start().starts_with('{') {
        match serde_json::from_str::<DiagnosticFrame>(data) {
            Ok(DiagnosticFrame { error: None }) => return,
            Ok(DiagnosticFrame {
                error: Some(mut error),
            }) => {
                if let Ok(serde_json::Value::Object(mut frame)) = serde_json::from_str(data) {
                    redaction.error(&mut error);
                    frame.insert(
                        "error".into(),
                        serde_json::to_value(error).expect("RPC error serializes"),
                    );
                    *data = serde_json::to_string(&frame).expect("JSON object serializes");
                    return;
                }
            }
            Err(_) => {}
        }
    }
    // Malformed error envelopes can still appear in SDK parse diagnostics.
    if let Cow::Owned(redacted) = redaction.text(data) {
        *data = redacted;
    }
}
