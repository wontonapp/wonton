//! 总线连插件用的 HTTP 客户端：响应体边读边限制大小。
//!
//! rmcp 自带的 reqwest 客户端把 JSON 响应整个读进内存再解析，插件回几 GB 核心就没了，整个家庭服务器跟着挂（rmcp 3.8.0 只限制
//! 了 SSE 的单个事件）。所以 POST 自己发。不实现 `server/discover` 的回退：总线握手用 `initialize`（[`super::upstream`]）。

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{StreamExt, future, stream};
use hyper::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue};
use rmcp::model::{
    ClientJsonRpcMessage, ClientRequest, ErrorCode, ErrorData, JsonRpcMessage, RequestId,
    ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
    EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use serde_json::Value;
use sse_stream::{Error as SseError, Sse, SseStream};

pub(super) type HttpError = StreamableHttpError<reqwest::Error>;

/// 响应超过上限时给调用方的 JSON-RPC 错误码（实现自定义的范围）。传输层的错误到不了调用方：rmcp 把 SSE 中途的错误一律变成
/// `TransportClosed`，只有带请求 id 的回答才一定回到发起这个请求的调用，别的调用不受影响，也不用断开连接。插件自己回这个码，
/// 只影响它自己那次调用的分类。
pub(super) const RESPONSE_TOO_LARGE: ErrorCode = ErrorCode(-32090);
/// 插件对这一个请求的回答不是 MCP：非 2xx 又没有 JSON-RPC 错误、`Content-Type` 不对、JSON 解析不了。理由同上，而且这是插件的
/// 问题：连接本身没事，错误要和内网地址回 500 一样算 `upstream_error`。
const NOT_MCP: ErrorCode = ErrorCode(-32091);
/// 错误响应最多读这么多：JSON-RPC 错误不会更大，再大的不是错误，是别的东西。
const MAX_ERROR_BODY_BYTES: usize = 1 << 20;
/// 错误消息最多带这么多原话。
const MAX_ERROR_WORDS: usize = 2 << 10;
/// 错误响应的响应体最多等这么久：状态码已经拿到了，原话只是附带的。
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);
/// `tools/list` 的一页最多这么多字节，比别的响应小得多：每个 Agent 都会列所有插件的工具，同时在内存里（[`super::upstream`]
/// 还限制了总量）。
pub(super) const MAX_TOOLS_BYTES: usize = 8 << 20;

#[derive(Clone)]
pub(super) struct BoundedHttp {
    pub http: reqwest::Client,
    /// 一个响应最多这么多字节。
    pub limit: usize,
    pub diagnostics: super::diagnostics::Diagnostics,
}

/// 超过上限时传输层知道多少：收到过的字节（`read`，收到即算，不一定读完）、插件声明的大小（`declared`，这时一个字节没读）。
/// [`too_large_reply`] 把它写进错误 `data`，[`super::upstream`] 读出来给 Agent 说明“至少读到了多少”。
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct TooLargeDetail {
    pub limit: Option<u64>,
    pub read: Option<u64>,
    pub declared: Option<u64>,
}

/// 传输层写在错误 `data` 里的详情（[`too_large_reply`]）。插件自己回这个错误码时没有或者不合规矩，字段是 `None`，
/// 调用方用自己知道的上限兜底。
pub(super) fn too_large_detail(data: Option<&serde_json::Value>) -> TooLargeDetail {
    let Some(Value::Object(fields)) = data else {
        return TooLargeDetail::default();
    };
    let field = |key: &str| fields.get(key).and_then(Value::as_u64);
    TooLargeDetail {
        limit: field("limit"),
        read: field("read"),
        declared: field("declared"),
    }
}

/// 响应体的读取结果：读完了，或者超过上限停下（带 [`TooLargeDetail`] 的字段）。
enum Body {
    Complete(Vec<u8>),
    TooLarge { read: u64, declared: Option<u64> },
}

fn too_large_reply(
    limit: usize,
    request: RequestId,
    read: Option<u64>,
    declared: Option<u64>,
) -> ServerJsonRpcMessage {
    let why = format!("response exceeds {limit} bytes");
    let mut detail = serde_json::Map::new();
    detail.insert("limit".to_owned(), serde_json::json!(limit as u64));
    if let Some(read) = read {
        detail.insert("read".to_owned(), serde_json::json!(read));
    }
    if let Some(declared) = declared {
        detail.insert("declared".to_owned(), serde_json::json!(declared));
    }
    ServerJsonRpcMessage::error(
        ErrorData::new(RESPONSE_TOO_LARGE, why, Some(detail.into())),
        Some(request),
    )
}

