use std::sync::Arc;

use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::Connection};
use tokio::{io::AsyncWriteExt, net::TcpStream, time::timeout};
use wonton_protocol::{MatrixOpenIdToken, ShareReply, ShareRequest, SharedPlugins};

use super::{SHARE_ALPN, ShareError, ShareRefusal, ShareStream, frame};

/// One source connection. Clones share the same immutable identity and never
/// reconnect. A new connection requires a newly supplied OpenID token.
#[derive(Clone, Debug)]
pub struct ShareClient(Arc<Session>);

#[derive(Debug)]
struct Session {
    _endpoint: Endpoint,
    connection: Connection,
    initial: SharedPlugins,
    user: Arc<str>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.connection.close(0u32.into(), b"share closed");
    }
}

impl ShareClient {
    /// `target` must come from HTTPS discovery with a valid certificate. Iroh
    /// authenticates its endpoint key; that alone does not make discovery trusted.
    pub async fn connect(
        endpoint: Endpoint,
        target: EndpointAddr,
        fresh_token: MatrixOpenIdToken,
    ) -> Result<Self, ShareError> {
        let connection = timeout(frame::DEADLINE, endpoint.connect(target, SHARE_ALPN))
            .await
            .map_err(|_| ShareError::Timeout)?
            .map_err(|_| ShareError::Transport)?;
        let request = ShareRequest::Authenticate {
            openid: fresh_token,
        };
        let result = timeout(frame::DEADLINE, exchange(&connection, &request))
            .await
            .map_err(|_| ShareError::Timeout)
            .and_then(|result| result);
        // The token-bearing request is dropped here, not stored in Session.
        drop(request);
        let initial = match result {
            Ok(shares) if !shares.user_id.is_empty() => shares,
            Ok(_) => {
                connection.close(0u32.into(), b"share closed");
                return Err(ShareError::Protocol);
            }
            Err(error) => {
                connection.close(0u32.into(), b"share closed");
                return Err(error);
            }
        };
        let user = initial.user_id.as_str().into();
        Ok(Self(Arc::new(Session {
            _endpoint: endpoint,
            connection,
            initial,
            user,
        })))
    }

    pub fn initial_shares(&self) -> &SharedPlugins {
        &self.0.initial
    }
    pub fn user_id(&self) -> &str {
        &self.0.user
    }
    pub fn remote_id(&self) -> EndpointId {
        self.0.connection.remote_id()
    }
    pub fn connection_id(&self) -> usize {
        self.0.connection.stable_id()
    }
    pub fn is_closed(&self) -> bool {
        self.0.connection.close_reason().is_some()
    }
    pub fn close(&self) {
        self.0.connection.close(0u32.into(), b"share closed");
    }
    /// A close monitor without a background task or a cached reconnect token.
    pub async fn closed(&self) {
        self.0.connection.closed().await;
    }

    pub async fn list(&self) -> Result<SharedPlugins, ShareError> {
        if self.is_closed() {
            return Err(ShareError::Closed);
        }
        let result = timeout(
            frame::DEADLINE,
            exchange(&self.0.connection, &ShareRequest::List),
        )
        .await
        .map_err(|_| ShareError::Timeout)?;
        match result {
            Ok(shares) if shares.user_id == self.user_id() => Ok(shares),
            Ok(_) => {
                self.close();
                Err(ShareError::Protocol)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn open_forward(&self) -> Result<ShareStream, ShareError> {
        if self.is_closed() {
            return Err(ShareError::Closed);
        }
        let opening = async {
            let (mut send, recv) = self
                .0
                .connection
                .open_bi()
                .await
                .map_err(|_| ShareError::Transport)?;
            send.write_all(&[frame::FORWARD])
                .await
                .map_err(|_| ShareError::Transport)?;
            Ok(ShareStream::new(
                self.0.connection.clone(),
                self.0.user.clone(),
                send,
                recv,
            ))
        };
        timeout(frame::DEADLINE, opening)
            .await
            .map_err(|_| ShareError::Timeout)?
    }

    /// Raw authenticated forwarding for a caller that already validated its local
    /// HTTP proxy credentials. This is not a public unauthenticated TCP listener.
    pub async fn forward_raw_tcp(&self, mut tcp: TcpStream) -> Result<(), ShareError> {
        let mut stream = self.open_forward().await?;
        tokio::io::copy_bidirectional(&mut tcp, &mut stream)
            .await
            .map_err(|_| ShareError::Transport)?;
        stream.shutdown().await.map_err(|_| ShareError::Transport)
    }
}

async fn exchange(conn: &Connection, request: &ShareRequest) -> Result<SharedPlugins, ShareError> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|_| ShareError::Transport)?;
    send.write_all(&[frame::CONTROL])
        .await
        .map_err(|_| ShareError::Transport)?;
    frame::write(&mut send, request, frame::MAX_REQUEST).await?;
    match frame::read(&mut recv, frame::MAX_REPLY).await? {
        ShareReply::Shares { shares } => Ok(shares),
        ShareReply::Refused { refusal, identity } => {
            Err(ShareError::Refused(ShareRefusal { refusal, identity }))
        }
        ShareReply::Other => Err(ShareError::Protocol),
    }
}
