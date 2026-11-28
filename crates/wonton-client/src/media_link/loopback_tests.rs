//! 媒体回环的行为测试：对面是本机的假核心，按 `If-Range` 决定回 206 还是整份，`If-None-Match` 对上回 304。

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use url::Url;
use wonton_protocol::media::WONTON_DEVICE_HEADER;

use super::api::MediaApi;
use super::loopback::{ApiSource, Loopback};

const CONTENT: &[u8] = b"0123456789";
const ETAG: &str = "\"v1\"";

/// 假核心收到的请求：路径带查询串、设备头。
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

async fn fake_core() -> (Url, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let seen = Seen::default();
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let record = record.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let record = record.clone();
                    async move { Ok::<_, Infallible>(answer(&record, &request)) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (base, seen)
}

fn answer(seen: &Seen, request: &Request<Incoming>) -> Response<Full<Bytes>> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let target = request.uri().path_and_query().unwrap().as_str().to_owned();
    seen.lock()
        .unwrap()
        .push((target, header(WONTON_DEVICE_HEADER)));
    let start = header("range")
        .and_then(|range| {
            range
                .strip_prefix("bytes=")?
                .split('-')
                .next()?
                .parse()
                .ok()
        })
        .filter(|_| header("if-range").is_none_or(|tag| tag == ETAG));
    if header("if-none-match").is_some_and(|tag| tag == ETAG) {
        let mut response = Response::new(Full::new(Bytes::new()));
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        response.headers_mut().insert("etag", ETAG.parse().unwrap());
        return response;
    }
    let mut response = match start {
        Some(start) => {
            let mut response = Response::new(Full::new(Bytes::copy_from_slice(&CONTENT[start..])));
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            response.headers_mut().insert(
                "content-range",
                format!("bytes {start}-9/10").parse().unwrap(),
            );
            response
        }
        None => Response::new(Full::new(Bytes::from_static(CONTENT))),
    };
    response.headers_mut().insert("etag", ETAG.parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "private, max-age=60".parse().unwrap());
    response
}

fn source(base: Url) -> ApiSource {
    Arc::new(move || {
        Ok(MediaApi::new(
            base.clone(),
            reqwest::Client::builder().no_proxy().build().unwrap(),
            "this-device".into(),
        ))
    })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn a_request_without_the_token_is_refused() {
    let (base, seen) = fake_core().await;
    let loopback = Loopback::new();
    let prefix = loopback.prefix(source(base)).await.unwrap();
    let url = Url::parse(&prefix).unwrap();
    let origin = format!("http://{}", url.authority());
    for path in [
        "/media/sessions/s/stream",
        "/not-the-token/media/sessions/s/stream",
        "/",
    ] {
        let response = client()
            .get(format!("{origin}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ranges_reach_the_core_and_come_back_partial() {
    let (base, seen) = fake_core().await;
    let loopback = Loopback::new();
    let prefix = loopback.prefix(source(base)).await.unwrap();
    let response = client()
        .get(format!("{prefix}/sessions/s1/stream?quality=hi"))
        .header("range", "bytes=6-")
        .header("if-range", ETAG)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 6-9/10");
    assert_eq!(response.headers()["etag"], ETAG);
    assert_eq!(response.bytes().await.unwrap(), &b"6789"[..]);

    // 内容换过了（`If-Range` 对不上）：核心回整份，回环原样给。
    let whole = client()
        .get(format!("{prefix}/sessions/s1/stream"))
        .header("range", "bytes=6-")
        .header("if-range", "\"old\"")
        .send()
        .await
        .unwrap();
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(whole.headers()["cache-control"], "private, max-age=60");
    assert_eq!(whole.bytes().await.unwrap(), CONTENT);

    // 图片加载器带着缓存的 ETag 来：核心回 304，回环原样给。
    let cached = client()
        .get(format!("{prefix}/items/a1/artwork?kind=photo"))
        .header("if-none-match", ETAG)
        .send()
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].0,
        "/_wonton/api/media/sessions/s1/stream?quality=hi"
    );
    assert!(
        seen.iter()
            .all(|(_, device)| device.as_deref() == Some("this-device"))
    );
}

#[tokio::test]
async fn closing_gives_the_port_back() {
    let (base, _seen) = fake_core().await;
    let loopback = Loopback::new();
    let prefix = loopback.prefix(source(base)).await.unwrap();
    let addr = Url::parse(&prefix).unwrap().socket_addrs(|| None).unwrap()[0];
    loopback.close().await;
    TcpListener::bind(addr).await.unwrap();
}
