use std::any::Any;
use std::io;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use futures_util::FutureExt;
use hyper::StatusCode;
use hyper::body::{Body as HttpBody, Bytes, Frame, Incoming, SizeHint};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use iroh::EndpointId;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tower::ServiceExt;
use wonton_tunnel::{PeerStream, TunnelListener};

use crate::gateway::TunnelPeer;

pub(super) async fn serve(mut listener: TunnelListener, app: Router) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Some(stream) => {
                    connections.spawn(serve_stream(stream, app.clone()));
                }
                None => return,
            },
            // 及时回收结束的连接任务。
            Some(joined) = connections.join_next() => {
                if let Err(err) = joined
                    && err.is_panic()
                {
                    tracing::error!("隧道 HTTP 连接任务 panic：{err}");
                }
            }
        }
    }
}

/// 收尾时排空读半边最多等这么久：等客户端把剩下的数据读走、把自己的 FIN 发完，超时就丢。
const CLOSE_DRAIN: Duration = Duration::from_secs(2);

/// 隧道流交给 hyper 的共享句柄：hyper 升级时拿走的 IO 也指向它，收尾时要能拿回读写两个半边。
/// 读写在 poll 里用 `try_lock`，收尾时才跨 await 握锁；tokio 的锁柄是 `Send`，任务能进 `JoinSet`。
#[derive(Clone)]
struct SharedIo(Arc<tokio::sync::Mutex<PeerStream>>);

impl SharedIo {
    fn new(stream: PeerStream) -> Self {
        Self(Arc::new(tokio::sync::Mutex::new(stream)))
    }

    async fn lock(&self) -> tokio::sync::MutexGuard<'_, PeerStream> {
        self.0.lock().await
    }
}

