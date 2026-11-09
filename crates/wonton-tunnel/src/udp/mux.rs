//! 一条隧道连接上的数据报分发：只有它读 `read_datagram`，按流向编号交给各流向，发送时现取上限分片。
//! 两端共用，不区分客户端和服务端。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use bytes::Bytes;
use iroh::endpoint::{Connection, SendDatagramError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, trace};
use wonton_protocol::UdpFlowStats;

use super::reassembly::Reassembler;
use crate::error::{TunnelError, from_connection_error};
use crate::wire::datagram::{FlowId, Frame, Packet, split};

/// 每个流向收端排队的完整包数；本机收端读得慢时丢新包，实时流宁可丢也不要晚到。
const RECEIVE_QUEUE: usize = 512;

/// 一个流向的计数，分片拼回后只算一个包。
#[derive(Debug, Default)]
pub(crate) struct FlowCounters {
    pub(crate) received: AtomicU64,
    pub(crate) sent: AtomicU64,
    pub(crate) oversized: AtomicU64,
}

#[derive(Debug)]
struct Route {
    deliver: mpsc::Sender<Bytes>,
    reassembler: Reassembler,
    counters: Arc<FlowCounters>,
    plugin: Arc<str>,
    port: u16,
}

/// 发送一个包的失败：连接断了，或者这条连接不传数据报。超长不算失败，计数后丢掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendFailure {
    Tunnel(TunnelError),
    DatagramsUnsupported,
}

#[derive(Debug)]
pub(crate) struct DatagramMux {
    conn: Connection,
    routes: Mutex<HashMap<FlowId, Route>>,
    /// `send_datagram` 成功的次数（按帧算）。
    submitted: AtomicU64,
    /// 建这个分发器时（还没发过数据报）发送缓冲的空余，等于缓冲容量。
    empty_space: usize,
    /// 最近一次能算准的推出帧数；发送缓冲不空时算不准，沿用上一次。
    evicted: Mutex<Option<u64>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    fragment_ttl: Duration,
    external_oversized: Option<Arc<AtomicU64>>,
}