/// 普通 JSON 响应超限：通知没有 id，也没人等回答，算传输错误。
fn too_large(
    limit: usize,
    message: &ClientJsonRpcMessage,
    session: Option<String>,
    read: Option<u64>,
    declared: Option<u64>,
) -> Result<StreamableHttpPostResponse, HttpError> {
    match request_id(message) {
        Some(id) => Ok(StreamableHttpPostResponse::Json(
            too_large_reply(limit, id, read, declared),
            session,
        )),
        None => Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
            format!("response exceeds {limit} bytes"),
        ))),
    }
}

impl BoundedHttp {
    async fn send_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let accept = format!("{EVENT_STREAM_MIME_TYPE}, {JSON_MIME_TYPE}");
        let mut request = self.http.post(uri.as_ref()).header(ACCEPT, accept);
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        for (name, value) in custom_headers {
            request = request.header(name, value);
        }
        let had_session = session_id.is_some();
        if let Some(session) = session_id {
            request = request.header(HEADER_SESSION_ID, session.as_ref());
        }
        let response = request.json(&message).send().await?;
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && had_session {
            return Err(StreamableHttpError::SessionExpired);
        }
        let session = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
        let expects_reply = matches!(message, ClientJsonRpcMessage::Request(_));
        let limit = self.limit_for(&message);
        // 规范要求回 202，有的服务回空的 200。
        if status.is_success() && !expects_reply && response.content_length() == Some(0) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if !status.is_success() {
            return self.error_status(response, &message, session).await;
        }
        match content_type.as_deref() {
            Some(kind) if kind.starts_with(EVENT_STREAM_MIME_TYPE) => {
                let events = self.events(response, request_id(&message), limit);
                Ok(StreamableHttpPostResponse::Sse(events, session))
            }
            Some(kind) if kind.starts_with(JSON_MIME_TYPE) => {
                match self.read_body(response, limit).await? {
                    Body::Complete(body) => {
                        match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                            Ok(parsed)
                                if request_id(&message)
                                    .is_some_and(|id| !answers(&parsed, &id)) =>
                            {
                                let why = format!(
                                    "reply does not answer this request: {}",
                                    self.diagnostics.words(&body, MAX_ERROR_WORDS)
                                );
                                not_mcp(&message, session, why)
                            }
                            Ok(parsed) => Ok(StreamableHttpPostResponse::Json(parsed, session)),
                            Err(_) if !expects_reply => Ok(StreamableHttpPostResponse::Accepted),
                            Err(e) => not_mcp(
                                &message,
                                session,
                                format!(
                                    "can't parse the response: {e}: {}",
                                    self.diagnostics.words(&body, MAX_ERROR_WORDS)
                                ),
                            ),
                        }
                    }
                    Body::TooLarge { read, declared } => {
                        too_large(limit, &message, session, Some(read), declared)
                    }
                }
            }
            other => {
                let why = format!("unexpected Content-Type: {other:?}");
                not_mcp(&message, session, why)
            }
        }
    }

    fn limit_for(&self, message: &ClientJsonRpcMessage) -> usize {
        match message {
            JsonRpcMessage::Request(request)
                if matches!(request.request, ClientRequest::ListToolsRequest(_)) =>
            {
                self.limit.min(MAX_TOOLS_BYTES)
            }
            _ => self.limit,
        }
    }

    /// 读整个响应体，超过 `limit` 时停下，带收到过多少、声明了多少。
    async fn read_body(
        &self,
        mut response: reqwest::Response,
        limit: usize,
    ) -> Result<Body, HttpError> {
        let declared = response.content_length().unwrap_or(0);
        if usize::try_from(declared).map_or(true, |declared| declared > limit) {
            return Ok(Body::TooLarge {
                read: 0,
                declared: usize::try_from(declared)
                    .ok()
                    .map(|declared| declared as u64),
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > limit {
                return Ok(Body::TooLarge {
                    read: (body.len() + chunk.len()) as u64,
                    declared: None,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Body::Complete(body))
    }

    /// 非 2xx 的回答。带这个请求的 JSON-RPC 错误就是那个错误，否则是这个请求的 HTTP 错误；响应体太大或者等不到都不影响这个结论。
    /// 读的时候连接断了才是传输错误。
    async fn error_status(
        &self,
        response: reqwest::Response,
        message: &ClientJsonRpcMessage,
        session: Option<String>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let status = response.status();
        let limit = self.limit.min(MAX_ERROR_BODY_BYTES);
        let read = self.read_body(response, limit);
        let body = match tokio::time::timeout(ERROR_BODY_TIMEOUT, read).await {
            Ok(Ok(Body::Complete(body))) => body,
            Ok(Ok(Body::TooLarge { .. })) | Ok(Err(_)) | Err(_) => Vec::new(),
        };
        if let Ok(error @ JsonRpcMessage::Error(_)) = serde_json::from_slice(&body)
            && request_id(message).is_some_and(|id| answers(&error, &id))
        {
            return Ok(StreamableHttpPostResponse::Json(error, session));
        }
        let mut why = format!(
            "HTTP {status}: {}",
            self.diagnostics.words(&body, MAX_ERROR_WORDS)
        );
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            // 不自动重试、也不重新取密钥：凭据不对是清单和插件的事，如实报出来。
            why.push_str("; the app's MCP server rejected the credentials");
        }
        not_mcp(message, session, why)
    }

    /// SSE 响应超限：不再读下去，流的最后是一条超限的回答。没读完的事件解析器不会交出来。
    fn events(
        &self,
        response: reqwest::Response,
        request: Option<RequestId>,
        limit: usize,
    ) -> BoxStream<'static, Result<Sse, SseError>> {
        let exceeded = Arc::new(AtomicBool::new(false));
        // 超限停下时，回答里要写收到过多少：scan 的状态跟着流结束就没了，用共享的计数器留给最后的回答。
        let received = Arc::new(AtomicU64::new(0));
        let counted = received.clone();
        let over = exceeded.clone();
        let bytes = response.bytes_stream().scan((), move |_, chunk| {
            if let Ok(chunk) = &chunk {
                let seen =
                    counted.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
                if seen > limit as u64 {
                    over.store(true, Ordering::Relaxed);
                    return future::ready(None);
                }
            }
            future::ready(Some(chunk.map_err(io::Error::other)))
        });
        let last = stream::once(future::ready(())).filter_map(move |()| {
            let reply = exceeded
                .load(Ordering::Relaxed)
                .then(|| {
                    request.as_ref().map(|id| {
                        too_large_reply(
                            limit,
                            id.clone(),
                            Some(received.load(Ordering::Relaxed)),
                            None,
                        )
                    })
                })
                .flatten()
                .map(|reply| Ok(event_of(&reply)));
            future::ready(reply)
        });
        SseStream::from_bytes_stream(bytes).chain(last).boxed()
    }
}

fn request_id(message: &ClientJsonRpcMessage) -> Option<RequestId> {
    match message {
        ClientJsonRpcMessage::Request(request) => Some(request.id.clone()),
        _ => None,
    }
}

/// 这条回答是不是给这个请求的。rmcp 丢掉没有 id 的错误，id 不对的会送给别的调用，这个请求就永远等不到回答：supergateway 换会话
/// 后回 `{"error": …, "id": null}`，MCP TS SDK 的 express 处理 500 时也这样。
fn answers(reply: &ServerJsonRpcMessage, request: &RequestId) -> bool {
    match reply {
        JsonRpcMessage::Response(response) => same_id(request, &response.id),
        JsonRpcMessage::Error(error) => error.id.as_ref().is_some_and(|id| same_id(request, id)),
        _ => false,
    }
}

/// 和 rmcp 3.8 配回答的规则一样（`matches_response_id`，它不导出）：数字 id 的字符串形式也算，`"0"` 就是 0。
fn same_id(request: &RequestId, reply: &RequestId) -> bool {
    if request == reply {
        return true;
    }
    match (request, reply) {
        (RequestId::Number(number), RequestId::String(text)) => {
            text.parse::<i64>().is_ok_and(|parsed| parsed == *number)
        }
        _ => false,
    }
}

/// 回答不是 MCP：归到这个请求头上（带 id 的回答只回到发起它的调用）。通知没人等回答，算传输错误。
fn not_mcp(
    message: &ClientJsonRpcMessage,
    session: Option<String>,
    why: String,
) -> Result<StreamableHttpPostResponse, HttpError> {
    match request_id(message) {
        Some(id) => Ok(StreamableHttpPostResponse::Json(
            ServerJsonRpcMessage::error(ErrorData::new(NOT_MCP, why, None), Some(id)),
            session,
        )),
        None => Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
            why,
        ))),
    }
}

fn event_of(message: &ServerJsonRpcMessage) -> Sse {
    Sse {
        event: None,
        data: serde_json::to_string(message).ok(),
        id: None,
        retry: None,
    }
}

impl StreamableHttpClient for BoundedHttp {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        self.diagnostics.post(
            self.send_message(uri, message, session_id, auth_header, custom_headers)
                .await,
        )
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), HttpError> {
        self.http
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
            .map_err(|error| self.diagnostics.error(error))
    }

    /// 服务器主动发消息的长连接：rmcp 按事件限制大小。
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, HttpError> {
        self.http
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                auth_header,
                custom_headers,
                self.limit,
            )
            .await
            .map(|events| self.diagnostics.events(events))
            .map_err(|error| self.diagnostics.error(error))
    }
}
