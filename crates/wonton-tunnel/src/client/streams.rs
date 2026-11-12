//! 客户端开着的隧道流，按请求类型计数。一条连接上能同时开的流数由服务端定，用完以后新流要等额度；
//! 这些计数用来看是哪类请求占着。

use std::{
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::endpoint::Connection;
use tracing::warn;

use super::TunnelStream;
use crate::{
    error::{TunnelError, from_connection_error, from_write_error},
    wire::{CLOSE_NORMAL, Request},
};

/// `open_bi` 只在对端没给额度时才等；等过这么久就记一条带计数的告警。
const OPEN_STREAM_SLOW: Duration = Duration::from_secs(1);
/// 等额度最长这么久，然后关掉这条连接，报传输不可用。
const OPEN_STREAM_DEADLINE: Duration = Duration::from_secs(3);

/// 客户端开着的隧道流数，按请求类型分开，外加这个客户端开着的流数最多时的总数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TunnelStreamStats {
    pub forward: usize,
    pub ping: usize,
    pub port: usize,
    pub udp: usize,
    pub peak: usize,
}

impl TunnelStreamStats {
    /// 开着的流的总数。
    pub fn open(&self) -> usize {
        self.forward + self.ping + self.port + self.udp
    }
}

#[derive(Debug, Default)]
pub(crate) struct StreamCounters {
    forward: AtomicUsize,
    ping: AtomicUsize,
    port: AtomicUsize,
    udp: AtomicUsize,
    open: AtomicUsize,
    peak: AtomicUsize,
    /// 这个客户端从各条流上收到并交给调用方的字节总数。重传的重复数据不算：交给调用方的只有新数据，
    /// 判活看它。
    received: AtomicU64,
}

impl StreamCounters {
    /// 记一批刚交给调用方的新数据。
    pub(crate) fn note_received(&self, bytes: usize) {
        self.received.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 到目前为止交给调用方的新数据字节数。
    pub(crate) fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub(super) fn snapshot(&self) -> TunnelStreamStats {
        TunnelStreamStats {
            forward: self.forward.load(Ordering::Relaxed),
            ping: self.ping.load(Ordering::Relaxed),
            port: self.port.load(Ordering::Relaxed),
            udp: self.udp.load(Ordering::Relaxed),
            peak: self.peak.load(Ordering::Relaxed),
        }
    }

    /// 记一条刚开的流，返回的租约丢弃时（流结束）减回去。
    pub(super) fn lease(self: &Arc<Self>, request: Request) -> StreamLease {
        self.counter(request).fetch_add(1, Ordering::Relaxed);
        let open = self.open.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(open, Ordering::Relaxed);
        StreamLease {
            counters: Arc::clone(self),
            request,
        }
    }

    fn counter(&self, request: Request) -> &AtomicUsize {
        match request {
            Request::Forward => &self.forward,
            Request::Ping => &self.ping,
            Request::Port => &self.port,
            Request::Udp => &self.udp,
        }
    }
}

#[derive(Debug)]
pub(crate) struct StreamLease {
    counters: Arc<StreamCounters>,
    request: Request,
}

impl StreamLease {
    pub(super) fn counters(&self) -> &Arc<StreamCounters> {
        &self.counters
    }
}

impl Drop for StreamLease {
    fn drop(&mut self) {
        self.counters
            .counter(self.request)
            .fetch_sub(1, Ordering::Relaxed);
        self.counters.open.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 开一条流并写入请求类型。
///
/// 额度用完时 `open_bi` 不报错，一直等到对端放出额度或连接关闭，所以要有时限。到时限还没有额度，说明占着
/// 额度的是长期开着的流（长轮询、WebSocket、挂着不回的请求），它们不结束，这条连接上的新请求就一个也开不出来，
/// 只报错不换连接的话之后每个请求都要白等这么久：关掉它，让 [`super::TunnelClient::open`] 的那一次重试换新连接。
/// 代价是这条连接上还开着的流一起断开，由各自的调用方重来。
pub(super) async fn open_stream(
    conn: &Connection,
    request: Request,
    counters: &Arc<StreamCounters>,
) -> Result<TunnelStream, TunnelError> {
    let conn_id = conn.stable_id();
    let mut opening = pin!(conn.open_bi());
    let opened = match tokio::time::timeout(OPEN_STREAM_SLOW, &mut opening).await {
        Ok(opened) => opened,
        Err(_) => {
            warn!(
                conn_id,
                ?request,
                streams = ?counters.snapshot(),
                "waiting for the server to allow another tunnel stream"
            );
            match tokio::time::timeout(OPEN_STREAM_DEADLINE - OPEN_STREAM_SLOW, opening).await {
                Ok(opened) => opened,
                Err(_) => {
                    warn!(
                        conn_id,
                        ?request,
                        streams = ?counters.snapshot(),
                        blocked = conn.stats().frame_tx.streams_blocked_bidi,
                        "the server allowed no tunnel stream in time; closing the connection"
                    );
                    conn.close(CLOSE_NORMAL, b"no stream credit");
                    return Err(TunnelError::transport(
                        "the server allowed no new tunnel stream in time",
                    ));
                }
            }
        }
    };
    let (mut send, recv) = opened.map_err(|err| from_connection_error(&err))?;
    let lease = counters.lease(request);
    send.write_all(&[request as u8])
        .await
        .map_err(|err| from_write_error(&err))?;
    Ok(TunnelStream {
        send,
        recv,
        conn_id,
        _lease: lease,
    })
}
