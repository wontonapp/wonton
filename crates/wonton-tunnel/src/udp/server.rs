//! 服务端的插件 UDP 流向：一个流向一个套接字，控制流开着流向就活着。
//!
//! 分发和分片都在 [`DatagramMux`]；这里只管编号、套接字、空闲关闭和有界的交付队列。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};
use tracing::debug;

use super::mux::{DatagramMux, FlowCounters, SendFailure};
use crate::wire::{STREAM_ABORTED, Status, datagram::FlowId};

/// 单个 UDP 载荷上限（UDP 长度字段的极限）。
const UDP_BUFFER: usize = 65_535;

/// 流向的空闲时限和分片等待时限。
#[derive(Clone, Copy, Debug)]
pub struct FlowTimeouts {
    pub idle: Duration,
    pub reassembly: Duration,
}

impl Default for FlowTimeouts {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(120),
            reassembly: super::reassembly::FRAGMENT_TTL,
        }
    }
}

/// 一条连接上的 UDP 流向：编号游标和共用的数据报分发。
pub(crate) struct Flows {
    conn: Connection,
    mux: Arc<DatagramMux>,
    next: AtomicU16,
    timeouts: FlowTimeouts,
}

impl Flows {
    pub(crate) fn start(
        conn: Connection,
        timeouts: FlowTimeouts,
        oversized: Arc<AtomicU64>,
    ) -> Arc<Self> {
        let mux = DatagramMux::start(conn.clone(), timeouts.reassembly, Some(oversized));
        Arc::new(Self {
            conn,
            mux,
            next: AtomicU16::new(0),
            timeouts,
        })
    }

    /// 新流向的编号和收端：连接内不复用；回绕时跳过仍被占用的编号。
    fn allocate(
        &self,
        plugin: &str,
        port: u16,
    ) -> Option<(FlowId, mpsc::Receiver<Bytes>, Arc<FlowCounters>)> {
        let mut claimed = None;
        let flow = allocate_id(&self.next, |id| {
            if let Some(route) = self.mux.register(id, plugin, port) {
                claimed = Some(route);
                true
            } else {
                false
            }
        })?;
        let (packets, counters) = claimed.expect("claimed flow has a registered route");
        Some((flow, packets, counters))
    }

