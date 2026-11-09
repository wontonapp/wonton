//! 服务器配对的服务端：在 [`PEER_PAIR_ALPN`](super::PEER_PAIR_ALPN) 上接受对方开的一条双向流，读一条
//! [`PeerMessage`](super::PeerMessage)、交给 [`PeerPairHandler`]、回一条 [`PeerReply`](super::PeerReply)。
//!
//! 决定和问结果都只认 QUIC 握手认证的对方 endpoint ID：冒充者拿不到别人扫过的结果。

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use iroh::endpoint::{Connection, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, info};

use super::{MAX_MESSAGE_LEN, PeerMessage, PeerReply};

/// 等对方开流、写完消息的时间：没完没了的请求占不住连接。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 回完以后等对方先关连接的时间。先关的话，回应还没被确认就可能连同连接一起丢掉。
const LINGER: Duration = Duration::from_secs(5);

const CLOSE_DONE: VarInt = VarInt::from_u32(0);

/// 决定一次服务器配对交互怎么做：出码方按 [`PeerPairHandler::scanned`] 验令牌，扫码方只认自己扫过的
/// endpoint ID 发来的配对结果，删除通知原样转交，问结果按自己的请求表答。
pub trait PeerPairHandler: Send + Sync + Debug + 'static {
    /// 有人扫了我们的码。`scanner` 是 QUIC 握手认证过的对方 endpoint ID，不可伪造。
    fn scanned(
        &self,
        scanner: EndpointId,
        request: &super::PeerIdentity,
        token: &super::PeerPairToken,
    ) -> PeerReply;

    /// 扫码方来问自己那条请求的结果。`scanner` 是 QUIC 握手认证过的对方 endpoint ID。
    fn outcome(&self, scanner: EndpointId) -> PeerReply;

    /// 我们在等这个 endpoint ID 的配对结果吗。结果只认我们自己扫过的那个对方（ADR-0050），别的来源一律不理。
    fn expecting_decision(&self, from: &EndpointId) -> bool;

    /// 我们扫的对方把主人的决定告诉我们了（已通过 [`Self::expecting_decision`] 校验）。
    fn decision(&self, from: EndpointId, approved: bool, identity: &super::PeerIdentity);

    /// 对方删了和我们的配对。`from` 是 QUIC 握手认证过的对方 endpoint ID。
    fn removed(&self, from: EndpointId, server_name: &str);
}

/// 挂在 iroh router 的 [`PEER_PAIR_ALPN`](super::PEER_PAIR_ALPN) 上的服务器配对协议。
#[derive(Debug)]
pub struct PeerPairProtocol<H> {
    handler: Arc<H>,
}

impl<H> Clone for PeerPairProtocol<H> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
        }
    }
}

impl<H: PeerPairHandler> PeerPairProtocol<H> {
    pub fn new(handler: Arc<H>) -> Self {
        Self { handler }
    }

    async fn respond(&self, conn: &Connection) -> Result<(), String> {
        let (mut send, mut recv) = tokio::time::timeout(REQUEST_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| "对方没有及时开流".to_owned())?
            .map_err(|e| e.to_string())?;
        let request = tokio::time::timeout(REQUEST_TIMEOUT, recv.read_to_end(MAX_MESSAGE_LEN))
            .await
            .map_err(|_| "对方没有及时写完消息".to_owned())?;
        let reply = match request.map_err(|e| e.to_string()).and_then(|bytes| {
            serde_json::from_slice::<PeerMessage>(&bytes).map_err(|e| e.to_string())
        }) {
            Ok(message) => self.dispatch(conn.remote_id(), message),
            Err(reason) => PeerReply::Error {
                reason: super::PeerRejectReason::BadRequest,
                message: format!("服务器配对的消息不合规：{reason}"),
            },
        };
        let body = serde_json::to_vec(&reply).expect("回应总能编成 JSON");
        send.write_all(&body).await.map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn dispatch(&self, remote: EndpointId, message: PeerMessage) -> PeerReply {
        match message {
            PeerMessage::Scan { token, identity } => {
                self.handler.scanned(remote, &identity, &token)
            }
            PeerMessage::Outcome => self.handler.outcome(remote),
            PeerMessage::Decision { approved, identity } => {
                if !self.handler.expecting_decision(&remote) {
                    debug!(from = %remote.fmt_short(), "不理不是自己扫过的服务器的配对结果");
                    return PeerReply::Error {
                        reason: super::PeerRejectReason::CodeRevoked,
                        message: "扫码方不在等这个来源的配对结果".to_owned(),
                    };
                }
                self.handler.decision(remote, approved, &identity);
                PeerReply::Ok
            }
            PeerMessage::Removed { server_name } => {
                self.handler.removed(remote, &server_name);
                PeerReply::Ok
            }
        }
    }
}

impl<H: PeerPairHandler> ProtocolHandler for PeerPairProtocol<H> {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        match self.respond(&conn).await {
            Ok(()) => {
                // 等对方读完回应后自己关；超时就由本端关。
                tokio::time::timeout(LINGER, conn.closed()).await.ok();
            }
            Err(reason) => {
                debug!(remote = %remote.fmt_short(), "服务器配对的消息没有处理完：{reason}")
            }
        }
        info!(remote = %remote.fmt_short(), "服务器配对连接结束");
        conn.close(CLOSE_DONE, b"done");
        Ok(())
    }
}
