//! 核心对内的 HTTP 客户端：网关转发、健康探测、调用身份提供方的管理接口共用一个连接池。

use std::time::Duration;

use axum::body::Body;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

/// JSON 响应体的上限，防止异常的上游把核心内存撑爆。
const MAX_JSON_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("请求地址不合法：{0}")]
    BadUrl(String),
    #[error("连接失败：{0}")]
    Connect(String),
    #[error("请求超时（{0:?}）")]
    Timeout(Duration),
    #[error("响应体读取失败：{0}")]
    Body(String),
    #[error("响应不是合法的 JSON：{0}")]
    Json(String),
}

#[derive(Clone)]
pub struct HttpClient {
    inner: Client<HttpConnector, Body>,
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
            .build(connector);
        Self { inner }
    }

    /// 原样发送请求（网关转发用），不设整体超时：长连接和流式响应由上游决定何时结束。
    pub async fn send(&self, request: Request<Body>) -> Result<Response<Incoming>, HttpError> {
        self.inner
            .request(request)
            .await
            .map_err(|e| HttpError::Connect(error_chain(&e)))
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
        let builder = Request::builder()
            .method(method)
            .uri(url)
            .header(header::ACCEPT, "application/json");
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
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
