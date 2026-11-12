use std::{io::ErrorKind, net::SocketAddr, time::Duration};

use tokio::{
    net::UdpSocket,
    time::{Instant, sleep, timeout},
};

use crate::support::WAIT;
use wonton_tunnel::{FlowTimeouts, wire::datagram::Frame};

use crate::port_udp::{Peer, QUIET as QUIET_PACKET, bounded, receive_retry, send_whole, upstream};

async fn rebind_released_source(source: SocketAddr) -> UdpSocket {
    bounded(async {
        loop {
            match UdpSocket::bind(source).await {
                Ok(socket) => return socket,
                Err(error) if error.kind() == ErrorKind::AddrInUse => {
                    sleep(Duration::from_millis(1)).await;
                }
                Err(error) => panic!("cannot rebind observed flow source {source}: {error}"),
            }
        }
    })
    .await
}

async fn assert_control_closed(recv: &mut iroh::endpoint::RecvStream) {
    let mut byte = [0];
    let result = bounded(recv.read(&mut byte)).await;
    assert!(
        matches!(result, Ok(None) | Err(_)),
        "control stream stayed live or carried payload: {result:?}"
    );
}

#[tokio::test]
async fn port_udp_fin_releases_socket_and_closes_control_stream() {
    bounded(async {
        let socket = upstream().await;
        let peer = Peer::start(Ok(socket.local_addr().unwrap())).await;
        let mut flow = peer.udp().await;
        let (_, source) = receive_retry(
            &socket,
            || send_whole(&peer.conn, flow.id, b"find-source"),
            "find source",
        )
        .await;
        flow.send.finish().unwrap();
        assert_control_closed(&mut flow.recv).await;
        let _rebound = rebind_released_source(source).await;
        // The closed flow is still held while the replacement proves unregister completed.
        let replacement = peer.udp().await;
        assert_ne!(replacement.id, flow.id);
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_recv_stop_releases_socket_while_send_half_remains_held() {
    bounded(async {
        let socket = upstream().await;
        let peer = Peer::start(Ok(socket.local_addr().unwrap())).await;
        let mut flow = peer.udp().await;
        let (_, source) = receive_retry(
            &socket,
            || send_whole(&peer.conn, flow.id, b"find-source"),
            "find source",
        )
        .await;
        flow.recv.stop(0u32.into()).unwrap();
        let replacement = peer.udp().await;
        assert_ne!(replacement.id, flow.id);
        let _rebound = rebind_released_source(source).await;
        // The send half is deliberately still owned by the peer when the socket is released.
        let _held_send = flow.send;
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_connection_close_releases_every_flow_socket() {
    bounded(async {
        let socket = upstream().await;
        let peer = Peer::start(Ok(socket.local_addr().unwrap())).await;
        let first = peer.udp().await;
        let second = peer.udp().await;
        let (_, source_a) = receive_retry(
            &socket,
            || send_whole(&peer.conn, first.id, b"first-source"),
            "first source",
        )
        .await;
        let (_, source_b) = receive_retry(
            &socket,
            || send_whole(&peer.conn, second.id, b"second-source"),
            "second source",
        )
        .await;
        assert_ne!(source_a, source_b);
        peer.conn.close(0u32.into(), b"release flows");
        let _rebound_a = rebind_released_source(source_a).await;
        let _rebound_b = rebind_released_source(source_b).await;
        // Retain both control stream handles until after rebinding, so drops cannot mask leaks.
        let _held_flows = (first, second);
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_injected_idle_timeout_closes_control_and_releases_socket() {
    bounded(async {
        let socket = upstream().await;
        let idle = Duration::from_millis(400);
        let timeouts = FlowTimeouts {
            idle,
            reassembly: Duration::from_millis(100),
        };
        let peer = Peer::with_options(Ok(socket.local_addr().unwrap()), timeouts, false).await;
        let mut flow = peer.udp().await;
        let sent = Instant::now();
        let (_, source) = receive_retry(
            &socket,
            || send_whole(&peer.conn, flow.id, b"last-activity"),
            "last activity",
        )
        .await;
        timeout(
            idle + Duration::from_secs(1),
            assert_control_closed(&mut flow.recv),
        )
        .await
        .expect("injected idle timeout was ignored");
        assert!(
            sent.elapsed() >= idle,
            "flow expired before the injected idle interval"
        );
        let _rebound = rebind_released_source(source).await;
        // The idle flow is still held while the replacement proves unregister completed.
        let replacement = peer.udp().await;
        assert_ne!(replacement.id, flow.id);
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_each_direction_independently_refreshes_idle_timeout() {
    bounded(async {
        let socket = upstream().await;
        let idle = Duration::from_millis(600);
        let interval = idle / 4;
        let timeouts = FlowTimeouts {
            idle,
            reassembly: Duration::from_millis(100),
        };
        let peer = Peer::with_options(Ok(socket.local_addr().unwrap()), timeouts, false).await;
        let mut flow = peer.udp().await;
        let (_, source) = receive_retry(
            &socket,
            || send_whole(&peer.conn, flow.id, b"bootstrap"),
            "bootstrap",
        )
        .await;
        let inbound_started = Instant::now();
        for n in 0..6 {
            sleep(interval).await;
            let seen = receive_retry(
                &socket,
                || send_whole(&peer.conn, flow.id, b"client-only"),
                &format!("inbound packet {n}"),
            )
            .await;
            assert_eq!(seen, (b"client-only".to_vec(), source));
        }
        assert!(inbound_started.elapsed() > idle);
        let outbound_started = Instant::now();
        let mut last_activity = Instant::now();
        for n in 0..6 {
            sleep(interval).await;
            last_activity = Instant::now();
            let deadline = Instant::now() + WAIT;
            let bytes = loop {
                socket.send_to(b"upstream-only", source).await.unwrap();
                match timeout(QUIET_PACKET, peer.conn.read_datagram()).await {
                    Ok(bytes) => break bytes.unwrap(),
                    Err(_) if Instant::now() < deadline => continue,
                    Err(_) => panic!("outbound packet {n} missing"),
                }
            };
            let frame = Frame::decode(&bytes).unwrap();
            assert_eq!(frame.flow, flow.id);
            assert_eq!(frame.fragment, None);
            assert_eq!(frame.payload, b"upstream-only");
        }
        assert!(outbound_started.elapsed() > idle);
        timeout(
            idle + Duration::from_secs(1),
            assert_control_closed(&mut flow.recv),
        )
        .await
        .expect("flow did not expire after bidirectional traffic stopped");
        assert!(
            last_activity.elapsed() >= idle,
            "last upstream packet did not refresh idle time"
        );
        let _rebound = rebind_released_source(source).await;
        peer.ping().await;
        peer.close().await;
    })
    .await;
}
