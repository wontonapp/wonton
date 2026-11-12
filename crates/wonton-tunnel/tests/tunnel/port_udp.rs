//! Raw Port/Udp peers exercise the server independently of the tunnel client.

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{
    Endpoint, SecretKey,
    endpoint::{Connection, QuicTransportConfig, RecvStream, SendStream, presets},
    protocol::Router,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    time::timeout,
};
use wonton_protocol::PortProtocol;
use wonton_tunnel::{
    ALPN, Allowlist, FlowTimeouts, PortResolveError, PortResolver, TunnelProtocol,
    wire::{
        MAX_PORT_REQUEST_LEN, PortRequest, Request, Status,
        datagram::{FlowId, Frame},
    },
};

use crate::support::{Timeouts, WAIT, init_tracing, loopback_endpoint};

pub(super) const QUIET: Duration = Duration::from_millis(80);
pub(super) const TEST_PORT: u16 = 5353;
pub(super) const PLUGIN: &str = "app.raw-peer";

#[derive(Debug)]
pub(super) struct Resolver {
    result: Result<SocketAddr, PortResolveError>,
    pub calls: Mutex<Vec<(String, u16, PortProtocol)>>,
}

impl PortResolver for Resolver {
    async fn resolve(
        &self,
        plugin: &str,
        port: u16,
        protocol: PortProtocol,
    ) -> Result<SocketAddr, PortResolveError> {
        self.calls
            .lock()
            .unwrap()
            .push((plugin.to_owned(), port, protocol));
        self.result
    }
}

pub(super) struct Peer {
    pub conn: Connection,
    pub protocol: TunnelProtocol<Resolver>,
    pub resolver: Arc<Resolver>,
    endpoint: Endpoint,
    router: Router,
}

impl Peer {
    pub async fn start(result: Result<SocketAddr, PortResolveError>) -> Self {
        Self::with_options(result, FlowTimeouts::default(), false).await
    }

    pub async fn with_options(
        result: Result<SocketAddr, PortResolveError>,
        timeouts: FlowTimeouts,
        fixed_mtu: bool,
    ) -> Self {
        let key = SecretKey::generate();
        let server = endpoint(SecretKey::generate(), fixed_mtu).await;
        let addr = server.addr();
        let resolver = Arc::new(Resolver {
            result,
            calls: Mutex::new(Vec::new()),
        });
        let protocol = TunnelProtocol::with_resolver(
            Allowlist::new([key.public()]),
            (Ipv4Addr::LOCALHOST, 9).into(),
            resolver.clone(),
        )
        .with_udp_timeouts(timeouts);
        let router = Router::builder(server)
            .accept(ALPN, protocol.clone())
            .spawn();
        let endpoint = endpoint(key, fixed_mtu).await;
        let conn = bounded(endpoint.connect(addr, ALPN)).await.unwrap();
        Self {
            conn,
            protocol,
            resolver,
            endpoint,
            router,
        }
    }

    pub async fn udp(&self) -> Flow {
        let (send, mut recv) = self.request(Request::Udp, &[]).await;
        let mut status = [255];
        bounded(recv.read_exact(&mut status)).await.unwrap();
        assert_eq!(status, [Status::Ok as u8]);
        let mut bytes = [0; 2];
        bounded(recv.read_exact(&mut bytes[..1])).await.unwrap();
        let len = match bytes[0] >> 6 {
            0 => 1,
            1 => {
                bounded(recv.read_exact(&mut bytes[1..])).await.unwrap();
                2
            }
            width => panic!("unsupported flow integer width {width}"),
        };
        let (id, used) = FlowId::decode(&bytes[..len]).unwrap();
        assert_eq!(used, len);
        Flow { id, send, recv }
    }

    pub async fn request(&self, kind: Request, payload: &[u8]) -> (SendStream, RecvStream) {
        let mut bytes = header(kind);
        bytes.extend_from_slice(payload);
        let (mut send, recv) = bounded(self.conn.open_bi()).await.unwrap();
        bounded(send.write_all(&bytes)).await.unwrap();
        (send, recv)
    }

    pub async fn ping(&self) {
        let (mut send, mut recv) = bounded(self.conn.open_bi()).await.unwrap();
        bounded(send.write_all(&[Request::Ping as u8]))
            .await
            .unwrap();
        send.finish().unwrap();
        assert_eq!(
            bounded(recv.read_to_end(8)).await.unwrap(),
            [Status::Ok as u8]
        );
    }

    pub async fn close(self) {
        self.conn.close(0u32.into(), b"test done");
        bounded(self.endpoint.close()).await;
        bounded(self.router.shutdown()).await.unwrap();
    }
}

async fn endpoint(key: SecretKey, fixed_mtu: bool) -> Endpoint {
    if !fixed_mtu {
        return loopback_endpoint(key, 0, Timeouts::Default).await;
    }
    init_tracing();
    let transport = QuicTransportConfig::builder()
        .initial_mtu(1200)
        .min_mtu(1200)
        .mtu_discovery_config(None)
        .build();
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .transport_config(transport)
        .bind()
        .await
        .unwrap()
}

