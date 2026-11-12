//! 服务端：在 [`PAIR_ALPN`] 上接受还没登记的设备，读一条请求、交给 [`PairHandler`]、回一条回应。

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use iroh::endpoint::{Connection, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, info};

use super::{MAX_MESSAGE_LEN, PairRequest, PairResponse, PairingSecret, RejectReason};

/// 等设备开流、写完请求的时间：未登记的设备占不住连接。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 回完以后等设备先关连接的时间。服务端先关的话，回应还没被确认就可能连同连接一起丢掉。
const LINGER: Duration = Duration::from_secs(5);

const CLOSE_DONE: VarInt = VarInt::from_u32(0);

/// 决定一台设备能不能配对：核心按发出去的码登记设备，或者说明为什么拒绝。
pub trait PairHandler: Send + Sync + Debug + 'static {
    /// `device` 是 QUIC 握手认证过的设备 endpoint ID。
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse;
}

/// 挂在 iroh router 的 [`PAIR_ALPN`](super::PAIR_ALPN) 上的配对协议。
#[derive(Debug)]
pub struct PairProtocol<H> {
    handler: Arc<H>,
}

impl<H> Clone for PairProtocol<H> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
        }
    }
}

impl<H: PairHandler> PairProtocol<H> {
    pub fn new(handler: Arc<H>) -> Self {
        Self { handler }
    }

    async fn respond(&self, conn: &Connection) -> Result<(), String> {
        let (mut send, mut recv) = tokio::time::timeout(REQUEST_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| "设备没有及时开流".to_owned())?
            .map_err(|e| e.to_string())?;
        let request = tokio::time::timeout(REQUEST_TIMEOUT, recv.read_to_end(MAX_MESSAGE_LEN))
            .await
            .map_err(|_| "设备没有及时写完请求".to_owned())?;
        let response = match request.map_err(|e| e.to_string()).and_then(|bytes| {
            serde_json::from_slice::<PairRequest>(&bytes).map_err(|e| e.to_string())
        }) {
            Ok(PairRequest::Pair {
                secret,
                device_name,
            }) => self
                .handler
                .pair(conn.remote_id(), &secret, device_name.trim()),
            Err(reason) => PairResponse::Rejected {
                reason: RejectReason::BadRequest,
                message: format!("配对请求不合规：{reason}"),
            },
        };
        let body = serde_json::to_vec(&response).expect("回应总能编成 JSON");
        send.write_all(&body).await.map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl<H: PairHandler> ProtocolHandler for PairProtocol<H> {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        match self.respond(&conn).await {
            Ok(()) => {
                // 等设备读完回应后自己关；超时就由服务端关。
                tokio::time::timeout(LINGER, conn.closed()).await.ok();
            }
            Err(reason) => debug!(remote = %remote.fmt_short(), "配对请求没有处理完：{reason}"),
        }
        info!(remote = %remote.fmt_short(), "配对连接结束");
        conn.close(CLOSE_DONE, b"done");
        Ok(())
    }
}
