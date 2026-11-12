//! Authenticated Mailpit v1.33 API and event transport over the core's constrained HTTP pool.

use std::time::Duration;

use aws_lc_rs::digest;
use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Limited};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_websockets::{ClientBuilder, Limits, Message, WebSocketStream};

use crate::http::HttpClient;

const TIMEOUT: Duration = Duration::from_secs(30);
// Mailpit 服务端每 54 秒 ping 一次客户端、60 秒收不到 pong 就直接关 TCP（不带 close frame）。
// 60 秒完全静默（连服务端 ping 都没有）说明连接已经死了：先主动 ping 一次逼对端证明连接还在，
// 再等不到任何帧（含 pong）就当作断开。
const READ_IDLE: Duration = Duration::from_secs(60);
const PONG_GRACE: Duration = Duration::from_secs(10);
const PAGE_SIZE: usize = 100;
// A 25 MiB source can expand sixfold through JSON escaping, plus response metadata.
const MAX_JSON_BYTES: usize = 160 * 1024 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = MAX_JSON_BYTES;
const WEBSOCKET_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Mailpit request failed: {0}")]
    Http(#[from] crate::http::HttpError),
    #[error("Mailpit message or attachment not found")]
    NotFound,
    #[error("Mailpit returned HTTP {0}")]
    Status(StatusCode),
    #[error("Mailpit request timed out")]
    Timeout,
    #[error("Invalid Mailpit API response: {0}")]
    Invalid(String),
    #[error("Mailpit response body failed or exceeded its size limit: {0}")]
    Body(String),
    #[error("Mailpit websocket failed: {0}")]
    Websocket(#[from] tokio_websockets::Error),
    #[error("Mailpit websocket upgrade failed: {0}")]
    Upgrade(String),
    #[error("Mailpit websocket closed with error code {0}")]
    Closed(u16),
    #[error("Mailpit websocket ended without a close frame")]
    UnexpectedEnd,
}

impl Error {
    /// 连接层面的失败（解析、连接、超时、对端断开）：等一等再重试就有意义，按传输不可用对待。
    /// 配置错误（密码不对、响应不合法）不在此列，重试不会好转，要按错误处理。
    pub(super) fn is_transport(&self) -> bool {
        match self {
            Error::Http(error) => matches!(
                error,
                crate::http::HttpError::Connect(_)
                    | crate::http::HttpError::Timeout(_)
                    | crate::http::HttpError::ResponseHeadersTimeout(_)
            ),
            Error::Timeout
            | Error::Websocket(_)
            | Error::Upgrade(_)
            | Error::Closed(_)
            | Error::UnexpectedEnd => true,
            Error::NotFound | Error::Status(_) | Error::Invalid(_) | Error::Body(_) => false,
        }
    }
}

#[derive(Clone)]
pub struct Api {
    base: String,
    authorization: HeaderValue,
    http: HttpClient,
}

impl Api {
    pub fn new(authority: &str, password: &str, http: HttpClient) -> Result<Self> {
        let authority: axum::http::uri::Authority = authority
            .parse()
            .map_err(|_| Error::Invalid("invalid Mailpit authority".into()))?;
        if authority.as_str().contains('@') {
            return Err(Error::Invalid(
                "Mailpit authority must not contain credentials".into(),
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!(
            "Basic {}",
            STANDARD.encode(format!("core:{password}"))
        ))
        .map_err(|_| Error::Invalid("invalid Mailpit authorization".into()))?;
        authorization.set_sensitive(true);
        Ok(Self {
            base: format!("http://{authority}"),
            authorization,
            http,
        })
    }

    pub async fn list(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut start = 0;
        let mut total = usize::MAX;
        while start < total {
            let limit = PAGE_SIZE.min(total - start);
            let path = format!("/api/v1/messages?start={start}&limit={limit}");
            let page: Page = serde_json::from_slice(
                &self
                    .exchange(Method::GET, &path, None, MAX_JSON_BYTES)
                    .await?,
            )
            .map_err(|e| Error::Invalid(e.to_string()))?;
            if page.start != start || page.messages.len() > limit {
                return Err(Error::Invalid("invalid message pagination".into()));
            }
            // Freeze the first count so a busy mailbox cannot keep this sweep alive forever.
            total = total.min(page.total);
            if page.messages.is_empty() {
                break;
            }
            start = start
                .checked_add(page.messages.len())
                .ok_or_else(|| Error::Invalid("message pagination overflow".into()))?;
            for message in page.messages {
                require_id(&message.id)?;
                ids.push(message.id);
            }
        }
        Ok(ids)
    }

    pub async fn message(&self, id: &str) -> Result<Value> {
        self.get_json(&message_path(id)?).await
    }

    pub async fn headers(&self, id: &str) -> Result<Value> {
        self.get_json(&format!("{}/headers", message_path(id)?))
            .await
    }

    pub async fn attachment(&self, id: &str, part: &str) -> Result<Vec<u8>> {
        require_id(part)?;
        let path = format!("{}/part/{}", message_path(id)?, segment(part));
        self.exchange(Method::GET, &path, None, MAX_ATTACHMENT_BYTES)
            .await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        require_id(id)?;
        // Mailpit interprets an absent/empty IDs array OR malformed JSON as delete-all.
        let body =
            serde_json::to_vec(&json!({"IDs": [id]})).map_err(|e| Error::Invalid(e.to_string()))?;
        self.exchange(Method::DELETE, "/api/v1/messages", Some(body), 1024)
            .await?;
        Ok(())
    }

    pub async fn subscribe(&self) -> Result<Events> {
        let connect = async {
            let mut key = [0; 16];
            getrandom::fill(&mut key)
                .map_err(|_| Error::Upgrade("random source unavailable".into()))?;
            let key = STANDARD.encode(key);
            let mut request = self.request(Method::GET, "/api/events", None)?;
            let headers = request.headers_mut();
            headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
            headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
            headers.insert(
                header::SEC_WEBSOCKET_VERSION,
                HeaderValue::from_static("13"),
            );
            headers.insert(
                header::SEC_WEBSOCKET_KEY,
                HeaderValue::from_str(&key).unwrap(),
            );
            let mut response = self.http.send(request).await?;
            validate_upgrade(&response, &key)?;
            let upgraded = hyper::upgrade::on(&mut response)
                .await
                .map_err(|e| Error::Upgrade(e.to_string()))?;
            let stream = ClientBuilder::new()
                .limits(Limits::default().max_payload_len(Some(MAX_EVENT_BYTES)))
                .take_over(TokioIo::new(upgraded));
            Ok(Events {
                stream,
                closed: false,
            })
        };
        tokio::time::timeout(TIMEOUT, connect)
            .await
            .map_err(|_| Error::Timeout)?
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let bytes = self
            .exchange(Method::GET, path, None, MAX_JSON_BYTES)
            .await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Invalid(e.to_string()))
    }

    fn request(&self, method: Method, path: &str, body: Option<Vec<u8>>) -> Result<Request<Body>> {
        let uri: Uri = format!("{}{path}", self.base)
            .parse()
            .map_err(|_| Error::Invalid("invalid Mailpit request URL".into()))?;
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::ACCEPT, "application/json");
        let body = match body {
            Some(body) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(body)
            }
            None => Body::empty(),
        };
        builder
            .body(body)
            .map_err(|e| Error::Invalid(e.to_string()))
    }

    async fn exchange(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let request = self.request(method, path, body)?;
        let exchange = async {
            let response = self.http.send(request).await?;
            success(response.status())?;
            let mut body = Limited::new(response.into_body(), limit);
            let mut bytes = Vec::new();
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|e| Error::Body(e.to_string()))?;
                if let Ok(data) = frame.into_data() {
                    bytes.extend_from_slice(&data);
                }
            }
            Ok(bytes)
        };
        tokio::time::timeout(TIMEOUT, exchange)
            .await
            .map_err(|_| Error::Timeout)?
    }
}

