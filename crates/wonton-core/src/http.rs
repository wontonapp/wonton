//! 核心对内的 HTTP 客户端：插件流量和受核心网络限制的系统服务流量使用独立连接池。

use std::collections::HashSet;
use std::error::Error as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use http_body_util::{BodyExt, Limited};
use hyper::body::{Body as HttpBody, Incoming};
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

pub(crate) mod deadline;
use deadline::{FlushConnector, ResponseDeadline};
mod system;
use system::SystemConnector;

/// JSON 响应体的上限，防止异常的上游把核心内存撑爆。
const MAX_JSON_BYTES: usize = 1 << 20;

/// 核心自己访问插件和外部服务的池的空闲期。
const POOL_IDLE: Duration = Duration::from_secs(60);

/// 网关转发池的空闲期：比常见上游的 keep-alive 都短——gunicorn 默认 2 秒、Node 5 秒、
/// uwsgi 60 秒、nginx 75 秒——空闲连接一直由我们先关，上游关掉的就不会是刚发出去的请求。
/// 重发（[`HttpClient::send_forwarded`]）只是这道之后的保险，不是主要手段。
pub(crate) const FORWARD_POOL_IDLE: Duration = Duration::from_secs(1);

/// 记住当前最多多少条连接被用过，超了整批忘掉（忘掉的连接按新连接算，只影响要不要重发）。
const TRACKED_CONNECTIONS: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("请求地址不合法：{0}")]
    BadUrl(String),
    #[error("连接失败：{0}")]
    Connect(String),
    #[error(
        "系统服务 {name} 的名字解析到了核心网络以外的地址：{addresses:?}；请检查系统容器和 DNS 配置"
    )]
    SystemResolution {
        name: String,
        addresses: Vec<std::net::SocketAddr>,
    },
    #[error("请求超时（{0:?}）")]
    Timeout(Duration),
    #[error("上游迟迟没有回应：请求体发送完成后 {0:?} 内没有响应头")]
    ResponseHeadersTimeout(Duration),
    #[error("响应体读取失败：{0}")]
    Body(String),
    #[error("响应不是合法的 JSON：{0}")]
    Json(String),
}

/// [`HttpClient::request_json`] 的请求体。
pub enum RequestBody<'a> {
    Empty,
    Json(&'a Value),
    Bytes {
        content_type: &'a str,
        data: Vec<u8>,
    },
}

#[derive(Clone)]
pub struct HttpClient {
    inner: Pool,
    connections: Connections,
}

/// 每条新建连接的编号。hyper-util 不对外说一条连接是不是从池里复用的，只能自己记：
/// 编号挂在连接元数据里，成功（响应扩展）和失败（`Error::connect_info`）两边都读得到，
/// 请求成功过的编号就是复用的连接。
#[derive(Clone, Default)]
struct Connections(Arc<Mutex<Tracked>>);

#[derive(Default)]
struct Tracked {
    next: u64,
    used: HashSet<u64>,
}

#[derive(Clone, Copy)]
struct ConnTag(u64);

impl Connections {
    fn next(&self) -> ConnTag {
        let mut tracked = self.0.lock().unwrap();
        tracked.next += 1;
        ConnTag(tracked.next)
    }

    fn mark_used(&self, tag: ConnTag) {
        let mut tracked = self.0.lock().unwrap();
        if tracked.used.len() >= TRACKED_CONNECTIONS {
            tracked.used.clear();
        }
        tracked.used.insert(tag.0);
    }

    fn is_used(&self, tag: ConnTag) -> bool {
        self.0.lock().unwrap().used.contains(&tag.0)
    }
}

