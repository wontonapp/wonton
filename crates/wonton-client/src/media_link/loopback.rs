//! 媒体的本机回环（ADR-0113 第 2 节“字节路径”第 1 步）：系统播放器、后台传输给不了单个请求设代理，也带不了
//! 设备头，所以它们只连 `127.0.0.1`，这里再经隧道转给核心的 `/_wonton/api/media/…`。
//!
//! 同机的其他 App 也连得上回环地址，路径第一段是每次启动换一个的随机令牌，对不上回 401。只转字节要的那几个
//! 头（`Range`、`If-Range`、`Content-Range`……），设备头由 [`MediaApi`] 加；Cookie 之类一概不带。
//! 第一次要地址时才开始监听；接收循环停了（iOS 挂起久了端口会失效）下次要地址时重绑，端口可能变。

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt, TryStreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, BodyStream, Empty};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::HeaderName;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use wonton_protocol::media::api::MEDIA_API_PREFIX;
use wonton_protocol::media::{MediaFailure, MediaFailureCode};

use super::api::{MediaApi, failure};

type Body = BoxBody<Bytes, std::io::Error>;

/// 这次要连核心时用哪个客户端：连着的服务器换了、代理换了，每个请求现取。
pub(crate) type ApiSource = Arc<dyn Fn() -> Result<MediaApi, MediaFailure> + Send + Sync>;

/// 带给核心的请求头。
const PASS_REQUEST: [&str; 8] = [
    "range",
    "if-range",
    "if-none-match",
    "if-modified-since",
    "content-range",
    "content-length",
    "content-type",
    "accept",
];

/// 带回给播放器的响应头。
const PASS_RESPONSE: [&str; 8] = [
    "cache-control",
    "content-type",
    "content-length",
    "content-range",
    "accept-ranges",
    "etag",
    "last-modified",
    "retry-after",
];

pub(crate) struct Loopback {
    token: String,
    /// 异步锁，绑定时也拿着：两处同时要地址时，后绑的不能把先给出去的那个端口关掉。
    running: tokio::sync::Mutex<Option<Running>>,
}

struct Running {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Loopback {
    pub(crate) fn new() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("OS random source is unavailable");
        Self {
            token: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
            running: tokio::sync::Mutex::new(None),
        }
    }

    /// 回环前缀，后面接核心媒体接口 `/_wonton/api/media` 之后的那段，如 `{前缀}/sessions/{id}/stream`。
    pub(crate) async fn prefix(&self, api: ApiSource) -> Result<String, MediaFailure> {
        let mut running = self.running.lock().await;
        if let Some(running) = running.as_ref()
            && !running.task.is_finished()
        {
            return Ok(self.prefix_at(running.addr));
        }
        let local = |e: std::io::Error| {
            failure(
                MediaFailureCode::LocalIo,
                format!("本机回环地址开不了：{e}"),
            )
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(local)?;
        let addr = listener.local_addr().map_err(local)?;
        let task = tokio::spawn(accept(
            listener,
            Arc::new(Shared {
                token: self.token.clone(),
                api,
            }),
        ));
        *running = Some(Running { addr, task });
        Ok(self.prefix_at(addr))
    }

    fn prefix_at(&self, addr: SocketAddr) -> String {
        format!("http://{addr}/{}/media", self.token)
    }

    /// 关掉监听，端口还给系统。在放的播放器的连接一起断。生产里随 [`Drop`] 关，测试要等它关完。
    #[cfg(test)]
    pub(crate) async fn close(&self) {
        if let Some(running) = self.running.lock().await.take() {
            running.task.abort();
            let _ = running.task.await;
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        if let Some(running) = self.running.get_mut() {
            running.task.abort();
        }
    }
}

struct Shared {
    token: String,
    api: ApiSource,
}

/// 单次接收失败时接着听：对方连上又走了，或者描述符一时用完了。其他错误才算地址失效。
fn transient(error: &std::io::Error) -> bool {
    const ENFILE: i32 = 23;
    const EMFILE: i32 = 24;
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset
    ) || matches!(error.raw_os_error(), Some(ENFILE | EMFILE))
}

async fn accept(listener: TcpListener, shared: Arc<Shared>) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) if transient(&e) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => {
                tracing::info!(error = %e, "媒体回环地址不能用了，下次要地址时重开");
                return;
            }
        };
        let shared = shared.clone();
        // 连接挂在这个任务底下：关掉回环时一起断，端口不被在放的播放器占着。
        connections.spawn(async move {
            let service = service_fn(move |request| {
                let shared = shared.clone();
                async move { Ok::<_, Infallible>(respond(&shared, request).await) }
            });
            let connection = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            if let Err(e) = connection.await {
                tracing::debug!(error = %e, "媒体回环的一条连接结束");
            }
        });
        while connections.try_join_next().is_some() {}
    }
}

fn status(code: StatusCode) -> Response<Body> {
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed());
    *response.status_mut() = code;
    response
}

/// 比较时不提前返回：同机的其他 App 不能靠响应时间一个字符一个字符地猜令牌。
fn same(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// 路径去掉令牌和 `/media` 之后的那段（带查询串）；令牌对不上为空。
fn core_path(token: &str, request: &Request<Incoming>) -> Option<String> {
    let path = request.uri().path().strip_prefix('/')?;
    let (given, rest) = path.split_once('/')?;
    if !same(given, token) {
        return None;
    }
    let rest = rest.strip_prefix("media")?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let query = request
        .uri()
        .query()
        .map_or_else(String::new, |q| format!("?{q}"));
    Some(format!("{MEDIA_API_PREFIX}{rest}{query}"))
}

async fn respond(shared: &Shared, request: Request<Incoming>) -> Response<Body> {
    let Some(path) = core_path(&shared.token, &request) else {
        return status(StatusCode::UNAUTHORIZED);
    };
    let api = match (shared.api)() {
        Ok(api) => api,
        Err(failure) => {
            tracing::info!(code = ?failure.code, error = %failure.message, "媒体回环连不上家里的服务器");
            return status(StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    let (parts, body) = request.into_parts();
    let mut forwarded = api.request(parts.method.clone(), &path);
    for name in PASS_REQUEST {
        if let Some(value) = parts.headers.get(name) {
            forwarded = forwarded.header(name, value.as_bytes());
        }
    }
    // 取字节的请求不带体：给 GET 套一个流体会变成分块编码，有的服务端会拒。
    if parts.method != hyper::Method::GET && parts.method != hyper::Method::HEAD {
        let stream =
            BodyStream::new(body).try_filter_map(|frame| async move { Ok(frame.into_data().ok()) });
        forwarded = forwarded.body(reqwest::Body::wrap_stream(stream));
    }
    let upstream = match forwarded.send().await {
        Ok(upstream) => upstream,
        Err(e) => {
            tracing::info!(error = %e, "媒体回环转给核心时断了");
            return status(StatusCode::BAD_GATEWAY);
        }
    };
    let code = upstream.status();
    let headers = upstream.headers().clone();
    let frames = upstream
        .bytes_stream()
        .map(|chunk| chunk.map(Frame::data).map_err(std::io::Error::other));
    let mut response = Response::new(BodyExt::boxed(http_body_util::StreamBody::new(frames)));
    *response.status_mut() = code;
    let out = response.headers_mut();
    for name in PASS_RESPONSE {
        if let Some(value) = headers.get(name) {
            out.insert(HeaderName::from_static(name), value.clone());
        }
    }
    response
}
