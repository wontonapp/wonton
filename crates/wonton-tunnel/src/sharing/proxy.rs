use std::{io, net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use super::{ShareClient, frame};
use crate::{ProxyCredential, http_front};

/// A source-specific loopback HTTP proxy. Credentials are supplied by the caller,
/// never derived from OpenID or shared between source connections.
#[derive(Debug)]
pub struct ShareLocalProxy {
    local_addr: SocketAddr,
    credential: ProxyCredential,
    task: JoinHandle<()>,
}

impl ShareLocalProxy {
    pub async fn bind(
        client: ShareClient,
        addr: SocketAddr,
        credential: ProxyCredential,
        domain: String,
    ) -> io::Result<Self> {
        let domain = validated_domain(&domain)?;
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "share proxy requires loopback",
            ));
        }
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let access = Arc::new((credential.clone(), domain));
        let task = tokio::spawn(accept_loop(listener, client, access));
        Ok(Self {
            local_addr,
            credential,
            task,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    pub fn credential(&self) -> &ProxyCredential {
        &self.credential
    }
    pub async fn shutdown(mut self) {
        self.task.abort();
        (&mut self.task).await.ok();
    }
}

impl Drop for ShareLocalProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) fn validated_domain(domain: &str) -> io::Result<String> {
    let domain = domain.trim_end_matches('.');
    let valid = !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && matches!(url::Host::parse(domain), Ok(url::Host::Domain(_)));
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid share proxy domain",
        ));
    }
    Ok(domain.to_ascii_lowercase())
}

async fn accept_loop(
    listener: TcpListener,
    client: ShareClient,
    access: Arc<(ProxyCredential, String)>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = client.closed() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((tcp, _)) = accepted else { break; };
                if connections.len() >= 64 { continue; }
                connections.spawn(serve(tcp, client.clone(), access.clone()));
            }
        }
    }
}

async fn serve(tcp: TcpStream, client: ShareClient, access: Arc<(ProxyCredential, String)>) {
    let (mut read, mut write) = tcp.into_split();
    let prefix = match timeout(
        frame::DEADLINE,
        http_front::accept(
            &mut read,
            &mut write,
            &access.0,
            &access.1,
            #[cfg(feature = "test-hooks")]
            0,
        ),
    )
    .await
    {
        Ok(Some(prefix)) => prefix,
        _ => return,
    };
    let Ok(mut stream) = client.open_forward().await else {
        write.write_all(b"HTTP/1.1 502 Bad Gateway\r\nx-wonton-tunnel-error: share-unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.ok();
        return;
    };
    if stream.write_all(&prefix).await.is_err() {
        return;
    }
    let Ok(mut tcp) = read.reunite(write) else {
        return;
    };
    if copy_bidirectional(&mut tcp, &mut stream).await.is_ok() {
        stream.shutdown().await.ok();
    }
}