pub(super) struct Flow {
    pub id: FlowId,
    pub send: SendStream,
    pub recv: RecvStream,
}

pub(super) async fn bounded<F: Future>(future: F) -> F::Output {
    timeout(WAIT, future)
        .await
        .expect("loopback operation timed out")
}

pub(super) fn header(kind: Request) -> Vec<u8> {
    let mut bytes = [0; MAX_PORT_REQUEST_LEN];
    let len = PortRequest::new(kind, PLUGIN, TEST_PORT)
        .unwrap()
        .encode(&mut bytes)
        .unwrap();
    bytes[..len].to_vec()
}

pub(super) fn send_frame(conn: &Connection, frame: Frame<'_>) {
    let max = conn.max_datagram_size().expect("DATAGRAM negotiated");
    let mut bytes = vec![0; frame.header_len() + frame.payload.len()];
    let len = frame.encode(&mut bytes, max).unwrap();
    assert_eq!(len, bytes.len());
    conn.send_datagram(bytes.into()).unwrap();
}

pub(super) fn send_whole(conn: &Connection, flow: FlowId, payload: &[u8]) {
    send_frame(
        conn,
        Frame {
            flow,
            fragment: None,
            payload,
        },
    );
}

pub(super) async fn upstream() -> UdpSocket {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}

/// 数据报不重传，丢了就重发：重发到收到为止，整段仍受 `WAIT` 限制。
pub(super) async fn receive_retry(
    socket: &UdpSocket,
    mut send: impl FnMut(),
    tag: &str,
) -> (Vec<u8>, SocketAddr) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        send();
        match tokio::time::timeout(QUIET, receive(socket)).await {
            Ok(seen) => return seen,
            Err(_) if tokio::time::Instant::now() < deadline => continue,
            Err(_) => panic!("{tag}: no upstream packet after retries"),
        }
    }
}

pub(super) async fn receive(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buffer = vec![0; 65_535];
    let (len, source) = bounded(socket.recv_from(&mut buffer)).await.unwrap();
    buffer.truncate(len);
    (buffer, source)
}

pub(super) async fn assert_quiet(socket: &UdpSocket) {
    let mut buffer = [0; 65_535];
    assert!(
        timeout(QUIET, socket.recv_from(&mut buffer)).await.is_err(),
        "unexpected upstream packet"
    );
}

