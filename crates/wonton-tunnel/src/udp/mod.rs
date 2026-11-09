//! 插件端口经隧道的两种转发（ADR-0072）：TCP 端口一条连接一条 `Port` 流；UDP 端口一个流向一条一直开着的
//! `Udp` 流，载荷走同一连接上的 QUIC 数据报。

pub(crate) mod mux;
pub(crate) mod reassembly;
pub(crate) mod server;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::debug;
use wonton_protocol::UdpFlowStats;

use crate::client::TunnelStream;
use crate::error::{TunnelError, from_read_error};
use crate::forward;
use crate::wire::{CodecError, datagram::FlowId};
use mux::{DatagramMux, FlowCounters, SendFailure};

/// 打开插件端口或经流向收发失败的原因，调用方按类别显示，不能混为一谈。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PortError {
    /// 隧道本身的问题：连不上、被拒绝、已关闭或协议不符。
    #[error(transparent)]
    Tunnel(#[from] TunnelError),
    /// 服务端不认识 `Port`/`Udp` 请求，要升级家里的服务器。
    #[error("the server is too old to forward plugin ports")]
    ServerTooOld,
    #[error("the plugin is not installed")]
    PluginNotInstalled,
    #[error("the plugin did not declare this port and protocol")]
    PortNotDeclared,
    #[error("the plugin is not running")]
    PluginNotRunning,
    /// 插件在跑，服务端却连不上它的服务。
    #[error("the server cannot reach the plugin service")]
    UpstreamUnavailable,
    /// 这条连接不传 QUIC 数据报（对端或本机关掉了）。
    #[error("the tunnel connection does not carry datagrams")]
    DatagramsUnsupported,
    /// 流向被对端或本机结束了。
    #[error("the UDP flow has ended")]
    FlowEnded,
    /// 插件 ID 或端口不符合线协议。
    #[error("invalid plugin port request: {0}")]
    Invalid(CodecError),
}

impl From<SendFailure> for PortError {
    fn from(failure: SendFailure) -> Self {
        match failure {
            SendFailure::Tunnel(err) => Self::Tunnel(err),
            SendFailure::DatagramsUnsupported => Self::DatagramsUnsupported,
        }
    }
}

/// 服务端接受了的插件 TCP 端口流。
#[derive(Debug)]
pub struct PortStream {
    pub(crate) stream: TunnelStream,
}

impl PortStream {
    /// 两个方向各自搬到结束；任一方向出错就复位隧道流，对端不会把截断的数据当成正常结束。
    pub async fn forward(mut self, tcp: TcpStream) {
        let (mut read, mut write) = tcp.into_split();
        let stream = &mut self.stream;
        let delivered = stream.counters().clone();
        let pumped = tokio::try_join!(
            forward::tcp_to_tunnel(&mut read, &mut stream.send),
            forward::tunnel_to_tcp(&mut stream.recv, &mut write, Some(&delivered)),
        );
        if let Err(err) = pumped {
            debug!("plugin port connection ended with an error: {err}");
            stream.abort();
        }
    }
}

/// 一个 UDP 流向。拆成可克隆的发送端和独占的收端；收端丢掉时流向结束。
#[derive(Debug)]
pub struct UdpFlow {
    pub sender: UdpSender,
    pub receiver: UdpReceiver,
}

/// 流向的发送端，发送不阻塞。
#[derive(Debug, Clone)]
pub struct UdpSender {
    mux: Arc<DatagramMux>,
    flow: FlowId,
    /// 同一连接、同一方向、同一流向内不复用包编号（`wire::datagram` 的约束）。
    next_packet: Arc<AtomicU64>,
    counters: Arc<FlowCounters>,
    plugin: Arc<str>,
    port: u16,
}

/// 流向的收端，持有那条一直开着的双向流。
#[derive(Debug)]
pub struct UdpReceiver {
    packets: mpsc::Receiver<Bytes>,
    control: TunnelStream,
    mux: Arc<DatagramMux>,
    flow: FlowId,
}

impl UdpFlow {
    pub(crate) fn new(
        mux: Arc<DatagramMux>,
        flow: FlowId,
        control: TunnelStream,
        plugin: &str,
        port: u16,
    ) -> Result<Self, PortError> {
        let (packets, counters) = mux.register(flow, plugin, port).ok_or_else(|| {
            TunnelError::protocol(format!("flow {} is already in use", flow.get()))
        })?;
        Ok(Self {
            sender: UdpSender {
                mux: mux.clone(),
                flow,
                next_packet: Arc::default(),
                counters,
                plugin: plugin.into(),
                port,
            },
            receiver: UdpReceiver {
                packets,
                control,
                mux,
                flow,
            },
        })
    }
}

impl UdpSender {
    /// 发一个 UDP 包；放不进两片时丢掉并计进 `oversized_drops`，照样返回成功（UDP 应用收不到这种反馈）。
    pub fn send(&self, payload: &[u8]) -> Result<(), PortError> {
        let packet = self.next_packet.fetch_add(1, Ordering::Relaxed);
        Ok(self.mux.send(self.flow, packet, payload, &self.counters)?)
    }

    pub fn stats(&self) -> UdpFlowStats {
        UdpFlowStats {
            flow_id: self.flow.get(),
            plugin_id: self.plugin.to_string(),
            port: self.port,
            received_packets: self.counters.received.load(Ordering::Relaxed),
            sent_packets: self.counters.sent.load(Ordering::Relaxed),
            oversized_drops: self.counters.oversized.load(Ordering::Relaxed),
            buffer_evicted_drops: None,
        }
    }
}

impl UdpReceiver {
    /// 下一个完整的包。对端结束流向、连接断开时返回错误，之后不会再有包。
    pub async fn recv(&mut self) -> Result<Bytes, PortError> {
        tokio::select! {
            biased;
            packet = self.packets.recv() => packet.ok_or_else(|| {
                TunnelError::transport("the tunnel connection carrying this flow is gone").into()
            }),
            ended = self.control.recv.read_chunk(1) => match ended {
                Ok(None) => Err(PortError::FlowEnded),
                Ok(Some(_)) => Err(TunnelError::protocol("unexpected bytes on a UDP flow stream").into()),
                Err(err) => Err(from_read_error(&err).into()),
            },
        }
    }
}

impl Drop for UdpReceiver {
    fn drop(&mut self) {
        self.mux.unregister(self.flow);
        // 正常结束：服务端读到流结束，关掉它那边的套接字。
        self.control.send.finish().ok();
    }
}