#[derive(Clone)]
enum Pool {
    General(Client<FlushConnector<HttpConnector>, Body>),
    System(Client<FlushConnector<SystemConnector>, Body>),
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        Self::general(POOL_IDLE)
    }

    /// 网关转发插件流量的池，空闲期见 [`FORWARD_POOL_IDLE`]。
    pub(crate) fn for_forwarding() -> Self {
        Self::general(FORWARD_POOL_IDLE)
    }

    fn general(idle: Duration) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        connector.set_nodelay(true);
        let connections = Connections::default();
        let inner = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(idle)
            .build(FlushConnector::new(connector, connections.clone()));
        Self {
            inner: Pool::General(inner),
            connections,
        }
    }

    fn with_system_connector(connector: SystemConnector) -> Self {
        let connections = Connections::default();
        let inner = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(POOL_IDLE)
            .build(FlushConnector::new(connector, connections.clone()));
        Self {
            inner: Pool::System(inner),
            connections,
        }
    }

    /// 原样发送请求（网关转发用），不设整体超时：长连接和流式响应由上游决定何时结束。
    pub async fn send(&self, request: Request<Body>) -> Result<Response<Incoming>, HttpError> {
        self.send_typed(request)
            .await
            .map_err(SendFailure::into_error)
    }

    /// 网关转发用，`limit` 是响应头期限（`None` 时不设，见 [`Self::send`]）。
    ///
    /// 从池里复用的连接被上游关掉、请求又还没拿到响应时，空请求体重发一次，和 Go 的
    /// `Transport.shouldRetryRequest`、nginx 的 `proxy_next_upstream` 一样：只有幂等请求
    /// （GET、HEAD、OPTIONS、TRACE，或带 `Idempotency-Key` 头）在没有请求体、复用的连接上、
    /// 一个字节都没写出去或对端在回第一个响应字节之前关掉连接时才重发，其余照旧报错。
    /// 重试的新请求走新连接，不会再被重发第二次。
    pub(crate) async fn send_forwarded(
        &self,
        request: Request<Body>,
        limit: Option<Duration>,
    ) -> Result<Response<Incoming>, HttpError> {
        let replayable =
            request.body().is_end_stream() && is_idempotent(request.method(), request.headers());
        if !replayable {
            return self
                .send_once(request, limit)
                .await
                .map_err(SendFailure::into_error);
        }
        let (parts, _) = request.into_parts();
        let rebuild = || Request::from_parts(parts.clone(), Body::empty());
        let first = self.send_once(rebuild(), limit).await;
        match first {
            Err(failure) if failure.replayable() => self
                .send_once(rebuild(), limit)
                .await
                .map_err(SendFailure::into_error),
            other => other.map_err(SendFailure::into_error),
        }
    }

    async fn send_once(
        &self,
        request: Request<Body>,
        limit: Option<Duration>,
    ) -> Result<Response<Incoming>, SendFailure> {
        let Some(limit) = limit else {
            return self.send_typed(request).await;
        };
        let (request, deadline) = ResponseDeadline::tcp(request, limit);
        let sending = async { self.send_typed(request).await };
        deadline
            .wait(
                sending,
                SendFailure::from(HttpError::ResponseHeadersTimeout(limit)),
            )
            .await
    }

    async fn send_typed(&self, request: Request<Body>) -> Result<Response<Incoming>, SendFailure> {
        let result = match &self.inner {
            Pool::General(client) => client.request(request).await,
            Pool::System(client) => client.request(request).await,
        };
        match result {
            Ok(response) => {
                if let Some(tag) = response.extensions().get::<ConnTag>() {
                    self.connections.mark_used(*tag);
                }
                Ok(response)
            }
            Err(error) => Err(SendFailure {
                reused: connection_tag(&error).is_some_and(|tag| self.connections.is_used(tag)),
                stale_connection: is_stale_connection(&error),
                error: send_error(&error),
            }),
        }
    }

    pub async fn get_status(&self, url: &str, timeout: Duration) -> Result<u16, HttpError> {
        let request = Request::get(url)
            .body(Body::empty())
            .map_err(|e| HttpError::BadUrl(e.to_string()))?;
        let response = tokio::time::timeout(timeout, self.send(request))
            .await
            .map_err(|_| HttpError::Timeout(timeout))??;
        Ok(response.status().as_u16())
    }

    /// 发送 JSON 请求并解析 JSON 响应（响应为空时得到 `Value::Null`）。
    pub async fn json(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(StatusCode, Value), HttpError> {
        let body = body.map_or(RequestBody::Empty, RequestBody::Json);
        self.request_json(method, url, None, body, timeout).await
    }

    /// 同 [`Self::json`]，可以带访问令牌（`Authorization: Bearer`），请求体也可以是原始字节（如上传文件）。
    pub async fn request_json(
        &self,
        method: Method,
        url: &str,
        bearer: Option<&str>,
        body: RequestBody<'_>,
        timeout: Duration,
    ) -> Result<(StatusCode, Value), HttpError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(url)
            .header(header::ACCEPT, "application/json");
        if let Some(token) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            RequestBody::Empty => builder.body(Body::empty()),
            RequestBody::Json(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            RequestBody::Bytes { content_type, data } => builder
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(data)),
        }
        .map_err(|e| HttpError::BadUrl(e.to_string()))?;
        let exchange = async {
            let response = self.send(request).await?;
            let status = response.status();
            let bytes = Limited::new(response.into_body(), MAX_JSON_BYTES)
                .collect()
                .await
                .map_err(|e| HttpError::Body(e.to_string()))?
                .to_bytes();
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).map_err(|e| HttpError::Json(e.to_string()))?
            };
            Ok((status, value))
        };
        tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| HttpError::Timeout(timeout))?
    }
}

