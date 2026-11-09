use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::time::{sleep, timeout};
use wonton_tunnel::{
    FlowTimeouts,
    wire::datagram::{self, FlowId, Fragment, FragmentIndex, Frame, Packet},
};

use crate::port_udp::{
    Peer, QUIET, assert_quiet, bounded, receive, receive_retry, send_frame, send_whole, upstream,
};

fn fragment(flow: FlowId, packet_id: u64, index: FragmentIndex, payload: &[u8]) -> Frame<'_> {
    Frame {
        flow,
        fragment: Some(Fragment { packet_id, index }),
        payload,
    }
}

async fn receive_pair(conn: &Connection, flow: FlowId, expected: &[u8]) -> usize {
    let first = bounded(conn.read_datagram()).await.unwrap();
    let second = bounded(conn.read_datagram()).await.unwrap();
    let a = Frame::decode(&first).unwrap();
    let b = Frame::decode(&second).unwrap();
    assert_eq!(a.flow, flow);
    assert_eq!(b.flow, flow);
    let fa = a
        .fragment
        .expect("outgoing packet must have exactly two fragments");
    let fb = b
        .fragment
        .expect("outgoing packet must have exactly two fragments");
    assert_eq!(fa.packet_id, fb.packet_id);
    assert_ne!(fa.index, fb.index);
    let mut joined = vec![0; expected.len()];
    assert_eq!(
        datagram::join(Some(a), Some(b), &mut joined)
            .unwrap()
            .unwrap(),
        expected
    );
    let largest = first.len().max(second.len());
    assert!(
        timeout(QUIET, conn.read_datagram()).await.is_err(),
        "packet emitted more than two frames"
    );
    largest
}

#[tokio::test]
async fn port_udp_reordered_fragments_and_exact_outgoing_two_frame_boundary() {
    bounded(async {
        let socket = upstream().await;
        let peer = Peer::with_options(
            Ok(socket.local_addr().unwrap()),
            FlowTimeouts::default(),
            true,
        )
        .await;
        let flow = peer.udp().await;
        let max = peer.conn.max_datagram_size().unwrap();
        let fragment_header = flow.id.encoded_len() + 1 + 8;
        let budget = max.checked_sub(fragment_header).unwrap();
        let payload: Vec<u8> = (0..2 * budget).map(|n| (n % 251) as u8).collect();
        assert_eq!(payload.len() + 2 * fragment_header, 2 * max);
        let Packet::Pair(pair) = datagram::split(flow.id, 7, &payload, max).unwrap() else {
            panic!("exact two-frame payload must be fragmented");
        };
        assert_eq!(pair[0].payload.len(), budget);
        assert_eq!(pair[1].payload.len(), budget);
        send_frame(&peer.conn, pair[1]);
        send_frame(&peer.conn, pair[0]);
        let (seen, source) = receive(&socket).await;
        assert_eq!(seen, payload);
        assert_quiet(&socket).await;
        socket.send_to(&seen, source).await.unwrap();
        // Symmetric fixed-MTU endpoints must expose the same exact sending budget.
        assert_eq!(receive_pair(&peer.conn, flow.id, &payload).await, max);
        assert_eq!(peer.protocol.oversized_drops(), 0);
        let oversized = vec![0x5a; 2 * budget + 1];
        assert_eq!(oversized.len(), payload.len() + 1);
        socket.send_to(&oversized, source).await.unwrap();
        bounded(async {
            while peer.protocol.oversized_drops() == 0 {
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
        assert_eq!(peer.protocol.oversized_drops(), 1);
        assert!(timeout(QUIET, peer.conn.read_datagram()).await.is_err());
        socket.send_to(b"after-oversized", source).await.unwrap();
        let bytes = peer.conn.read_datagram().await.unwrap();
        let frame = Frame::decode(&bytes).unwrap();
        assert_eq!(frame.flow, flow.id);
        assert_eq!(frame.fragment, None);
        assert_eq!(frame.payload, b"after-oversized");
        assert_eq!(peer.protocol.oversized_drops(), 1);
        peer.ping().await;
        peer.close().await;
    })
    .await;
}

#[tokio::test]
async fn port_udp_missing_expired_and_duplicate_fragments_never_emit_partial_or_duplicate_packets()
{
    bounded(async {
        let socket = upstream().await;
        let reassembly = Duration::from_millis(200);
        let timeouts = FlowTimeouts {
            idle: Duration::from_secs(5),
            reassembly,
        };
        let peer = Peer::with_options(Ok(socket.local_addr().unwrap()), timeouts, false).await;
        let flow = peer.udp().await;
        send_frame(
            &peer.conn,
            fragment(flow.id, 10, FragmentIndex::First, b"missing-"),
        );
        assert_quiet(&socket).await;
        sleep(reassembly).await;
        // An expired packet cannot be revived by its late mate or a replay of both pieces.
        send_frame(
            &peer.conn,
            fragment(flow.id, 10, FragmentIndex::Second, b"late"),
        );
        send_frame(
            &peer.conn,
            fragment(flow.id, 10, FragmentIndex::First, b"missing-"),
        );
        assert_quiet(&socket).await;
        send_frame(
            &peer.conn,
            fragment(flow.id, 11, FragmentIndex::First, b"one-"),
        );
        send_frame(
            &peer.conn,
            fragment(flow.id, 11, FragmentIndex::First, b"replacement-"),
        );
        send_frame(
            &peer.conn,
            fragment(flow.id, 11, FragmentIndex::Second, b"packet"),
        );
        let (seen, _) = receive_retry(
            &socket,
            || {
                send_frame(
                    &peer.conn,
                    fragment(flow.id, 11, FragmentIndex::First, b"one-"),
                );
                send_frame(
                    &peer.conn,
                    fragment(flow.id, 11, FragmentIndex::Second, b"packet"),
                );
            },
            "duplicate first fragment",
        )
        .await;
        assert_eq!(seen, b"one-packet");
        send_frame(
            &peer.conn,
            fragment(flow.id, 11, FragmentIndex::First, b"one-"),
        );
        send_frame(
            &peer.conn,
            fragment(flow.id, 11, FragmentIndex::Second, b"packet"),
        );
        assert_quiet(&socket).await;
        let seen = receive_retry(
            &socket,
            || {
                send_frame(
                    &peer.conn,
                    fragment(flow.id, 12, FragmentIndex::Second, b"works"),
                );
                send_frame(
                    &peer.conn,
                    fragment(flow.id, 12, FragmentIndex::First, b"later-"),
                );
            },
            "reordered pair",
        )
        .await;
        assert_eq!(seen.0, b"later-works");
        send_frame(
            &peer.conn,
            fragment(flow.id, 12, FragmentIndex::Second, b"works"),
        );
        assert_quiet(&socket).await;
        let seen = receive_retry(
            &socket,
            || send_whole(&peer.conn, flow.id, b"whole-still-works"),
            "whole packet",
        )
        .await;
        assert_eq!(seen.0, b"whole-still-works");
        peer.ping().await;
        peer.close().await;
    })
    .await;
}
