//! 插件端口（ADR-0072）的客户端：`Port` 流、`Udp` 流向和数据报分片，服务端用真的 `TunnelServer`。
//! “旧服务端”是不接端口解析的 `TunnelServer`，它对 `Port`/`Udp` 回 `UnsupportedRequest`。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use iroh::SecretKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use wonton_protocol::PortProtocol;
use wonton_tunnel::{
    Allowlist, PortError, PortResolveError, PortResolver, TunnelClient, TunnelServer, UdpFlow,
};

use crate::support::{Timeouts, WAIT, loopback_endpoint};

const PLUGIN: &str = "app.remote";

type Routes = HashMap<(u16, PortProtocol), Result<SocketAddr, PortResolveError>>;

#[derive(Debug, Clone, Default)]
struct Resolver(Arc<Routes>);

impl PortResolver for Resolver {
    async fn resolve(
        &self,
        plugin: &str,
        port: u16,
        protocol: PortProtocol,
    ) -> Result<SocketAddr, PortResolveError> {
        if plugin != PLUGIN {
            return Err(PortResolveError::PluginNotInstalled);
        }
        self.0
            .get(&(port, protocol))
            .copied()
            .unwrap_or(Err(PortResolveError::PortNotDeclared))
    }
}

async fn serve(routes: Routes) -> (TunnelServer, TunnelClient) {
    let phone_key = SecretKey::generate();
    let server = TunnelServer::spawn_with_resolver(
        loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await,
        Allowlist::new([phone_key.public()]),
        (Ipv4Addr::LOCALHOST, 9).into(),
        Arc::new(Resolver(Arc::new(routes))),
    );
    let phone = loopback_endpoint(phone_key, 0, Timeouts::Default).await;
    let client = TunnelClient::new(phone, server.endpoint().addr());
    (server, client)
}

async fn udp_echo() -> SocketAddr {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            socket.send_to(&buf[..n], from).await.unwrap();
        }
    });
    addr
}

async fn tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = tcp.split();
                tokio::io::copy(&mut read, &mut write).await.ok();
            });
        }
    });
    addr
}

async fn round_trip(flow: &mut UdpFlow, payload: &[u8]) -> Vec<u8> {
    flow.sender.send(payload).unwrap();
    tokio::time::timeout(WAIT, flow.receiver.recv())
        .await
        .expect("echo did not come back")
        .unwrap()
        .to_vec()
}