/// 一次发送失败，外加重发要的两个判断：错误是不是连接被对端关掉、连接是不是从池里复用的。
/// 请求本身能不能重发（方法、有没有请求体）由调用方先判掉。
struct SendFailure {
    error: HttpError,
    stale_connection: bool,
    reused: bool,
}

impl SendFailure {
    fn into_error(self) -> HttpError {
        self.error
    }

    /// 对端在复用连接上把请求或空等着的连接关掉了。
    fn replayable(&self) -> bool {
        self.stale_connection && self.reused
    }
}

impl From<HttpError> for SendFailure {
    fn from(error: HttpError) -> Self {
        Self {
            error,
            stale_connection: false,
            reused: false,
        }
    }
}

/// Go 的 `Request.isReplayable`（src/net/http/request.go:1543）：幂等方法，或带了
/// `Idempotency-Key` / `X-Idempotency-Key` 头。没请求体才用得上，由调用方判断。
fn is_idempotent(method: &Method, headers: &hyper::HeaderMap) -> bool {
    *method == Method::GET
        || *method == Method::HEAD
        || *method == Method::OPTIONS
        || *method == Method::TRACE
        || headers.contains_key("idempotency-key")
        || headers.contains_key("x-idempotency-key")
}

/// Go 的 `shouldRetryRequest`（src/net/http/transport.go:756）里两种可重发的错误：
/// `nothingWrittenError`（`:780`，一个字节都没写出去，hyper 报 `Canceled`）和
/// `errServerClosedIdle`（`:794`，对端在回第一个字节之前关掉了空闲连接，hyper 报
/// `IncompleteMessage`）。连接没建起来的错误（`is_connect`）不是连接被关，排除。
fn is_stale_connection(error: &hyper_util::client::legacy::Error) -> bool {
    !error.is_connect()
        && error
            .source()
            .and_then(|source| source.downcast_ref::<hyper::Error>())
            .is_some_and(|error| error.is_canceled() || error.is_incomplete_message())
}

/// 错误里带的连接编号，没有就是连接还没建起来。
fn connection_tag(error: &hyper_util::client::legacy::Error) -> Option<ConnTag> {
    let mut extensions = hyper::http::Extensions::new();
    error.connect_info()?.get_extras(&mut extensions);
    extensions.get::<ConnTag>().copied()
}

/// 把 hyper 的错误映射成 `HttpError`，系统服务的解析拒绝单独保留。
fn send_error(error: &hyper_util::client::legacy::Error) -> HttpError {
    let mut cause: &dyn std::error::Error = error;
    loop {
        if let Some(HttpError::SystemResolution { name, addresses }) =
            cause.downcast_ref::<HttpError>()
        {
            return HttpError::SystemResolution {
                name: name.clone(),
                addresses: addresses.clone(),
            };
        }
        match cause.source() {
            Some(source) => cause = source,
            None => return HttpError::Connect(error_chain(error)),
        }
    }
}

/// 把错误链拼成一行，连接失败时底层原因（如 connection refused）往往在 source 里。
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push('：');
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