impl DatagramMux {
    pub(crate) fn start(
        conn: Connection,
        fragment_ttl: Duration,
        external_oversized: Option<Arc<AtomicU64>>,
    ) -> Arc<Self> {
        let mux = Arc::new(Self {
            empty_space: conn.datagram_send_buffer_space(),
            conn: conn.clone(),
            routes: Mutex::default(),
            submitted: AtomicU64::new(0),
            evicted: Mutex::new(None),
            reader: Mutex::new(None),
            fragment_ttl,
            external_oversized,
        });
        let task = tokio::spawn(read_loop(conn, Arc::downgrade(&mux)));
        *lock(&mux.reader) = Some(task);
        mux
    }
    fn oversized(&self, counters: &FlowCounters) {
        counters.oversized.fetch_add(1, Ordering::Relaxed);
        if let Some(counter) = &self.external_oversized {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn conn_id(&self) -> usize {
        self.conn.stable_id()
    }

    pub(crate) fn is_live(&self) -> bool {
        self.conn.close_reason().is_none()
    }

    /// 登记流向，返回它的收端。编号已被占用时返回 `None`：同一连接上流向编号不能重复。
    pub(crate) fn register(
        &self,
        flow: FlowId,
        plugin: &str,
        port: u16,
    ) -> Option<(mpsc::Receiver<Bytes>, Arc<FlowCounters>)> {
        let mut routes = lock(&self.routes);
        if routes.contains_key(&flow) {
            return None;
        }
        let (deliver, receive) = mpsc::channel(RECEIVE_QUEUE);
        let counters = Arc::new(FlowCounters::default());
        routes.insert(
            flow,
            Route {
                deliver,
                reassembler: Reassembler::with_ttl(self.fragment_ttl),
                counters: counters.clone(),
                plugin: plugin.into(),
                port,
            },
        );
        Some((receive, counters))
    }

    pub(crate) fn unregister(&self, flow: FlowId) {
        lock(&self.routes).remove(&flow);
    }

    /// 发一个 UDP 包：现取 `max_datagram_size()`，放不进两片就丢掉并计数。
    pub(crate) fn send(
        &self,
        flow: FlowId,
        packet_id: u64,
        payload: &[u8],
        counters: &FlowCounters,
    ) -> Result<(), SendFailure> {
        let Some(max) = self.conn.max_datagram_size() else {
            return Err(SendFailure::DatagramsUnsupported);
        };
        let frames = match split(flow, packet_id, payload, max) {
            Ok(Packet::Whole(frame)) => vec![frame],
            Ok(Packet::Pair(pair)) => pair.to_vec(),
            Err(_) => {
                self.oversized(counters);
                return Ok(());
            }
        };
        for frame in frames {
            let mut datagram = vec![0; frame.header_len() + frame.payload.len()];
            if frame.encode(&mut datagram, max).is_err() {
                self.oversized(counters);
                return Ok(());
            }
            match self.conn.send_datagram(Bytes::from(datagram)) {
                Ok(()) => {
                    self.submitted.fetch_add(1, Ordering::Relaxed);
                }
                // 取上限和发送之间路径变了（ADR-0072：新路径加进来时会从 1414 掉回 1162），这一包丢掉。
                Err(SendDatagramError::TooLarge) => {
                    self.oversized(counters);
                    return Ok(());
                }
                Err(SendDatagramError::ConnectionLost(err)) => {
                    return Err(SendFailure::Tunnel(from_connection_error(&err)));
                }
                Err(SendDatagramError::UnsupportedByPeer | SendDatagramError::Disabled) => {
                    return Err(SendFailure::DatagramsUnsupported);
                }
            }
        }
        counters.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// 发送缓冲推出的帧数：成功提交的帧数减去 QUIC 实际发出的 DATAGRAM 帧数。只在发送缓冲空着时算得准
    /// （还排着的帧不算推出），其余时候沿用上一次算准的值；一次也没算准过时为空。
    pub(crate) fn evicted(&self) -> Option<u64> {
        let submitted = self.submitted.load(Ordering::Relaxed);
        let mut last = lock(&self.evicted);
        if self.conn.datagram_send_buffer_space() >= self.empty_space {
            let sent = self.conn.stats().frame_tx.datagram;
            let now = submitted.saturating_sub(sent);
            *last = Some(last.map_or(now, |previous| previous.max(now)));
        }
        *last
    }

    pub(crate) fn flows(&self) -> Vec<UdpFlowStats> {
        let routes = lock(&self.routes);
        let mut flows: Vec<_> = routes
            .iter()
            .map(|(flow, route)| UdpFlowStats {
                flow_id: flow.get(),
                plugin_id: route.plugin.to_string(),
                port: route.port,
                received_packets: route.counters.received.load(Ordering::Relaxed),
                sent_packets: route.counters.sent.load(Ordering::Relaxed),
                oversized_drops: route.counters.oversized.load(Ordering::Relaxed),
                buffer_evicted_drops: None,
            })
            .collect();
        flows.sort_by_key(|flow| flow.flow_id);
        flows
    }

    fn deliver(&self, datagram: &Bytes) {
        let frame = match Frame::decode(datagram) {
            Ok(frame) => frame,
            Err(err) => {
                debug!("dropped a malformed datagram: {err}");
                return;
            }
        };
        let mut routes = lock(&self.routes);
        let Some(route) = routes.get_mut(&frame.flow) else {
            trace!(
                flow = frame.flow.get(),
                "dropped a datagram for an unknown flow"
            );
            return;
        };
        let Some(packet) = route.reassembler.accept(datagram, frame, Instant::now()) else {
            return;
        };
        route.counters.received.fetch_add(1, Ordering::Relaxed);
        if route.deliver.try_send(packet).is_err() {
            trace!(
                flow = frame.flow.get(),
                "the local receiver is behind, dropped a packet"
            );
        }
    }
}

impl Drop for DatagramMux {
    fn drop(&mut self) {
        if let Some(task) = lock(&self.reader).take() {
            task.abort();
        }
    }
}

/// 读到连接关闭为止。只持有弱引用：分发器没人用了，读循环跟着结束。
async fn read_loop(conn: Connection, mux: Weak<DatagramMux>) {
    while let Ok(datagram) = conn.read_datagram().await {
        let Some(mux) = mux.upgrade() else {
            return;
        };
        mux.deliver(&datagram);
    }
    // 连接没了：丢掉各流向的发送端，收端随之读到结束。
    if let Some(mux) = mux.upgrade() {
        lock(&mux.routes).clear();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