#[tokio::test]
async fn port_udp_tcp_preserves_opaque_payload_after_one_header() {
    bounded(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let peer = Peer::start(Ok(listener.local_addr().unwrap())).await;
        let mut payload = header(Request::Udp);
        payload.extend_from_slice(b"\0\xffnot-http\r\n");
        let (mut send, mut recv) = peer.request(Request::Port, &payload).await;
        send.finish().unwrap();
        let mut status = [255];
        recv.read_exact(&mut status).await.unwrap();
        assert_eq!(status, [Status::Ok as u8]);
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        tcp.read_to_end(&mut seen).await.unwrap();
        assert_eq!(seen, payload);
        let response = b"\xff\0opaque-response\x03\x04";
        tcp.write_all(response).await.unwrap();
        tcp.shutdown().await.unwrap();
        assert_eq!(recv.read_to_end(1024).await.unwrap(), response);
        assert_eq!(
            *peer.resolver.calls.lock().unwrap(),
            [(PLUGIN.to_owned(), TEST_PORT, PortProtocol::Tcp)]
        );
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_every_resolver_refusal_is_returned_on_both_request_kinds() {
    bounded(async {
        for (error, expected) in [
            (PortResolveError::Unsupported, Status::UnsupportedRequest),
            (
                PortResolveError::PluginNotInstalled,
                Status::PluginNotInstalled,
            ),
            (PortResolveError::PortNotDeclared, Status::PortNotDeclared),
            (PortResolveError::PluginNotRunning, Status::PluginNotRunning),
            (
                PortResolveError::UpstreamUnavailable,
                Status::UpstreamUnavailable,
            ),
        ] {
            let peer = Peer::start(Err(error)).await;
            for kind in [Request::Port, Request::Udp] {
                let (mut send, mut recv) = peer.request(kind, &[]).await;
                send.finish().unwrap();
                assert_eq!(recv.read_to_end(8).await.unwrap(), [expected as u8]);
                peer.ping().await;
            }
            assert_eq!(
                *peer.resolver.calls.lock().unwrap(),
                [
                    (PLUGIN.to_owned(), TEST_PORT, PortProtocol::Tcp),
                    (PLUGIN.to_owned(), TEST_PORT, PortProtocol::Udp),
                ]
            );
            peer.close().await;
        }
    })
    .await;
}

#[tokio::test]
async fn port_udp_tcp_connect_failure_reports_upstream_unavailable() {
    bounded(async {
        // A bound but non-listening TCP socket reserves an address which cannot accept connections.
        let reserved = tokio::net::TcpSocket::new_v4().unwrap();
        reserved.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        let peer = Peer::start(Ok(reserved.local_addr().unwrap())).await;
        let (mut send, mut recv) = peer.request(Request::Port, &[]).await;
        send.finish().unwrap();
        assert_eq!(
            recv.read_to_end(8).await.unwrap(),
            [Status::UpstreamUnavailable as u8]
        );
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_bad_headers_do_not_poison_the_connection_or_call_the_resolver() {
    bounded(async {
        let peer = Peer::start(Err(PortResolveError::PluginNotInstalled)).await;
        for kind in [Request::Port, Request::Udp] {
            let valid = header(kind);
            let mut cases: Vec<Vec<u8>> =
                (1..valid.len()).map(|len| valid[..len].to_vec()).collect();
            cases.extend([
                vec![kind as u8, 0, 4, 0],
                vec![kind as u8, 3, b'A', b'.', b'b', 4, 0],
                vec![kind as u8, 3, b'a', b'.', 255, 4, 0],
                vec![kind as u8, 3, b'a', b'.', b'b', 3, 255],
                vec![kind as u8, 64],
                vec![kind as u8, 255],
            ]);
            for bytes in cases {
                let (mut send, mut recv) = peer.conn.open_bi().await.unwrap();
                send.write_all(&bytes).await.unwrap();
                send.finish().unwrap();
                assert_eq!(
                    recv.read_to_end(8).await.unwrap(),
                    [Status::UnsupportedRequest as u8],
                    "header {bytes:?}"
                );
                peer.ping().await;
            }
        }
        assert!(peer.resolver.calls.lock().unwrap().is_empty());
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_two_flows_have_distinct_ids_and_socket_sources_without_crosstalk() {
    bounded(async {
        let socket = upstream().await;
        let peer = Peer::start(Ok(socket.local_addr().unwrap())).await;
        let first = peer.udp().await;
        let second = peer.udp().await;
        assert_ne!(first.id, second.id);
        send_whole(&peer.conn, first.id, b"first");
        let (bytes, source_a) = receive(&socket).await;
        assert_eq!(bytes, b"first");
        send_whole(&peer.conn, second.id, b"second");
        let (bytes, source_b) = receive(&socket).await;
        assert_eq!(bytes, b"second");
        assert_ne!(source_a, source_b);
        for (source, flow, payload) in [
            (source_b, second.id, b"reply-two"),
            (source_a, first.id, b"reply-one"),
        ] {
            socket.send_to(payload, source).await.unwrap();
            let bytes = peer.conn.read_datagram().await.unwrap();
            let frame = Frame::decode(&bytes).unwrap();
            assert_eq!(frame.flow, flow);
            assert_eq!(frame.fragment, None);
            assert_eq!(frame.payload, payload);
        }
        send_whole(&peer.conn, first.id, b"");
        assert_eq!(receive(&socket).await, (Vec::new(), source_a));
        assert_quiet(&socket).await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_maximum_header_length_is_accepted_before_overlong_header_is_rejected() {
    bounded(async {
        let peer = Peer::start(Err(PortResolveError::PluginNotInstalled)).await;
        let plugin = format!("a.{}", "b".repeat(61));
        assert_eq!(plugin.len(), wonton_tunnel::wire::MAX_PLUGIN_ID_LEN);
        for kind in [Request::Port, Request::Udp] {
            let mut bytes = [0; MAX_PORT_REQUEST_LEN];
            let used = PortRequest::new(kind, &plugin, u16::MAX)
                .unwrap()
                .encode(&mut bytes)
                .unwrap();
            assert_eq!(used, MAX_PORT_REQUEST_LEN);
            let (mut send, mut recv) = peer.conn.open_bi().await.unwrap();
            send.write_all(&bytes).await.unwrap();
            send.finish().unwrap();
            assert_eq!(
                recv.read_to_end(8).await.unwrap(),
                [Status::PluginNotInstalled as u8]
            );
            peer.ping().await;
            let (mut send, mut recv) = peer.conn.open_bi().await.unwrap();
            let oversized = u8::try_from(plugin.len() + 1).unwrap();
            send.write_all(&[kind as u8, oversized]).await.unwrap();
            send.finish().unwrap();
            assert_eq!(
                recv.read_to_end(8).await.unwrap(),
                [Status::UnsupportedRequest as u8]
            );
            peer.ping().await;
        }
        assert_eq!(
            *peer.resolver.calls.lock().unwrap(),
            [
                (plugin.clone(), u16::MAX, PortProtocol::Tcp),
                (plugin, u16::MAX, PortProtocol::Udp),
            ]
        );
        peer.close().await;
    })
    .await;
}
