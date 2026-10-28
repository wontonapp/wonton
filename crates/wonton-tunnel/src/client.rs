//! 隧道客户端：所有请求复用一条到服务端的连接；连接断开后，下一次使用时自动重连。
//!
//! 不设在线超时门禁：建连失败与否由 iroh 自己判断（打洞、中继兜底、QUIC 握手超时），客户端只如实报告。

use std::{
    fmt,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, TransportAddr,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::sync::{OnceCell, watch};
use tracing::{debug, info};

use crate::{
    error::{
        TunnelError, from_connect_error, from_connection_error, from_read_exact_error,
        from_write_error,
    },
    forward,
    wire::{ALPN, CLOSE_NORMAL, Request, Status},
};

/// 隧道客户端的连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    /// 还没有连接过。第一次使用时自动连接。
    Idle,
    /// 正在建立连接，或者连接已建立但服务端还没确认本设备的授权。
    Connecting,
    /// 连接可用，服务端已确认授权。
    Connected,
    /// 连接失败或已断开，原因见错误；下一次使用时自动重连。
    Disconnected(TunnelError),
    /// 客户端已关闭，不再重连。
    Closed,
}

/// 传输路径的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// 局域网直连或打洞成功的 P2P 直连。
    Direct,
    /// 经中继服务器转发。
    Relay,
}

/// 当前连接正在使用的传输路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInfo {
    /// 直连还是中继。
    pub kind: PathKind,
    /// 对端在这条路径上的地址：直连时是 IP 和端口，中继时是中继服务器。
    pub remote: TransportAddr,
    /// QUIC 对这条路径的往返时间估计。
    pub rtt: Duration,
}

impl fmt::Display for TunnelStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("未连接"),
            Self::Connecting => f.write_str("连接中"),
            Self::Connected => f.write_str("已连接"),
            Self::Disconnected(err) => write!(f, "已断开：{err}"),
            Self::Closed => f.write_str("已关闭"),
        }
    }
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Direct => "直连",
            Self::Relay => "中继",
        })
    }
}

/// 到一台服务端的隧道客户端。克隆得到的是同一个客户端，共享同一条连接。
#[derive(Debug, Clone)]
pub struct TunnelClient {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    endpoint: Endpoint,
    server: EndpointAddr,
    /// 当前的建连尝试。同一时刻最多一次建连，并发请求共享它的结果。
    attempt: Mutex<Arc<Attempt>>,
    status: watch::Sender<TunnelStatus>,
    closed: AtomicBool,
}

#[derive(Debug, Default)]
struct Attempt {
    result: OnceCell<Result<Connection, TunnelError>>,
}

/// 隧道上的一条流，已写入请求类型。
#[derive(Debug)]
pub(crate) struct TunnelStream {
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    /// 承载这条流的连接，用于在服务端回状态后把它标记为已授权。
    pub(crate) conn_id: usize,
}

impl TunnelStream {
    /// 复位流的两个方向。
    pub(crate) fn abort(&mut self) {
        forward::abort(&mut self.send, &mut self.recv);
    }
}