#[derive(Deserialize)]
struct Page {
    total: usize,
    start: usize,
    messages: Vec<MessageId>,
}

#[derive(Deserialize)]
struct MessageId {
    #[serde(rename = "ID")]
    id: String,
}

pub struct Events {
    stream: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    closed: bool,
}

impl Events {
    pub async fn next(&mut self) -> Result<Option<String>> {
        while !self.closed {
            let Some(message) = self.read().await? else {
                self.closed = true;
                return Err(Error::UnexpectedEnd);
            };
            if message.is_ping() || message.is_close() {
                // The library queues automatic pong/close replies; flush before returning.
                tokio::time::timeout(TIMEOUT, self.stream.flush())
                    .await
                    .map_err(|_| Error::Timeout)??;
                if let Some((code, _)) = message.as_close() {
                    self.closed = true;
                    let code = u16::from(code);
                    return if matches!(code, 1000 | 1001 | 1005) {
                        Ok(None)
                    } else {
                        Err(Error::Closed(code))
                    };
                }
                continue;
            }
            if message.is_pong() {
                continue;
            }
            let text = message
                .as_text()
                .ok_or_else(|| Error::Invalid("binary websocket event".into()))?;
            let event: Event =
                serde_json::from_str(text).map_err(|e| Error::Invalid(e.to_string()))?;
            if event.kind == "new" {
                let message: MessageId = serde_json::from_value(event.data)
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                require_id(&message.id)?;
                return Ok(Some(message.id));
            }
        }
        Ok(None)
    }

