//! 核心对内的 HTTP 客户端：插件流量和受核心网络限制的系统服务流量使用独立连接池。

use std::time::Duration;

use axum::body::Body;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
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
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        connector.set_nodelay(true);
        let inner = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(60))
            .build(FlushConnector(connector));
        Self {
            inner: Pool::General(inner),
        }
    }

    fn with_system_connector(connector: SystemConnector) -> Self {
        let inner = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(60))
            .build(FlushConnector(connector));
        Self {
            inner: Pool::System(inner),
        }
    }

    /// 原样发送请求（网关转发用），不设整体超时：长连接和流式响应由上游决定何时结束。
    pub async fn send(&self, request: Request<Body>) -> Result<Response<Incoming>, HttpError> {
        let result = match &self.inner {
            Pool::General(client) => client.request(request).await,
            Pool::System(client) => client.request(request).await,
        };
        result.map_err(|e| {
            let mut cause: &dyn std::error::Error = &e;
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
                    None => return HttpError::Connect(error_chain(&e)),
                }
            }
        })
    }

    /// 只限制请求体写入上游之后的响应头等待，不限制上传或响应体。
    pub(crate) async fn send_with_header_deadline(
        &self,
        request: Request<Body>,
        limit: Duration,
    ) -> Result<Response<Incoming>, HttpError> {
        let (request, deadline) = ResponseDeadline::tcp(request, limit);
        deadline.wait(self.send(request)).await
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
