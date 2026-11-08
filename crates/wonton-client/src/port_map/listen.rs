//! 一个映射端口上的转发：TCP 每条本机连接开一条 `Port` 流；UDP 每个来源地址一个流向，空闲两分钟关掉。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, warn};
use wonton_protocol::PortMappingState;
use wonton_tunnel::{PortError, TunnelError, UdpFlow, UdpReceiver, UdpSender};

use crate::home::HomeLink;

/// 两个方向都没有包这么久就关掉流向：同 Linux conntrack 的 UDP 流超时，和核心一侧一致（ADR-0072）。
const IDLE: Duration = Duration::from_secs(120);
const SWEEP: Duration = Duration::from_secs(10);
/// 一个端口上同时的来源地址上限。同一端口上 Moonlight 只用几个套接字，上限只防本机别的程序把流向编号用光。
const MAX_SOURCES: usize = 256;
/// 流向建好之前先攒着的包；更多的丢掉，实时流不等。
const OPENING_QUEUE: usize = 16;
/// accept 出错（例如文件描述符耗尽）后暂停多久再继续，避免空转。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

pub(super) enum Socket {
    Tcp(std::net::TcpListener),
    Udp(std::net::UdpSocket),
}

/// 监听任务共用的：取隧道客户端、报告服务器拒绝的原因。
#[derive(Clone)]
pub(super) struct Ctx {
    pub(super) home: Arc<HomeLink>,
    pub(super) plugin: Arc<str>,
    /// 最近一次开端口被服务器拒绝的原因；下一次成功时清掉。
    pub(super) refusal: Arc<Mutex<Option<PortMappingState>>>,
    pub(super) wake: Arc<Notify>,
}

impl Ctx {
    fn failed(&self, port: u16, err: &PortError) {
        let refusal = match err {
            PortError::ServerTooOld => PortMappingState::ServerTooOld,
            PortError::PluginNotInstalled => PortMappingState::PluginNotInstalled,
            PortError::PortNotDeclared => PortMappingState::PortNotDeclared,
            PortError::PluginNotRunning => PortMappingState::PluginNotRunning,
            PortError::UpstreamUnavailable => PortMappingState::PluginUnreachable,
            // 隧道断开由 `TunnelDisconnected` 显示；其余是这一条连接或流向自己的问题。
            _ => {
                debug!(plugin = %self.plugin, port, "could not open the mapped port through the tunnel: {err}");
                return;
            }
        };
        self.set_refusal(Some(refusal));
    }

    fn succeeded(&self) {
        self.set_refusal(None);
    }

    fn set_refusal(&self, refusal: Option<PortMappingState>) {
        let mut current = lock(&self.refusal);
        if *current != refusal {
            *current = refusal;
            drop(current);
            self.wake.notify_one();
        }
    }
}

pub(super) fn spawn(sockets: Vec<Socket>, ctx: &Ctx) -> Vec<JoinHandle<()>> {
    sockets
        .into_iter()
        .filter_map(|socket| match socket {
            Socket::Tcp(listener) => {
                let port = listener.local_addr().ok()?.port();
                listener.set_nonblocking(true).ok()?;
                let listener = TcpListener::from_std(listener).ok()?;
                Some(tokio::spawn(serve_tcp(listener, port, ctx.clone())))
            }
            Socket::Udp(socket) => {
                let port = socket.local_addr().ok()?.port();
                socket.set_nonblocking(true).ok()?;
                let socket = UdpSocket::from_std(socket).ok()?;
                Some(tokio::spawn(serve_udp(Arc::new(socket), port, ctx.clone())))
            }
        })
        .collect()
}

/// 任务被停时连同 JoinSet 一起丢掉，经它转发的连接一起断开。
async fn serve_tcp(listener: TcpListener, port: u16, ctx: Ctx) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, _)) => {
                    connections.spawn(relay_tcp(tcp, port, ctx.clone()));
                }
                Err(err) => {
                    warn!(port, "mapped port failed to accept a connection: {err}");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            Some(_) = connections.join_next() => {}
        }
    }
}

async fn relay_tcp(tcp: TcpStream, port: u16, ctx: Ctx) {
    let Some(client) = ctx.home.tunnel().await else {
        return;
    };
    match client.open_port(&ctx.plugin, port).await {
        Ok(stream) => {
            ctx.succeeded();
            stream.forward(tcp).await;
        }
        Err(err) => ctx.failed(port, &err),
    }
}

enum Source {
    Opening {
        generation: u64,
        queue: Vec<Vec<u8>>,
    },
    Open {
        generation: u64,
        sender: UdpSender,
        last: Arc<Mutex<Instant>>,
        _task: AbortOnDrop,
    },
}

/// 来源被移除时停掉它的回送任务，收端随之丢掉，流向结束。
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Event {
    Opened(SocketAddr, u64, Result<UdpFlow, PortError>),
    Ended(SocketAddr, u64),
}