impl TunnelClient {
    /// 用已绑定的 endpoint 连接 `server`。不会立即建连，第一次使用时才连。
    ///
    /// `server` 可以只有 endpoint ID（靠 endpoint 配置的地址发现找到服务端），也可以带上中继地址和
    /// 直连地址。
    pub fn new(endpoint: Endpoint, server: impl Into<EndpointAddr>) -> Self {
        let (status, _) = watch::channel(TunnelStatus::Idle);
        Self {
            inner: Arc::new(Inner {
                endpoint,
                server: server.into(),
                attempt: Mutex::new(Arc::default()),
                status,
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// 服务端的 endpoint ID。
    pub fn server_id(&self) -> EndpointId {
        self.inner.server.id
    }

    /// 当前状态。
    pub fn status(&self) -> TunnelStatus {
        self.inner.status.borrow().clone()
    }

    /// 订阅状态变化。watch 只保留最新状态，订阅方处理慢也不会积压事件。
    pub fn watch_status(&self) -> watch::Receiver<TunnelStatus> {
        self.inner.status.subscribe()
    }

    /// 确认隧道可用：必要时建连，再和服务端做一次往返，确认本设备已获授权。
    ///
    /// 返回这次调用的总耗时；已有连接时约等于一个往返时间，需要建连时包含建连时间。
    pub async fn ping(&self) -> Result<Duration, TunnelError> {
        let started = Instant::now();
        let mut stream = self.open(Request::Ping).await?;
        stream.send.finish().ok();
        self.read_status(&mut stream.recv, stream.conn_id).await?;
        Ok(started.elapsed())
    }

    /// 当前连接正在使用的传输路径。没有可用连接时返回 `None`。
    pub fn path(&self) -> Option<PathInfo> {
        let conn = self.inner.live_connection()?;
        let paths = conn.paths();
        let selected = paths.iter().find(|path| path.is_selected())?;
        let kind = if selected.is_relay() {
            PathKind::Relay
        } else {
            PathKind::Direct
        };
        Some(PathInfo {
            kind,
            remote: selected.remote_addr().clone(),
            rtt: selected.rtt(),
        })
    }

    /// 关闭客户端：断开当前连接，之后的请求都返回 [`TunnelError::Closed`]。
    ///
    /// endpoint 归调用方所有，由调用方关闭。
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Some(conn) = self.inner.live_connection() {
            conn.close(CLOSE_NORMAL, b"client closed");
        }
        self.inner.status.send_replace(TunnelStatus::Closed);
    }

    /// 开一条流并写入请求类型，不等服务端回状态，调用方可以紧接着发送数据。
    pub(crate) async fn open(&self, request: Request) -> Result<TunnelStream, TunnelError> {
        let mut retried = false;
        loop {
            let conn = self.connection().await?;
            match open_stream(&conn, request).await {
                Ok(stream) => return Ok(stream),
                // 连接恰好在检查之后断开。这时还没发出任何数据，换一条新连接重试一次是安全的。
                Err(err @ TunnelError::TransportUnavailable { .. }) if !retried => {
                    debug!("开流失败，换新连接重试：{err}");
                    retried = true;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// 读服务端在流上回的状态字节。服务端回了状态，说明它接受了这台设备。
    pub(crate) async fn read_status(
        &self,
        recv: &mut RecvStream,
        conn_id: usize,
    ) -> Result<(), TunnelError> {
        let mut byte = [0u8; 1];
        recv.read_exact(&mut byte)
            .await
            .map_err(|err| from_read_exact_error(&err))?;
        let status = Status::from_byte(byte[0])
            .ok_or_else(|| TunnelError::protocol(format!("未知的状态字节 {}", byte[0])))?;
        self.inner.mark(conn_id, TunnelStatus::Connected);
        match status {
            Status::Ok => Ok(()),
            Status::UpstreamUnavailable => Err(TunnelError::UpstreamUnavailable),
            Status::UnsupportedRequest => Err(TunnelError::protocol("服务端不支持这个请求类型")),
        }
    }

    /// 取一条可用连接：复用现有连接；没有或已断开时建新连接，并发调用方共享同一次建连。
    async fn connection(&self) -> Result<Connection, TunnelError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(TunnelError::Closed);
        }
        let attempt = self.inner.usable_attempt();
        let result = attempt
            .result
            .get_or_init(|| self.inner.clone().connect())
            .await;
        match result {
            Ok(conn) => match conn.close_reason() {
                None => Ok(conn.clone()),
                // 刚建好就被关闭（例如服务端拒绝）。下一次调用会重新建连。
                Some(reason) => Err(from_connection_error(&reason)),
            },
            Err(err) => Err(err.clone()),
        }
    }
}

impl Inner {
    fn lock_attempt(&self) -> MutexGuard<'_, Arc<Attempt>> {
        self.attempt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 返回进行中或可用的建连尝试；上一次已经失败或连接已断开时换一个新的。
    fn usable_attempt(&self) -> Arc<Attempt> {
        let mut current = self.lock_attempt();
        let stale = match current.result.get() {
            None => false,
            Some(Ok(conn)) => conn.close_reason().is_some(),
            Some(Err(_)) => true,
        };
        if stale {
            *current = Arc::default();
        }
        current.clone()
    }

    fn live_connection(&self) -> Option<Connection> {
        match self.lock_attempt().result.get() {
            Some(Ok(conn)) if conn.close_reason().is_none() => Some(conn.clone()),
            _ => None,
        }
    }

    async fn connect(self: Arc<Self>) -> Result<Connection, TunnelError> {
        self.set_status(TunnelStatus::Connecting);
        let server = self.server.id.fmt_short();
        let conn = match self.endpoint.connect(self.server.clone(), ALPN).await {
            Ok(conn) => conn,
            Err(err) => {
                let err = from_connect_error(&err);
                info!(%server, "隧道建连失败：{err}");
                self.set_status(TunnelStatus::Disconnected(err.clone()));
                return Err(err);
            }
        };
        if self.closed.load(Ordering::SeqCst) {
            conn.close(CLOSE_NORMAL, b"client closed");
            return Err(TunnelError::Closed);
        }
        info!(%server, "隧道连接已建立");
        watch_close(Arc::downgrade(&self), &conn);
        Ok(conn)
    }

    /// 只有 `conn_id` 仍是当前连接时才更新状态，避免旧连接的事件覆盖新连接的状态。
    fn mark(&self, conn_id: usize, status: TunnelStatus) {
        let current = self.lock_attempt();
        if matches!(current.result.get(), Some(Ok(conn)) if conn.stable_id() == conn_id) {
            self.set_status(status);
        }
    }

    /// 更新状态；客户端关闭后状态固定为 `Closed`。
    fn set_status(&self, status: TunnelStatus) {
        self.status.send_if_modified(|current| {
            if *current == TunnelStatus::Closed || *current == status {
                return false;
            }
            *current = status;
            true
        });
    }
}

/// 监听连接关闭，把断开原因写进状态。只持有弱引用，不会让连接或客户端多活。
fn watch_close(inner: Weak<Inner>, conn: &Connection) {
    let closed = conn.weak_handle().closed();
    let conn_id = conn.stable_id();
    tokio::spawn(async move {
        let Some(closed) = closed.await else {
            return;
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let err = from_connection_error(&closed.reason);
        info!(server = %inner.server.id.fmt_short(), "隧道连接已断开：{err}");
        inner.mark(conn_id, TunnelStatus::Disconnected(err));
    });
}

async fn open_stream(conn: &Connection, request: Request) -> Result<TunnelStream, TunnelError> {
    let (mut send, recv) = conn
        .open_bi()
        .await
        .map_err(|err| from_connection_error(&err))?;
    send.write_all(&[request as u8])
        .await
        .map_err(|err| from_write_error(&err))?;
    Ok(TunnelStream {
        send,
        recv,
        conn_id: conn.stable_id(),
    })
}