impl AsyncRead for SharedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut stream = match self.get_mut().0.try_lock() {
            Ok(stream) => stream,
            Err(_) => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        Pin::new(&mut *stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for SharedIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut stream = match self.get_mut().0.try_lock() {
            Ok(stream) => stream,
            Err(_) => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        Pin::new(&mut *stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut stream = match self.get_mut().0.try_lock() {
            Ok(stream) => stream,
            Err(_) => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        Pin::new(&mut *stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut stream = match self.get_mut().0.try_lock() {
            Ok(stream) => stream,
            Err(_) => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };
        Pin::new(&mut *stream).poll_shutdown(cx)
    }
}

/// 请求体的在途标记：收到请求体置真，收完置假。握手和空闲都是假。
struct InFlightBody {
    inner: Incoming,
    in_flight: Arc<AtomicBool>,
}

impl hyper::body::Body for InFlightBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let this = self.get_mut();
        this.in_flight.store(true, Ordering::Relaxed);
        let polled = HttpBody::poll_frame(Pin::new(&mut this.inner), cx);
        if matches!(polled, Poll::Ready(None)) || this.inner.is_end_stream() {
            this.in_flight.store(false, Ordering::Relaxed);
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// 响应体出错留个记号：hyper 只把它包成自己的错误，走到收尾时已经分不出是网关的响应体坏了。
struct FailedBody {
    inner: Body,
    failed: Arc<AtomicBool>,
}

impl hyper::body::Body for FailedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = self.get_mut();
        let polled = HttpBody::poll_frame(Pin::new(&mut this.inner), cx);
        if matches!(polled, Poll::Ready(Some(Err(_)))) {
            this.failed.store(true, Ordering::Relaxed);
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// 异常收尾的侧别（`upstream` 是网关这边、`downstream` 是设备那边）和机制，进日志。
/// 只按 hyper 给得出的事实分，看不出哪一侧就写 `unknown`。
fn classify(err: &hyper::Error, response_body_failed: bool) -> (&'static str, &'static str) {
    if response_body_failed {
        return ("upstream", "response_body_aborted");
    }
    if err.is_parse() {
        return ("downstream", "malformed");
    }
    if err.is_incomplete_message() {
        return ("downstream", "incomplete");
    }
    // hyper 说的是写响应体时用户代码中止，不是请求体。
    if err.is_body_write_aborted() {
        return ("upstream", "response_body_aborted");
    }
    if err.is_canceled() || err.is_closed() {
        return ("unknown", "cancelled");
    }
    if err.is_shutdown() {
        return ("downstream", "write_failed");
    }
    if err.is_timeout() {
        return ("unknown", "timeout");
    }
    match io_kind(err) {
        Some(
            io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected,
        ) => ("downstream", "write_failed"),
        Some(io::ErrorKind::UnexpectedEof) => ("downstream", "incomplete"),
        _ => ("unknown", "other"),
    }
}

fn io_kind(err: &hyper::Error) -> Option<io::ErrorKind> {
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            return Some(io.kind());
        }
        source = cause.source();
    }
    None
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "（panic 内容认不出）".to_owned())
}

/// 收尾日志：正常结束记 debug，异常记 ERROR 带事件名、侧别、机制、panic 标记和详情。
fn log_outcome(
    device: &EndpointId,
    upgraded: bool,
    outcome: &Result<Result<(), hyper::Error>, Box<dyn Any + Send>>,
    response_body_failed: bool,
) {
    let (side, category, is_panic, detail) = match outcome {
        Ok(Ok(())) => ("downstream", "closed", false, "连接正常结束".to_owned()),
        Ok(Err(err)) => {
            let (side, category) = classify(err, response_body_failed);
            (side, category, false, err.to_string())
        }
        Err(payload) => ("upstream", "panic", true, panic_text(payload.as_ref())),
    };
    match (upgraded, category == "closed") {
        (false, true) => tracing::debug!(
            device = %device.fmt_short(),
            side = %side,
            category = %category,
            "隧道上的 HTTP 连接结束"
        ),
        (true, true) => tracing::debug!(
            device = %device.fmt_short(),
            side = %side,
            category = %category,
            "隧道连接升级结束"
        ),
        (false, false) => tracing::error!(
            event = "tunnel_stream_aborted",
            device = %device.fmt_short(),
            side = %side,
            is_panic,
            category = %category,
            detail = %detail,
            "隧道上的 HTTP 连接异常结束"
        ),
        // 101 之后读写在处理器的任务里，这里不收尾，但交接失败照样得记下来。
        (true, false) => tracing::error!(
            event = "tunnel_stream_aborted",
            device = %device.fmt_short(),
            side = %side,
            is_panic,
            category = %category,
            detail = %detail,
            "隧道连接升级后交接失败"
        ),
    }
}

/// 用网关服务一条隧道流（一条设备本地代理收到的 TCP 连接），支持 keep-alive 和 WebSocket 等协议升级。
///
/// 异常结束时不能直接丢弃流：`PeerStream` 没有 shutdown 就 Drop 会复位，客户端下一次读拿到复位，
/// 代理在状态字节前收到就回 502（#5）。所以先 shutdown 写半边让客户端读到 EOF；有在途请求时再排空
/// 读半边，让客户端把自己的 FIN 发完。升级后的读写还在处理器的任务里，这里不收尾，仍交给 Drop。
async fn serve_stream(stream: PeerStream, app: Router) {
    let device = stream.remote_id();
    let io = SharedIo::new(stream);
    let in_flight = Arc::new(AtomicBool::new(false));
    let body_failed = Arc::new(AtomicBool::new(false));
    let upgraded = Arc::new(AtomicBool::new(false));
    let service_in_flight = in_flight.clone();
    let service_body_failed = body_failed.clone();
    let service_upgraded = upgraded.clone();
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let app = app.clone();
        let in_flight = service_in_flight.clone();
        let body_failed = service_body_failed.clone();
        let upgraded = service_upgraded.clone();
        async move {
            let mut req = req.map(|body| {
                Body::new(InFlightBody {
                    inner: body,
                    in_flight,
                })
            });
            req.extensions_mut().insert(TunnelPeer(device));
            let response = app.oneshot(req).await;
            if let Ok(response) = &response
                && response.status() == StatusCode::SWITCHING_PROTOCOLS
            {
                upgraded.store(true, Ordering::Relaxed);
            }
            response.map(|response| {
                response.map(|body| {
                    Body::new(FailedBody {
                        inner: body,
                        failed: body_failed,
                    })
                })
            })
        }
    });
    let served = http1::Builder::new()
        .serve_connection(TokioIo::new(io.clone()), service)
        .with_upgrades();
    let outcome = AssertUnwindSafe(served).catch_unwind().await;

    let upgraded = upgraded.load(Ordering::Relaxed);
    log_outcome(
        &device,
        upgraded,
        &outcome,
        body_failed.load(Ordering::Relaxed),
    );
    if upgraded {
        return;
    }

    let mut stream = io.lock().await;
    if let Err(err) = AsyncWriteExt::shutdown(&mut *stream).await {
        tracing::debug!(
            device = %device.fmt_short(),
            error = %err,
            "隧道写半边 shutdown 失败，客户端读到的是复位"
        );
    }
    if in_flight.load(Ordering::Relaxed) {
        let drained = timeout(CLOSE_DRAIN, async {
            let mut buf = [0u8; 4096];
            while let Ok(read) = AsyncReadExt::read(&mut *stream, &mut buf).await {
                if read == 0 {
                    break;
                }
            }
        })
        .await;
        if drained.is_err() {
            tracing::debug!(device = %device.fmt_short(), "隧道对端没在 {CLOSE_DRAIN:?} 内读完");
        }
    }
}

#[cfg(test)]
#[path = "tunnel_close_tests.rs"]
mod tunnel_close_tests;
