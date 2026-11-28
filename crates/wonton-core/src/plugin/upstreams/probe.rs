//! 探测插件网页入口：和装完时的探测、健康详情一样请求一次，看应答。

use std::time::Duration;

use super::Endpoint;
use crate::http::HttpError;

/// 网页探测一次的时限，和装完时的探测（[`super::health::wait_webui`]）、`plugins.health` 的详情接口
/// （[`super::health_probe`]）一样。
pub(in crate::plugin) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 请求一次网页入口，返回状态码。unix 套接字走 reqwest，普通网络走共享的 HTTP 客户端。
pub(in crate::plugin) async fn status(
    http: &crate::http::HttpClient,
    endpoint: &Endpoint,
    path: &str,
    timeout: Duration,
) -> Result<u16, HttpError> {
    let Some(socket) = endpoint.socket() else {
        return http.get_status(&endpoint.url(path), timeout).await;
    };
    let client = reqwest::Client::builder()
        .unix_socket(socket.to_path_buf())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| HttpError::Connect(e.to_string()))?;
    let request = client.get(endpoint.url(path)).send();
    tokio::time::timeout(timeout, request)
        .await
        .map_err(|_| HttpError::Timeout(timeout))?
        .map(|response| response.status().as_u16())
        .map_err(|e| HttpError::Connect(e.to_string()))
}

/// 探测一次网页入口（`plugins.health` 的详情接口，规范“插件健康探测”）：请求带插件公开地址的 Host 头，
/// 像浏览器经网关访问那样，但不带任何主人身份。返回（结果，耗时）。期限 [`PROBE_TIMEOUT`] 由调用方传入。
pub(in crate::plugin) async fn probe(
    http: &crate::http::HttpClient,
    endpoint: &Endpoint,
    path: &str,
    host: &str,
    timeout: Duration,
) -> (Duration, Result<u16, HttpError>) {
    let started = std::time::Instant::now();
    let outcome = match endpoint.socket() {
        None => {
            let request = hyper::Request::get(endpoint.url(path))
                .header(hyper::header::HOST, host)
                .body(axum::body::Body::empty());
            match request {
                Ok(request) => match tokio::time::timeout(timeout, http.send(request)).await {
                    Ok(Ok(response)) => Ok(response.status().as_u16()),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(HttpError::Timeout(timeout)),
                },
                Err(e) => Err(HttpError::BadUrl(e.to_string())),
            }
        }
        Some(socket) => {
            // 和 [`status`] 一样：unix 套接字的主机名不参与解析，reqwest 按套接字连。
            let client = match reqwest::Client::builder()
                .unix_socket(socket.to_path_buf())
                .no_proxy()
                .build()
            {
                Ok(client) => client,
                Err(e) => return (started.elapsed(), Err(HttpError::Connect(e.to_string()))),
            };
            let sent = client
                .get(endpoint.url(path))
                .header(reqwest::header::HOST, host)
                .send();
            match tokio::time::timeout(timeout, sent).await {
                Ok(Ok(response)) => Ok(response.status().as_u16()),
                Ok(Err(e)) => Err(HttpError::Connect(e.to_string())),
                Err(_) => Err(HttpError::Timeout(timeout)),
            }
        }
    };
    (started.elapsed(), outcome)
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    /// 回 503 的套接字服务，核对 unix 套接字的探测也带 Host、拿得到状态码和耗时。
    #[tokio::test]
    async fn unix_socket_probe_carries_host_and_reports_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.starts_with("GET / HTTP/1.1"));
            assert!(request.contains("host: app-example-t.wonton.example"));
            let _ = stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
                .await;
        });
        let http = crate::http::HttpClient::new();
        let (elapsed, outcome) = probe(
            &http,
            &Endpoint::unix(&path),
            "/",
            "app-example-t.wonton.example",
            PROBE_TIMEOUT,
        )
        .await;
        assert_eq!(outcome.unwrap(), 503);
        assert!(elapsed.as_millis() > 0);
    }
}