    /// 读下一帧；空闲超过 [`READ_IDLE`] 先 ping 一次逼对端证明连接还在，[`PONG_GRACE`]
    /// 内等不到任何帧就按超时断开处理，交给上层立即重连。
    async fn read(&mut self) -> Result<Option<Message>> {
        let next = match tokio::time::timeout(READ_IDLE, self.stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                let probe = async {
                    self.stream.send(Message::ping(Vec::new())).await?;
                    self.stream.flush().await
                };
                tokio::time::timeout(TIMEOUT, probe)
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(Error::from)?;
                tokio::time::timeout(PONG_GRACE, self.stream.next())
                    .await
                    .map_err(|_| Error::Timeout)?
            }
        };
        next.transpose().map_err(Error::from)
    }
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default, rename = "Data")]
    data: Value,
}

fn success(status: StatusCode) -> Result<()> {
    match status {
        StatusCode::NOT_FOUND => Err(Error::NotFound),
        status if status.is_success() => Ok(()),
        status => Err(Error::Status(status)),
    }
}

fn require_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::Invalid("empty message or part ID".into()));
    }
    Ok(())
}

fn segment(value: &str) -> impl std::fmt::Display + '_ {
    utf8_percent_encode(value, NON_ALPHANUMERIC)
}

fn message_path(id: &str) -> Result<String> {
    require_id(id)?;
    Ok(format!("/api/v1/message/{}", segment(id)))
}

fn validate_upgrade<B>(response: &Response<B>, key: &str) -> Result<()> {
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        success(response.status())?;
        return Err(Error::Upgrade("expected HTTP 101".into()));
    }
    let has_token = |name: header::HeaderName, token: &str| {
        response.headers().get_all(name).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(token))
            })
        })
    };
    let mut digest = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    digest.update(key.as_bytes());
    digest.update(WEBSOCKET_GUID);
    let accept = STANDARD.encode(digest.finish().as_ref());
    if !has_token(header::UPGRADE, "websocket")
        || !has_token(header::CONNECTION, "upgrade")
        || response
            .headers()
            .get(header::SEC_WEBSOCKET_ACCEPT)
            .is_none_or(|value| value.as_bytes() != accept.as_bytes())
        || response
            .headers()
            .contains_key(header::SEC_WEBSOCKET_EXTENSIONS)
        || response
            .headers()
            .contains_key(header::SEC_WEBSOCKET_PROTOCOL)
    {
        return Err(Error::Upgrade("invalid websocket upgrade response".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