#[tokio::test]
async fn a_tcp_port_stream_carries_bytes_both_ways() {
    let routes = Routes::from([((47989, PortProtocol::Tcp), Ok(tcp_echo().await))]);
    let (_server, client) = serve(routes).await;
    let stream = client.open_port(PLUGIN, 47989).await.unwrap();
    let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut app = TcpStream::connect(local.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = local.accept().await.unwrap();
    tokio::spawn(stream.forward(accepted));
    app.write_all(b"hello plugin").await.unwrap();
    app.shutdown().await.unwrap();
    let mut echoed = Vec::new();
    tokio::time::timeout(WAIT, app.read_to_end(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, b"hello plugin");
}

/// 2000 字节超过回环路径上的 `max_datagram_size()`（1162 或 1414），两个方向都要拆两片再拼回。
#[tokio::test]
async fn udp_packets_round_trip_whole_and_in_two_fragments() {
    let routes = Routes::from([((47998, PortProtocol::Udp), Ok(udp_echo().await))]);
    let (_server, client) = serve(routes).await;
    let mut flow = client.open_udp(PLUGIN, 47998).await.unwrap();
    assert_eq!(round_trip(&mut flow, b"small").await, b"small");
    let large: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(round_trip(&mut flow, &large).await, large);
    assert_eq!(
        round_trip(&mut flow, &[]).await,
        b"",
        "empty UDP packets are valid"
    );
    let stats = flow.sender.stats();
    assert_eq!((stats.sent_packets, stats.received_packets), (3, 3));
    assert_eq!(stats.oversized_drops, 0);
    assert_eq!(client.udp_flows(), [stats]);
}

#[tokio::test]
async fn a_packet_beyond_two_fragments_is_dropped_and_counted() {
    let routes = Routes::from([((47998, PortProtocol::Udp), Ok(udp_echo().await))]);
    let (_server, client) = serve(routes).await;
    let mut flow = client.open_udp(PLUGIN, 47998).await.unwrap();
    flow.sender.send(&vec![1u8; 5000]).unwrap();
    assert_eq!(round_trip(&mut flow, b"after").await, b"after");
    let stats = flow.sender.stats();
    assert_eq!(stats.oversized_drops, 1);
    assert_eq!((stats.sent_packets, stats.received_packets), (1, 1));
}

#[tokio::test]
async fn evictions_are_measured_once_a_flow_has_sent() {
    let routes = Routes::from([((47998, PortProtocol::Udp), Ok(udp_echo().await))]);
    let (_server, client) = serve(routes).await;
    client.ping().await.unwrap();
    assert_eq!(
        client.udp_buffer_evicted(),
        None,
        "no datagrams on this connection yet"
    );
    let mut flow = client.open_udp(PLUGIN, 47998).await.unwrap();
    round_trip(&mut flow, b"one").await;
    assert_eq!(client.udp_buffer_evicted(), Some(0));
}

#[tokio::test]
async fn closing_the_receiver_ends_the_flow_and_a_lost_server_ends_it_too() {
    let routes = Routes::from([((47998, PortProtocol::Udp), Ok(udp_echo().await))]);
    let (server, client) = serve(routes).await;
    let first = client.open_udp(PLUGIN, 47998).await.unwrap();
    let mut second = client.open_udp(PLUGIN, 47998).await.unwrap();
    assert_ne!(first.sender.stats().flow_id, second.sender.stats().flow_id);
    drop(first);
    assert_eq!(client.udp_flows().len(), 1);
    assert_eq!(round_trip(&mut second, b"still").await, b"still");
    server.shutdown().await;
    let ended = tokio::time::timeout(WAIT, second.receiver.recv())
        .await
        .unwrap();
    assert!(
        matches!(ended, Err(PortError::Tunnel(_) | PortError::FlowEnded)),
        "{ended:?}"
    );
}

#[tokio::test]
async fn refusals_keep_their_cause() {
    let routes = Routes::from([
        (
            (1024, PortProtocol::Tcp),
            Err(PortResolveError::PluginNotRunning),
        ),
        (
            (1025, PortProtocol::Udp),
            Err(PortResolveError::UpstreamUnavailable),
        ),
    ]);
    let (_server, client) = serve(routes).await;
    let port = |port| client.open_port(PLUGIN, port);
    assert_eq!(port(1024).await.unwrap_err(), PortError::PluginNotRunning);
    assert_eq!(port(1025).await.unwrap_err(), PortError::PortNotDeclared);
    let udp = client.open_udp(PLUGIN, 1025).await.unwrap_err();
    assert_eq!(udp, PortError::UpstreamUnavailable);
    let other = client.open_udp("app.other", 1025).await.unwrap_err();
    assert_eq!(other, PortError::PluginNotInstalled);
    let low = client.open_port(PLUGIN, 80).await.unwrap_err();
    assert!(matches!(low, PortError::Invalid(_)), "{low:?}");
}

#[tokio::test]
async fn an_older_server_is_reported_as_too_old() {
    let phone_key = SecretKey::generate();
    let server = TunnelServer::spawn(
        loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await,
        Allowlist::new([phone_key.public()]),
        (Ipv4Addr::LOCALHOST, 9).into(),
    );
    let phone = loopback_endpoint(phone_key, 0, Timeouts::Default).await;
    let client = TunnelClient::new(phone, server.endpoint().addr());
    let tcp = client.open_port(PLUGIN, 47989).await.unwrap_err();
    assert_eq!(tcp, PortError::ServerTooOld);
    let udp = client.open_udp(PLUGIN, 47998).await.unwrap_err();
    assert_eq!(udp, PortError::ServerTooOld);
}