    /// 服务一条 `Udp` 流。
    pub(crate) async fn serve(
        &self,
        mut send: SendStream,
        mut recv: RecvStream,
        address: SocketAddr,
        (plugin, port): (&str, u16),
    ) {
        if self.conn.max_datagram_size().is_none() {
            crate::server::reply(&mut send, Status::UnsupportedRequest).await;
            return;
        }
        let bind = match address.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        };
        let socket = match UdpSocket::bind((bind, 0)).await {
            Ok(socket) => socket,
            Err(err) => {
                debug!("failed to bind plugin UDP socket: {err}");
                crate::server::reply(&mut send, Status::UpstreamUnavailable).await;
                return;
            }
        };
        if let Err(err) = socket.connect(address).await {
            debug!(%address, "failed to connect plugin UDP socket: {err}");
            crate::server::reply(&mut send, Status::UpstreamUnavailable).await;
            return;
        }
        let Some((id, mut packets, counters)) = self.allocate(plugin, port) else {
            crate::server::reply(&mut send, Status::UpstreamUnavailable).await;
            return;
        };
        let mut header = [Status::Ok as u8, 0, 0];
        let len = id.encode(&mut header[1..]).expect("flow id fits two bytes");
        if send.write_all(&header[..len + 1]).await.is_err() {
            self.mux.unregister(id);
            return;
        }
        run(
            &mut send,
            &mut recv,
            &socket,
            &self.mux,
            id,
            &counters,
            &self.timeouts,
            &mut packets,
        )
        .await;
        self.mux.unregister(id);
        drop(socket);
        recv.stop(STREAM_ABORTED).ok();
        send.finish().ok();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "一条流向的参数都在这里，拆开会重复传"
)]
async fn run(
    send: &mut SendStream,
    recv: &mut RecvStream,
    socket: &UdpSocket,
    mux: &Arc<DatagramMux>,
    id: FlowId,
    counters: &FlowCounters,
    timeouts: &FlowTimeouts,
    packets: &mut tokio::sync::mpsc::Receiver<bytes::Bytes>,
) {
    let mut upstream = vec![0; UDP_BUFFER];
    let mut control = [0];
    let mut next_packet = 0u64;
    let idle = sleep(timeouts.idle);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            _ = send.stopped() => {
                debug!(flow = id.get(), "UDP flow ended: stopped");
                break;
            }
            result = recv.read(&mut control) => {
                match result {
                    Ok(None) => debug!(flow = id.get(), "UDP flow ended: control stream EOF"),
                    Ok(Some(_)) => debug!(flow = id.get(), "UDP flow ended: control stream carried data"),
                    Err(error) => debug!(flow = id.get(), error = %error, "UDP flow ended: control stream read error"),
                }
                break;
            }
            _ = &mut idle => {
                debug!(flow = id.get(), "UDP flow ended: idle timeout");
                break;
            }
            packet = packets.recv() => {
                let Some(packet) = packet else {
                    debug!(flow = id.get(), "UDP flow ended: delivery queue closed");
                    break;
                };
                if let Err(error) = socket.send(&packet).await {
                    debug!(flow = id.get(), error = %error, "UDP flow ended: plugin socket send error");
                    break;
                }
                idle.as_mut().reset(Instant::now() + timeouts.idle);
            }
            received = socket.recv(&mut upstream) => match received {
                Ok(len) => {
                    idle.as_mut().reset(Instant::now() + timeouts.idle);
                    match mux.send(id, next_packet, &upstream[..len], counters) {
                        Ok(()) => next_packet += 1,
                        Err(SendFailure::Tunnel(error)) => {
                            debug!(flow = id.get(), error = %error, "UDP flow ended: tunnel lost");
                            break;
                        }
                        Err(SendFailure::DatagramsUnsupported) => {
                            debug!(flow = id.get(), "UDP flow ended: datagrams unsupported");
                            crate::server::reply(send, Status::UnsupportedRequest).await;
                            break;
                        }
                    }
                }
                Err(error) => {
                    debug!(flow = id.get(), error = %error, "UDP flow ended: plugin socket receive error");
                    break;
                }
            },
        }
    }
}
fn allocate_id(next: &AtomicU16, mut claim: impl FnMut(FlowId) -> bool) -> Option<FlowId> {
    for _ in 0..=FlowId::MAX {
        let value = next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(if value == FlowId::MAX { 0 } else { value + 1 })
            })
            .expect("flow ID cursor update always succeeds");
        let flow = FlowId::new(value).expect("flow ID cursor stays in range");
        if claim(flow) {
            return Some(flow);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU16, Ordering};

    use super::{FlowId, allocate_id};

    #[test]
    fn allocator_wraps_skips_taken_and_reuses_released_ids() {
        let next = AtomicU16::new(FlowId::MAX);
        let mut taken = HashSet::from([FlowId::new(FlowId::MAX).unwrap(), FlowId::new(0).unwrap()]);
        assert_eq!(
            allocate_id(&next, |flow| taken.insert(flow)),
            Some(FlowId::new(1).unwrap())
        );

        taken.remove(&FlowId::new(FlowId::MAX).unwrap());
        next.store(FlowId::MAX, Ordering::Relaxed);
        assert_eq!(
            allocate_id(&next, |flow| taken.insert(flow)),
            Some(FlowId::new(FlowId::MAX).unwrap())
        );
    }

    #[test]
    fn allocator_returns_none_when_all_ids_are_taken() {
        let next = AtomicU16::new(0);
        let mut taken: HashSet<_> = (0..=FlowId::MAX)
            .map(|value| FlowId::new(value).unwrap())
            .collect();
        assert_eq!(allocate_id(&next, |flow| taken.insert(flow)), None);
    }
}
