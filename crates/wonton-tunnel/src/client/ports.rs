//! 客户端打开插件端口：`Port` 流和 `Udp` 流向（ADR-0072）。

use std::sync::Arc;

use iroh::endpoint::RecvStream;
use wonton_protocol::UdpFlowStats;

use super::{TunnelClient, TunnelStream, lock};
use crate::error::{TunnelError, from_read_exact_error, from_write_error};
use crate::udp::{PortError, PortStream, UdpFlow, mux::DatagramMux};
use crate::wire::{MAX_PORT_REQUEST_LEN, PortRequest, Request, Status, datagram::FlowId};

impl TunnelClient {
    /// 打开插件声明的 TCP 端口，等服务端接受后返回。
    pub async fn open_port(&self, plugin: &str, port: u16) -> Result<PortStream, PortError> {
        let stream = self.request_port(Request::Port, plugin, port).await?;
        Ok(PortStream { stream })
    }

    /// 为插件声明的 UDP 端口建一个流向；返回的 [`UdpFlow`] 收端丢掉时流向结束。
    pub async fn open_udp(&self, plugin: &str, port: u16) -> Result<UdpFlow, PortError> {
        let mut stream = self.request_port(Request::Udp, plugin, port).await?;
        let mux = self.datagram_mux(stream.conn_id)?;
        let flow = read_flow_id(&mut stream.recv).await?;
        UdpFlow::new(mux, flow, stream, plugin, port)
    }

    /// 当前连接上的 UDP 流向；没有连接时为空。
    pub fn udp_flows(&self) -> Vec<UdpFlowStats> {
        self.current_mux()
            .map(|mux| mux.flows())
            .unwrap_or_default()
    }

    /// 当前连接被发送缓冲推出的数据报帧数；这条连接上还没开过流向、或者一次也没算准过时为空。
    pub fn udp_buffer_evicted(&self) -> Option<u64> {
        self.current_mux()?.evicted()
    }

    fn current_mux(&self) -> Option<Arc<DatagramMux>> {
        let conn = self.inner.live_connection()?;
        lock(&self.inner.datagrams)
            .clone()
            .filter(|mux| mux.conn_id() == conn.stable_id())
    }

    /// 承载 `conn_id` 的连接的分发器，一条连接只有一个；连接已经换掉时按传输问题失败，不在新连接上开流向。
    fn datagram_mux(&self, conn_id: usize) -> Result<Arc<DatagramMux>, PortError> {
        let conn = self
            .inner
            .live_connection()
            .filter(|conn| conn.stable_id() == conn_id)
            .ok_or_else(|| TunnelError::transport("the connection closed while opening a flow"))?;
        if conn.max_datagram_size().is_none() {
            return Err(PortError::DatagramsUnsupported);
        }
        let mut current = lock(&self.inner.datagrams);
        if let Some(mux) = current
            .as_ref()
            .filter(|mux| mux.conn_id() == conn_id && mux.is_live())
        {
            return Ok(mux.clone());
        }
        let mux = DatagramMux::start(conn, crate::udp::reassembly::FRAGMENT_TTL, None);
        *current = Some(mux.clone());
        Ok(mux)
    }

    async fn request_port(
        &self,
        kind: Request,
        plugin: &str,
        port: u16,
    ) -> Result<TunnelStream, PortError> {
        let request = PortRequest::new(kind, plugin, port).map_err(PortError::Invalid)?;
        let mut header = [0; MAX_PORT_REQUEST_LEN];
        let len = request.encode(&mut header).map_err(PortError::Invalid)?;
        // `open` 已经写了请求类型这一个字节。旧服务端读到它就回状态并停掉这条流，头的其余部分可能写不进去：
        // 先读状态，“服务器太旧”不能被写失败盖住。
        let mut stream = self.open(kind).await?;
        let written = stream.send.write_all(&header[1..len]).await;
        self.read_port_status(&mut stream).await?;
        written.map_err(|err| from_write_error(&err))?;
        Ok(stream)
    }

    async fn read_port_status(&self, stream: &mut TunnelStream) -> Result<(), PortError> {
        let mut byte = [0u8; 1];
        stream
            .recv
            .read_exact(&mut byte)
            .await
            .map_err(|err| from_read_exact_error(&err))?;
        let status = Status::from_byte(byte[0])
            .ok_or_else(|| TunnelError::protocol(format!("unknown status byte {}", byte[0])))?;
        self.inner.confirm(stream.conn_id);
        match status {
            Status::Ok => Ok(()),
            Status::UnsupportedRequest => Err(PortError::ServerTooOld),
            Status::PluginNotInstalled => Err(PortError::PluginNotInstalled),
            Status::PortNotDeclared => Err(PortError::PortNotDeclared),
            Status::PluginNotRunning => Err(PortError::PluginNotRunning),
            Status::UpstreamUnavailable => Err(PortError::UpstreamUnavailable),
        }
    }
}

async fn read_flow_id(recv: &mut RecvStream) -> Result<FlowId, PortError> {
    let mut bytes = [0u8; 2];
    recv.read_exact(&mut bytes[..1])
        .await
        .map_err(|err| from_read_exact_error(&err))?;
    if bytes[0] >> 6 == 1 {
        recv.read_exact(&mut bytes[1..])
            .await
            .map_err(|err| from_read_exact_error(&err))?;
    }
    let (flow, _) = FlowId::decode(&bytes)
        .map_err(|err| TunnelError::protocol(format!("invalid flow identifier: {err}")))?;
    Ok(flow)
}
