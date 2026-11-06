//! 响应头期限从请求体写入上游套接字后开始，不把上传或连接池等待算进去。

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use axum::body::Body;
use futures_util::{TryFutureExt, future::MapOk};
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper::{
    Request, Uri,
    body::{Body as HttpBody, Bytes, Frame, SizeHint},
};
use hyper_util::{
    client::legacy::connect::{CaptureConnection, Connected, Connection, capture_connection},
    rt::TokioIo,
};
use tokio::{net::TcpStream, sync::watch, time::Instant};
use tower::Service;

use super::HttpError;

#[derive(Clone, Default)]
pub(crate) struct Flushed(Arc<Mutex<Option<watch::Sender<Option<Instant>>>>>);

impl Flushed {
    fn arm(&self, sender: watch::Sender<Option<Instant>>) {
        *self.0.lock().unwrap() = Some(sender);
    }

    fn finish(&self) {
        if let Some(sender) = self.0.lock().unwrap().take() {
            sender.send_replace(Some(Instant::now()));
        }
    }
}

pub(crate) struct FlushIo<T> {
    inner: T,
    pub(crate) flushed: Flushed,
}

impl<T> FlushIo<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner,
            flushed: Flushed::default(),
        }
    }
}

impl<T: Connection> Connection for FlushIo<T> {
    fn connected(&self) -> Connected {
        self.inner.connected().extra(self.flushed.clone())
    }
}

impl<T: Read + Unpin> Read for FlushIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: Write + Unpin> Write for FlushIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.flushed.finish();
        }
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[derive(Clone)]
pub(super) struct FlushConnector<C>(pub(super) C);

impl<C> Service<Uri> for FlushConnector<C>
where
    C: Service<Uri, Response = TokioIo<TcpStream>>,
{
    type Response = FlushIo<TokioIo<TcpStream>>;
    type Error = C::Error;
    type Future = MapOk<C::Future, fn(TokioIo<TcpStream>) -> Self::Response>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        self.0.call(uri).map_ok(FlushIo::new)
    }
}

enum Destination {
    Tcp(CaptureConnection),
    Unix(Flushed),
}

struct UploadBody {
    inner: Body,
    destination: Destination,
    completed: watch::Sender<Option<Instant>>,
    armed: AtomicBool,
    remaining: Option<u64>,
}

impl UploadBody {
    fn arm(&self) {
        if self.armed.load(Ordering::Relaxed) {
            return;
        }
        let flushed = match &self.destination {
            Destination::Unix(flushed) => flushed.clone(),
            Destination::Tcp(captured) => {
                let connected = captured.connection_metadata();
                let Some(connected) = connected.as_ref() else {
                    return;
                };
                let mut extras = hyper::http::Extensions::new();
                connected.get_extras(&mut extras);
                extras
                    .remove::<Flushed>()
                    .expect("HTTP connector must track flushes")
            }
        };
        if !self.armed.swap(true, Ordering::Relaxed) {
            flushed.arm(self.completed.clone());
        }
    }
}

impl HttpBody for UploadBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let frame = std::task::ready!(Pin::new(&mut self.inner).poll_frame(cx));
        match &frame {
            Some(Ok(frame)) => {
                if let (Some(remaining), Some(data)) = (&mut self.remaining, frame.data_ref()) {
                    *remaining = remaining.saturating_sub(data.len() as u64);
                }
                if frame.is_trailers() || self.is_end_stream() {
                    self.arm();
                }
            }
            None => self.arm(),
            Some(Err(_)) => {}
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        let ended = self.inner.is_end_stream() || self.remaining == Some(0);
        if ended {
            self.arm();
        }
        ended
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

pub(crate) struct ResponseDeadline {
    started: watch::Receiver<Option<Instant>>,
    limit: Duration,
}

impl ResponseDeadline {
    pub(crate) fn tcp(mut request: Request<Body>, limit: Duration) -> (Request<Body>, Self) {
        let captured = capture_connection(&mut request);
        Self::wrap(request, Destination::Tcp(captured), limit)
    }

    pub(crate) fn unix(
        request: Request<Body>,
        flushed: Flushed,
        limit: Duration,
    ) -> (Request<Body>, Self) {
        Self::wrap(request, Destination::Unix(flushed), limit)
    }

    fn wrap(
        request: Request<Body>,
        destination: Destination,
        limit: Duration,
    ) -> (Request<Body>, Self) {
        let (completed, started) = watch::channel(None);
        let remaining = request
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse().ok());
        let request = request.map(|inner| {
            Body::new(UploadBody {
                inner,
                destination,
                completed,
                armed: AtomicBool::new(false),
                remaining,
            })
        });
        (request, Self { started, limit })
    }

    pub(crate) async fn wait<T>(
        mut self,
        sending: impl Future<Output = Result<T, HttpError>>,
    ) -> Result<T, HttpError> {
        let deadline = async {
            let started = self
                .started
                .wait_for(Option::is_some)
                .await
                .ok()
                .and_then(|value| *value);
            let Some(started) = started else {
                return std::future::pending().await;
            };
            tokio::time::sleep_until(started + self.limit).await;
        };
        tokio::select! {
            biased;
            response = sending => response,
            () = deadline => Err(HttpError::ResponseHeadersTimeout(self.limit)),
        }
    }
}
