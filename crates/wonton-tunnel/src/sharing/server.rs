use std::{fmt, sync::Arc};

use iroh::{
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::{task::JoinSet, time::timeout};
use wonton_protocol::{MatrixOpenIdToken, ShareIdentityFailure, ShareReply, ShareRequest};

use super::{
    ShareError, ShareHandler, ShareLimits, ShareRefusal, ShareSession, ShareStream, frame,
};

/// Connection limits are independent from core's per-endpoint connection rate limit.
#[derive(Clone)]
pub struct ShareProtocol {
    handler: Arc<dyn ShareHandler>,
    limits: ShareLimits,
}

impl fmt::Debug for ShareProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShareProtocol")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl ShareProtocol {
    pub fn new(handler: Arc<dyn ShareHandler>, limits: ShareLimits) -> Self {
        assert!(
            limits.max_active_streams > 0,
            "sharing needs at least one stream slot"
        );
        Self { handler, limits }
    }

    async fn authenticate(&self, conn: &Connection) -> Result<Arc<ShareSession>, ShareError> {
        let (mut send, mut recv) = conn.accept_bi().await.map_err(|_| ShareError::Transport)?;
        let authenticate = async {
            let token = match control_request(&mut recv).await? {
                ShareRequest::Authenticate { openid } => openid,
                _ => return reject_identity(&mut send, conn).await,
            };
            if !valid_token(&token) {
                return reject_identity(&mut send, conn).await;
            }
            match self.handler.authenticate(token).await {
                Ok(auth) if !auth.shares.user_id.is_empty() => {
                    let session = Arc::new(ShareSession {
                        user_id: auth.shares.user_id.as_str().into(),
                        generation: auth.generation,
                    });
                    frame::write(
                        &mut send,
                        &ShareReply::Shares {
                            shares: auth.shares,
                        },
                        frame::MAX_REPLY,
                    )
                    .await?;
                    Ok(session)
                }
                Ok(_) => Err(ShareError::Protocol),
                Err(refusal) => {
                    reply_refusal(&mut send, refusal).await?;
                    // Let the peer consume the refusal before closing the connection.
                    conn.closed().await;
                    Err(ShareError::Refused(refusal))
                }
            }
        };
        tokio::pin!(authenticate);
        tokio::select! {
            biased;
            _ = conn.accept_bi() => Err(ShareError::Protocol),
            result = &mut authenticate => result,
        }
    }

    async fn serve(&self, conn: &Connection, user: Arc<ShareSession>) {
        let mut streams = JoinSet::new();
        let permits = Arc::new(tokio::sync::Semaphore::new(
            self.limits.max_active_streams as usize,
        ));
        loop {
            tokio::select! {
                _ = conn.closed() => break,
                Some(_) = streams.join_next(), if !streams.is_empty() => {},
                accepted = conn.accept_bi() => {
                    let Ok((mut send, mut recv)) = accepted else { break; };
                    let Ok(permit) = permits.clone().try_acquire_owned() else {
                        send.reset(1u32.into()).ok();
                        recv.stop(1u32.into()).ok();
                        continue;
                    };
                    let handler = self.handler.clone();
                    let connection = conn.clone();
                    let user = user.clone();
                    streams.spawn(async move {
                        serve_stream(handler, connection, user, send, recv, permit).await;
                    });
                }
            }
        }
        // JoinSet drop aborts all HTTP handlers before this connection task exits.
    }
}

impl ProtocolHandler for ShareProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        conn.set_max_concurrent_bi_streams(self.limits.max_active_streams.into());
        if let Ok(Ok(user)) = timeout(frame::DEADLINE, self.authenticate(&conn)).await {
            self.serve(&conn, user).await;
        }
        conn.close(0u32.into(), b"share closed");
        Ok(())
    }
}

fn valid_token(token: &MatrixOpenIdToken) -> bool {
    !token.access_token.expose().is_empty()
        && !token.matrix_server_name.is_empty()
        && token.expires_in > 0
}

async fn control_request(recv: &mut RecvStream) -> Result<ShareRequest, ShareError> {
    let mut tag = [0];
    recv.read_exact(&mut tag)
        .await
        .map_err(|_| ShareError::Protocol)?;
    if tag[0] != frame::CONTROL {
        return Err(ShareError::Protocol);
    }
    frame::read(recv, frame::MAX_REQUEST).await
}

async fn reply_refusal(send: &mut SendStream, refusal: ShareRefusal) -> Result<(), ShareError> {
    frame::write(
        send,
        &ShareReply::Refused {
            refusal: refusal.refusal,
            identity: refusal.identity,
        },
        frame::MAX_REPLY,
    )
    .await
}

async fn reject_identity(
    send: &mut SendStream,
    conn: &Connection,
) -> Result<Arc<ShareSession>, ShareError> {
    let refusal = ShareRefusal::identity(ShareIdentityFailure::InvalidToken);
    reply_refusal(send, refusal).await?;
    conn.closed().await;
    Err(ShareError::Refused(refusal))
}

async fn serve_stream(
    handler: Arc<dyn ShareHandler>,
    conn: Connection,
    user: Arc<ShareSession>,
    mut send: SendStream,
    mut recv: RecvStream,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let mut tag = [0];
    if !matches!(
        timeout(frame::DEADLINE, recv.read_exact(&mut tag)).await,
        Ok(Ok(()))
    ) {
        send.reset(1u32.into()).ok();
        recv.stop(1u32.into()).ok();
        return;
    }
    if tag[0] == frame::FORWARD {
        let stream =
            ShareStream::new(conn.clone(), user.user_id.clone(), send, recv).with_permit(permit);
        handler.forward(&user, conn, stream).await;
        return;
    }
    let control = async {
        if tag[0] != frame::CONTROL {
            return Err(ShareError::Protocol);
        }
        match frame::read(&mut recv, frame::MAX_REQUEST).await? {
            ShareRequest::List => match handler.list(&user).await {
                Ok(shares) if shares.user_id == user.user_id() => {
                    frame::write(&mut send, &ShareReply::Shares { shares }, frame::MAX_REPLY).await
                }
                Ok(_) => Err(ShareError::Protocol),
                Err(refusal) => reply_refusal(&mut send, refusal).await,
            },
            // Reauthentication never runs, even if a valid token is provided.
            _ => {
                reply_refusal(
                    &mut send,
                    ShareRefusal::identity(ShareIdentityFailure::InvalidToken),
                )
                .await
            }
        }
    };
    if !matches!(timeout(frame::DEADLINE, control).await, Ok(Ok(()))) {
        send.reset(1u32.into()).ok();
        recv.stop(1u32.into()).ok();
    }
}
