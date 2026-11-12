use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::embed::PeerStream;

/// A raw HTTP duplex with an immutable authenticated Matrix user. Dropping without
/// shutdown resets its send direction; dropping always stops its receive direction.
/// The inner PeerStream owns that lifecycle; there is no second QUIC adapter.
#[derive(Debug)]
pub struct ShareStream {
    inner: PeerStream,
    connection: Connection,
    user_id: Arc<str>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl ShareStream {
    pub(super) fn new(
        connection: Connection,
        user_id: Arc<str>,
        send: SendStream,
        recv: RecvStream,
    ) -> Self {
        Self {
            inner: PeerStream::new(connection.remote_id(), send, recv),
            connection,
            user_id,
            _permit: None,
        }
    }

    pub(super) fn with_permit(mut self, permit: tokio::sync::OwnedSemaphorePermit) -> Self {
        self._permit = Some(permit);
        self
    }

    /// Transport identity only, never the authenticated Matrix identity.
    pub fn remote_id(&self) -> EndpointId {
        self.inner.remote_id()
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    pub fn connection_id(&self) -> usize {
        self.connection.stable_id()
    }
    /// Closing a session interrupts every stream on it, not just this duplex.
    pub fn close_connection(&self) {
        self.connection.close(0u32.into(), b"share closed");
    }
}

impl AsyncRead for ShareStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for ShareStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