struct UdpPort {
    socket: Arc<UdpSocket>,
    port: u16,
    ctx: Ctx,
    sources: HashMap<SocketAddr, Source>,
    events: mpsc::UnboundedSender<Event>,
    next_generation: u64,
}

async fn serve_udp(socket: Arc<UdpSocket>, port: u16, ctx: Ctx) {
    let (events, mut incoming) = mpsc::unbounded_channel();
    let mut state = UdpPort {
        socket: socket.clone(),
        port,
        ctx,
        sources: HashMap::new(),
        events,
        next_generation: 0,
    };
    let mut sweep = tokio::time::interval(SWEEP);
    let mut buf = vec![0u8; 65536];
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((n, from)) => state.packet(from, &buf[..n]),
                // 回环上的 ICMP 不可达也会报到这里（对方关了端口）；不影响别的来源。
                Err(err) => debug!(port, "mapped UDP port receive failed: {err}"),
            },
            Some(event) = incoming.recv() => state.event(event),
            _ = sweep.tick() => state.sweep(),
        }
    }
}

impl UdpPort {
    fn packet(&mut self, from: SocketAddr, payload: &[u8]) {
        let full = self.sources.len() >= MAX_SOURCES;
        match self.sources.get_mut(&from) {
            Some(Source::Opening { queue, .. }) => {
                if queue.len() < OPENING_QUEUE {
                    queue.push(payload.to_vec());
                }
            }
            Some(Source::Open { sender, last, .. }) => {
                *lock(last) = Instant::now();
                if let Err(err) = sender.send(payload) {
                    debug!(
                        port = self.port,
                        "UDP flow failed, opening a new one: {err}"
                    );
                    self.sources.remove(&from);
                }
            }
            None if full => {
                debug!(port = self.port, "too many UDP sources, dropped a packet");
            }
            None => self.open(from, payload.to_vec()),
        }
    }

    fn open(&mut self, from: SocketAddr, first: Vec<u8>) {
        let generation = self.next_generation;
        self.next_generation += 1;
        self.sources.insert(
            from,
            Source::Opening {
                generation,
                queue: vec![first],
            },
        );
        let ctx = self.ctx.clone();
        let port = self.port;
        let events = self.events.clone();
        tokio::spawn(async move {
            let opened = match ctx.home.tunnel().await {
                Some(client) => client.open_udp(&ctx.plugin, port).await,
                None => Err(PortError::Tunnel(TunnelError::Closed)),
            };
            events.send(Event::Opened(from, generation, opened)).ok();
        });
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Opened(from, generation, opened) => {
                let Some(Source::Opening { generation: g, .. }) = self.sources.get(&from) else {
                    return;
                };
                if *g != generation {
                    return;
                }
                let Some(Source::Opening { queue, .. }) = self.sources.remove(&from) else {
                    return;
                };
                match opened {
                    Ok(flow) => {
                        self.ctx.succeeded();
                        self.attach(from, generation, flow, queue);
                    }
                    Err(err) => self.ctx.failed(self.port, &err),
                }
            }
            Event::Ended(from, generation) => {
                let current = match self.sources.get(&from) {
                    Some(Source::Open { generation, .. } | Source::Opening { generation, .. }) => {
                        *generation
                    }
                    None => return,
                };
                if current == generation {
                    self.sources.remove(&from);
                }
            }
        }
    }

    fn attach(&mut self, from: SocketAddr, generation: u64, flow: UdpFlow, queue: Vec<Vec<u8>>) {
        let UdpFlow { sender, receiver } = flow;
        for packet in &queue {
            if sender.send(packet).is_err() {
                return;
            }
        }
        let last = Arc::new(Mutex::new(Instant::now()));
        let task = tokio::spawn(deliver(
            receiver,
            self.socket.clone(),
            from,
            last.clone(),
            self.events.clone(),
            generation,
        ));
        self.sources.insert(
            from,
            Source::Open {
                generation,
                sender,
                last,
                _task: AbortOnDrop(task),
            },
        );
    }

    fn sweep(&mut self) {
        let now = Instant::now();
        self.sources.retain(|_, source| match source {
            Source::Open { last, .. } => now.duration_since(*lock(last)) < IDLE,
            Source::Opening { .. } => true,
        });
    }
}

/// 把流向收到的包发回本机的来源地址；流向结束时报给端口任务。
async fn deliver(
    mut receiver: UdpReceiver,
    socket: Arc<UdpSocket>,
    to: SocketAddr,
    last: Arc<Mutex<Instant>>,
    events: mpsc::UnboundedSender<Event>,
    generation: u64,
) {
    while let Ok(packet) = receiver.recv().await {
        *lock(&last) = Instant::now();
        if let Err(err) = socket.send_to(&packet, to).await {
            debug!(%to, "could not deliver a UDP packet to the local app: {err}");
        }
    }
    events.send(Event::Ended(to, generation)).ok();
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
