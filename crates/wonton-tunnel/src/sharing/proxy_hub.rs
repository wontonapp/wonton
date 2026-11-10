//! Android has a process-global proxy address. Credentials select a source on
//! this listener; changing the active account never changes another page's route.

use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use super::{frame, proxy::validated_domain};
use crate::{ProxyCredential, http_front};

type Routes = Arc<Mutex<BTreeMap<u64, Arc<Route>>>>;

#[derive(Debug)]
struct Route {
    credential: ProxyCredential,
    downstream: SocketAddr,
    downstream_credential: ProxyCredential,
    domain: Arc<str>,
    revoked: watch::Sender<bool>,
}

#[derive(Debug)]
pub struct ProxyHub {
    local_addr: SocketAddr,
    routes: Routes,
    next: AtomicU64,
    task: JoinHandle<()>,
}

/// Keep this handle for the lifetime of a page/source. Drop revokes its credential
/// and interrupts all in-flight downstream sockets, including upgraded WebSockets.
#[derive(Debug)]
pub struct ProxyRegistration {
    id: u64,
    local_addr: SocketAddr,
    credential: ProxyCredential,
    routes: Routes,
    route: Arc<Route>,
}

impl ProxyHub {
    pub async fn bind() -> io::Result<Arc<Self>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let local_addr = listener.local_addr()?;
        let routes = Arc::new(Mutex::new(BTreeMap::new()));
        let task = tokio::spawn(accept_loop(listener, routes.clone()));
        Ok(Arc::new(Self {
            local_addr,
            routes,
            next: AtomicU64::new(1),
            task,
        }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Downstream must be a credential-protected LocalProxy or ShareLocalProxy.
    /// Its authorization is sent only to that loopback listener, never into HTTP
    /// forwarded to core. Each registration gets a distinct random credential.
    pub fn register(
        &self,
        downstream: SocketAddr,
        downstream_credential: ProxyCredential,
        domain: String,
    ) -> io::Result<ProxyRegistration> {
        if !downstream.ip().is_loopback() || downstream.port() == 0 || downstream == self.local_addr
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid downstream proxy",
            ));
        }
        let domain: Arc<str> = validated_domain(&domain)?.into();
        let credential = ProxyCredential::generate();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let route = Arc::new(Route {
            credential: credential.clone(),
            downstream,
            downstream_credential,
            domain,
            revoked: watch::channel(false).0,
        });
        lock(&self.routes).insert(id, route.clone());
        Ok(ProxyRegistration {
            id,
            local_addr: self.local_addr,
            credential,
            routes: self.routes.clone(),
            route,
        })
    }
}

impl Drop for ProxyHub {
    fn drop(&mut self) {
        for route in lock(&self.routes).values() {
            route.revoked.send_replace(true);
        }
        lock(&self.routes).clear();
        self.task.abort();
    }
}

impl ProxyRegistration {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    pub fn credential(&self) -> &ProxyCredential {
        &self.credential
    }
}

impl Drop for ProxyRegistration {
    fn drop(&mut self) {
        // Remove before signalling: new sockets cannot race into the revoked route.
        lock(&self.routes).remove(&self.id);
        self.route.revoked.send_replace(true);
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn accept_loop(listener: TcpListener, routes: Routes) {
    let mut sockets = JoinSet::new();
    // 走中枢时本地入口是这里，下游 LocalProxy 只见到 CONNECT；测试按这个序号还原客户端发请求的顺序。
    #[cfg(feature = "test-hooks")]
    static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    loop {
        tokio::select! {
            Some(_) = sockets.join_next(), if !sockets.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((tcp, _)) = accepted else { break; };
                if sockets.len() < 256 {
                    sockets.spawn(serve(
                        tcp,
                        routes.clone(),
                        #[cfg(feature = "test-hooks")]
                        NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                    ));
                }
            }
        }
    }
}

async fn serve(tcp: TcpStream, routes: Routes, #[cfg(feature = "test-hooks")] sequence: u64) {
    let (mut read, mut write) = tcp.into_split();
    let accepted = timeout(
        frame::DEADLINE,
        http_front::accept_selected(
            &mut read,
            &mut write,
            |header| {
                lock(&routes)
                    .values()
                    .find(|route| route.credential.matches(header))
                    .map(|route| (route.clone(), route.domain.clone()))
            },
            #[cfg(feature = "test-hooks")]
            sequence,
        ),
    )
    .await;
    let Ok(Some((route, prefix))) = accepted else {
        return;
    };
    // Subscribe before inspecting, so removal cannot be missed between the two.
    let mut revoked = route.revoked.subscribe();
    if *revoked.borrow_and_update() {
        return;
    }
    let Ok(mut tcp) = read.reunite(write) else {
        return;
    };
    tokio::select! {
        biased;
        _ = revoked.changed() => {},
        _ = relay(&mut tcp, &route, &prefix) => {},
    }
}

async fn relay(tcp: &mut TcpStream, route: &Route, prefix: &[u8]) -> io::Result<()> {
    let connected = timeout(frame::DEADLINE, connect_downstream(route)).await;
    let mut downstream = match connected {
        Ok(Ok(stream)) => stream,
        _ => {
            tcp.write_all(b"HTTP/1.1 502 Bad Gateway\r\nx-wonton-tunnel-error: share-unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await?;
            return Err(io::Error::other("downstream proxy unavailable"));
        }
    };
    downstream.write_all(prefix).await?;
    tokio::io::copy_bidirectional(tcp, &mut downstream).await?;
    Ok(())
}

async fn connect_downstream(route: &Route) -> io::Result<TcpStream> {
    let mut tcp = TcpStream::connect(route.downstream).await?;
    let credential = &route.downstream_credential;
    let encoded = STANDARD.encode(format!(
        "{}:{}",
        credential.username(),
        credential.password()
    ));
    let request = format!(
        "CONNECT {}:80 HTTP/1.1\r\nHost: {}:80\r\nProxy-Authorization: Basic {encoded}\r\n\r\n",
        route.domain, route.domain
    );
    // Cancellation must abort an upgraded downstream socket, not merely half-close
    // its upload and leave its download waiting indefinitely.
    tcp.set_zero_linger()?;
    tcp.write_all(request.as_bytes()).await?;
    // Consume only the bounded CONNECT response header, never application bytes.
    let mut bytes = Vec::with_capacity(256);
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() == 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "downstream proxy response too large",
            ));
        }
        tcp.read_exact(&mut byte).await?;
        bytes.push(byte[0]);
    }
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut response = httparse::Response::new(&mut headers);
    if response.parse(&bytes).is_err() || response.code != Some(200) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "downstream proxy refused",
        ));
    }
    Ok(tcp)
}
