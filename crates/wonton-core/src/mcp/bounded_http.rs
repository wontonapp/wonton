//! 总线连插件用的 HTTP 客户端：响应体边读边限制大小。
//!
//! rmcp 自带的 reqwest 客户端把 JSON 响应整个读进内存再解析，插件回几 GB 核心就没了，整个家庭服务器跟着挂（rmcp 3.8.0 只限制
//! 了 SSE 的单个事件）。所以 POST 自己发。不实现 `server/discover` 的回退：总线握手用 `initialize`（[`super::upstream`]）。

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

fn too_large_reply(limit: usize, request: RequestId) -> ServerJsonRpcMessage {
    let why = format!("响应超过 {limit} 字节");
    ServerJsonRpcMessage::error(ErrorData::new(RESPONSE_TOO_LARGE, why, None), Some(request))
}

/// 普通 JSON 响应超限：通知没有 id，也没人等回答，算传输错误。
fn too_large(
    limit: usize,
    message: &ClientJsonRpcMessage,
    session: Option<String>,
) -> Result<StreamableHttpPostResponse, HttpError> {
    match request_id(message) {
        Some(id) => Ok(StreamableHttpPostResponse::Json(
            too_large_reply(limit, id),
            session,
        )),
        None => Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
            format!("响应超过 {limit} 字节"),
        ))),
    }
}

impl BoundedHttp {
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

    /// 读整个响应体，超过 `limit` 时为空。
    async fn read_body(
        &self,
        mut response: reqwest::Response,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, HttpError> {
        let declared = response.content_length().unwrap_or(0);
        if usize::try_from(declared).map_or(true, |declared| declared > limit) {
            return Ok(None);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > limit {
                return Ok(None);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
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
            Ok(body) => body?.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        if let Ok(error @ JsonRpcMessage::Error(_)) = serde_json::from_slice(&body)
            && request_id(message).is_some_and(|id| answers(&error, &id))
        {
            return Ok(StreamableHttpPostResponse::Json(error, session));
        }
        not_mcp(message, session, format!("HTTP {status}：{}", words(&body)))
    }

    /// SSE 响应超限：不再读下去，流的最后是一条超限的回答。没读完的事件解析器不会交出来。
    fn events(
        &self,
        response: reqwest::Response,
        request: Option<RequestId>,
        limit: usize,
    ) -> BoxStream<'static, Result<Sse, SseError>> {
        let exceeded = Arc::new(AtomicBool::new(false));
        let over = exceeded.clone();
        let bytes = response.bytes_stream().scan(0usize, move |seen, chunk| {
            if let Ok(chunk) = &chunk {
                *seen = seen.saturating_add(chunk.len());
                if *seen > limit {
                    over.store(true, Ordering::Relaxed);
                    return future::ready(None);
                }
            }
            future::ready(Some(chunk.map_err(io::Error::other)))
        });
        let reply = request.map(|id| too_large_reply(limit, id));
        let last = stream::once(future::ready(())).filter_map(move |()| {
            let reply = reply
                .clone()
                .filter(|_| exceeded.load(Ordering::Relaxed))
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

/// 响应体的开头，给错误消息用。
fn words(body: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_WORDS)])
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
                let Some(body) = self.read_body(response, limit).await? else {
                    return too_large(limit, &message, session);
                };
                match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                    Ok(parsed) if request_id(&message).is_some_and(|id| !answers(&parsed, &id)) => {
                        let why = format!("回答对不上这个请求：{}", words(&body));
                        not_mcp(&message, session, why)
                    }
                    Ok(parsed) => Ok(StreamableHttpPostResponse::Json(parsed, session)),
                    Err(_) if !expects_reply => Ok(StreamableHttpPostResponse::Accepted),
                    Err(e) => not_mcp(
                        &message,
                        session,
                        format!("读不懂响应：{e}：{}", words(&body)),
                    ),
                }
            }
            other => {
                let why = format!("响应的 Content-Type 不对：{other:?}");
                not_mcp(&message, session, why)
            }
        }
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
    }
}
