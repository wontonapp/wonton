//! 给 App 自己的播放器用的回环地址（ADR-0020 的 `file_stream_url`）：播放器不能给单个请求设代理，也带不了自定义请求头。同机的其他 App 连得上
//! 回环地址，所以路径里带一个每次启动换一个的随机令牌。

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{HeaderValue, IF_RANGE, RANGE};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, FileId};
use wonton_tunnel::ERROR_HEADER;

use super::api::Api;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

type Body = BoxBody<Bytes, std::io::Error>;

/// 转给播放器的响应头。
const FORWARDED: [&str; 6] = [
    "content-type",
    "content-length",
    "content-range",
    "accept-ranges",
    "etag",
    "x-wonton-error",
];

/// 这次播放连家里时断过哪些文件，说明是什么。播放器放不了时用：等问起来隧道可能又好了，不能说成格式不支持。
type Interruptions = Arc<Mutex<HashMap<FileId, String>>>;

pub(super) struct StreamServer {
    token: String,
    /// 异步锁，绑定时也拿着：两个地址同时要时，后绑的不能把先给出去的那个端口关掉。
    running: tokio::sync::Mutex<Option<Running>>,
    interruptions: Interruptions,
}

struct Running {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl StreamServer {
    pub(super) fn new() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("OS random source is unavailable");
        Self {
            token: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
            running: tokio::sync::Mutex::new(None),
            interruptions: Interruptions::default(),
        }
    }

    /// 第一次要地址时才开始监听。iOS 挂起久了回环上监听的端口也可能失效，所以接收循环结束后下次要地址时重新绑定，端口可能变。
    pub(super) async fn url(&self, home: &Arc<HomeLink>, id: &FileId) -> Result<String, CoreError> {
        let addr = self.ensure_running(home).await?;
        lock(&self.interruptions).remove(id);
        Ok(format!("http://{addr}/{}/files/{id}/content", self.token))
    }

    pub(super) fn take_interruption(&self, id: &FileId) -> Option<String> {
        lock(&self.interruptions).remove(id)
    }

    async fn ensure_running(&self, home: &Arc<HomeLink>) -> Result<SocketAddr, CoreError> {
        let mut running = self.running.lock().await;
        if let Some(running) = running.as_ref()
            && !running.task.is_finished()
        {
            return Ok(running.addr);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| CoreError::Storage {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "files-stream-local-address",
                    reason = e.to_string()
                ),
            })?;
        let addr = listener.local_addr().map_err(|e| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "files-stream-local-address",
                reason = e.to_string()
            ),
        })?;
        let shared = Shared {
            home: home.clone(),
            token: self.token.clone(),
            interruptions: self.interruptions.clone(),
        };
        let task = tokio::spawn(accept(listener, Arc::new(shared)));
        *running = Some(Running { addr, task });
        Ok(addr)
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        if let Some(running) = self.running.get_mut() {
            running.task.abort();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// 每个播放器的连接都要用的。
struct Shared {
    home: Arc<HomeLink>,
    token: String,
    interruptions: Interruptions,
}

impl Shared {
    fn interrupted(&self, file: &FileId, detail: String) {
        lock(&self.interruptions).insert(file.clone(), detail);
    }
}

/// 单次接收失败时接着听：对方连上又走了，或者描述符一时用完了。其他错误才算地址失效。
fn transient(error: &std::io::Error) -> bool {
    /// Linux、Android 和 Darwin 上都是这两个值。
    const ENFILE: i32 = 23;
    const EMFILE: i32 = 24;
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset
    ) || matches!(error.raw_os_error(), Some(ENFILE | EMFILE))
}

async fn accept(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) if transient(&e) => {
                tracing::info!(error = %e, "could not accept this connection on the playback address");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => {
                tracing::info!(error = %e, "playback address is no longer usable, reopening on the next request");
                return;
            }
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let shared = shared.clone();
                async move { Ok::<_, Infallible>(respond(shared, request).await) }
            });
            let connection = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            if let Err(e) = connection.await {
                tracing::debug!(error = %e, "player connection ended");
            }
        });
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

fn requested_file(token: &str, path: &str) -> Option<FileId> {
    let mut parts = path.strip_prefix('/')?.split('/');
    let given = parts.next()?;
    if !same(given, token) || parts.next()? != "files" {
        return None;
    }
    let id = parts.next()?.parse().ok()?;
    (parts.next()? == "content" && parts.next().is_none()).then_some(id)
}

async fn respond(shared: Arc<Shared>, request: Request<Incoming>) -> Response<Body> {
    let Some(file) = requested_file(&shared.token, request.uri().path()) else {
        return status(StatusCode::NOT_FOUND);
    };
    let method = request.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return status(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Ok(api) = Api::connect(&shared.home) else {
        shared.interrupted(
            &file,
            l10n!(CATALOG, language(), "files-stream-not-connected"),
        );
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let mut forwarded = api.stream(method.clone(), &Api::content_path(&file));
    for name in [RANGE, IF_RANGE] {
        if let Some(value) = request.headers().get(&name) {
            forwarded = forwarded.header(name, value.as_bytes());
        }
    }
    let upstream = match forwarded.send().await {
        Ok(upstream) => upstream,
        Err(e) => {
            tracing::info!(file = %file, error = %e, "cannot reach the home server during playback");
            shared.interrupted(&file, e.to_string());
            return status(StatusCode::BAD_GATEWAY);
        }
    };
    let code = upstream.status();
    let headers = upstream.headers().clone();
    if let Some(code) = headers.get(ERROR_HEADER) {
        shared.interrupted(
            &file,
            l10n!(
                CATALOG,
                language(),
                "files-stream-tunnel-unavailable",
                code = format!("{code:?}")
            ),
        );
    }
    let mut response = if method == Method::HEAD {
        status(code)
    } else {
        let (shared, file) = (shared.clone(), file.clone());
        let frames = upstream.bytes_stream().map(move |chunk| {
            chunk.map(Frame::data).map_err(|e| {
                shared.interrupted(&file, e.to_string());
                std::io::Error::other(e)
            })
        });
        let mut response = Response::new(BodyExt::boxed(StreamBody::new(frames)));
        *response.status_mut() = code;
        response
    };
    let extra = response.headers_mut();
    for name in FORWARDED {
        if let Some(value) = headers.get(name) {
            extra.insert(name, value.clone());
        }
    }
    extra.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    extra.insert(
        "content-security-policy",
        HeaderValue::from_static("sandbox"),
    );
    response
}
