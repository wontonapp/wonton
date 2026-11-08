//! 只给测试用的插件端口服务端（feature `test-hooks`）：`Ping`、`Forward`、`Port`、`Udp` 都接，不查白名单。
//! 真正的服务端在 `server.rs`（ADR-0072 的第 2 步）；客户端的行为测试不等它，用这个最小的假服务端。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use iroh::Endpoint;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use tokio::net::{TcpStream, UdpSocket};
use tracing::debug;
use wonton_protocol::PortProtocol;

use crate::client::TunnelStream;
use crate::ports::PortResolver;
use crate::udp::{PortStream, UdpFlow, mux::DatagramMux};
use crate::wire::{ALPN, MAX_PORT_REQUEST_LEN, PortRequest, Request, Status, datagram::FlowId};

/// 测试里的家里服务器。丢掉它就关掉 endpoint 和所有连接。
#[derive(Debug)]
pub struct PortServer {
    router: Router,
}

impl PortServer {
    /// `upstream` 是 `Forward` 流的去处；为空时回 `UpstreamUnavailable`。
    pub fn spawn<R: PortResolver>(
        endpoint: Endpoint,
        resolver: R,
        upstream: Option<SocketAddr>,
    ) -> Self {
        let handler = Handler {
            resolver: Arc::new(resolver),
            upstream,
        };
        Self {
            router: Router::builder(endpoint).accept(ALPN, handler).spawn(),
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    pub async fn shutdown(self) {
        self.router.shutdown().await.ok();
    }
}

#[derive(Debug)]
struct Handler<R> {
    resolver: Arc<R>,
    upstream: Option<SocketAddr>,
}

impl<R: PortResolver> ProtocolHandler for Handler<R> {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let mux = DatagramMux::start(conn.clone());
        let next_flow = Arc::new(AtomicU16::new(0));
        while let Ok((send, recv)) = conn.accept_bi().await {
            let stream = TunnelStream {
                send,
                recv,
                conn_id: conn.stable_id(),
            };
            tokio::spawn(serve(
                stream,
                self.resolver.clone(),
                self.upstream,
                mux.clone(),
                next_flow.clone(),
            ));
        }
        Ok(())
    }
}

async fn serve<R: PortResolver>(
    mut stream: TunnelStream,
    resolver: Arc<R>,
    upstream: Option<SocketAddr>,
    mux: Arc<DatagramMux>,
    next_flow: Arc<AtomicU16>,
) {
    let mut kind = [0u8; 1];
    if stream.recv.read_exact(&mut kind).await.is_err() {
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Ping) => reply(&mut stream.send, Status::Ok).await,
        Some(Request::Forward) => forward_to(stream, upstream).await,
        Some(kind @ (Request::Port | Request::Udp)) => {
            let Some((plugin, port)) = read_header(kind, &mut stream.recv).await else {
                return;
            };
            let protocol = if kind == Request::Port {
                PortProtocol::Tcp
            } else {
                PortProtocol::Udp
            };
            let target = match resolver.resolve(&plugin, port, protocol).await {
                Ok(target) => target,
                Err(err) => return reply(&mut stream.send, err.status()).await,
            };
            if kind == Request::Port {
                forward_to(stream, Some(target)).await;
            } else {
                let flow = next_flow.fetch_add(1, Ordering::Relaxed);
                serve_udp(
                    stream,
                    target,
                    mux,
                    FlowId::new(flow).unwrap(),
                    &plugin,
                    port,
                )
                .await;
            }
        }
        None => reply(&mut stream.send, Status::UnsupportedRequest).await,
    }
}

async fn read_header(kind: Request, recv: &mut RecvStream) -> Option<(String, u16)> {
    let mut header = [0u8; MAX_PORT_REQUEST_LEN];
    header[0] = kind as u8;
    recv.read_exact(&mut header[1..2]).await.ok()?;
    let len = usize::from(header[1]) + 4;
    recv.read_exact(header.get_mut(2..len)?).await.ok()?;
    let (request, _) = PortRequest::decode(&header[..len]).ok()?;
    Some((request.plugin.to_owned(), request.port))
}

async fn forward_to(mut stream: TunnelStream, upstream: Option<SocketAddr>) {
    let Some(tcp) = connect(upstream).await else {
        return reply(&mut stream.send, Status::UpstreamUnavailable).await;
    };
    if stream.send.write_all(&[Status::Ok as u8]).await.is_ok() {
        PortStream { stream }.forward(tcp).await;
    }
}

async fn serve_udp(
    mut stream: TunnelStream,
    target: SocketAddr,
    mux: Arc<DatagramMux>,
    flow: FlowId,
    plugin: &str,
    port: u16,
) {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else {
        return reply(&mut stream.send, Status::UpstreamUnavailable).await;
    };
    if socket.connect(target).await.is_err() {
        return reply(&mut stream.send, Status::UpstreamUnavailable).await;
    }
    let mut ok = [Status::Ok as u8, 0, 0];
    let len = 1 + flow.encode(&mut ok[1..]).unwrap();
    if stream.send.write_all(&ok[..len]).await.is_err() {
        return;
    }
    let Ok(UdpFlow {
        sender,
        mut receiver,
    }) = UdpFlow::new(mux, flow, stream, plugin, port)
    else {
        return;
    };
    let mut buf = vec![0u8; 65536];
    loop {
        tokio::select! {
            received = socket.recv(&mut buf) => match received {
                Ok(n) => {
                    if sender.send(&buf[..n]).is_err() {
                        return;
                    }
                }
                Err(err) => return debug!("test plugin socket failed: {err}"),
            },
            packet = receiver.recv() => match packet {
                Ok(packet) => {
                    socket.send(&packet).await.ok();
                }
                Err(_) => return,
            },
        }
    }
}

async fn connect(target: Option<SocketAddr>) -> Option<TcpStream> {
    TcpStream::connect(target?).await.ok()
}

async fn reply(send: &mut SendStream, status: Status) {
    if send.write_all(&[status as u8]).await.is_ok() {
        send.finish().ok();
    }
}
