//! 慢中继上的前台恢复窗口（ADR-0032、#45）：一次 ping 往返 1.8 秒仍然可用时不能误判成失效；
//! 真断了要在中继的 3 秒窗口内换连接，不等 QUIC 的 30 秒空闲超时。
//!
//! 复现条件是 #45 的慢路径：客户端和服务端都清掉直连传输，只经一个本机 TCP 中继。每一程要过
//! 两跳延迟线，每跳 225 毫秒，一次 ping 往返约 1.8 秒。

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMap, RelayMode, SecretKey, endpoint::presets,
    test_utils::run_relay_server, tls::CaTlsConfig,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, tcp::OwnedReadHalf, tcp::OwnedWriteHalf},
    sync::{Mutex, Notify},
    time::timeout,
};
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelServer, TunnelStatus};

use crate::support::{Phone, Upstream, init_tracing};

/// #45 里被误判的慢路径。每一程要过两跳延迟线，一次 ping 两个往返，8 × 225 毫秒约 1.8 秒：
/// 超过快直连的 1.5 秒，仍在没有慢证据的中继窗口（3 秒）里。
const SLOW_EACH_WAY: Duration = Duration::from_millis(225);
/// 建中继和换连接后用的毫秒级延迟，别把时间花在建连上。
const FAST: Duration = Duration::from_millis(1);
/// 装死：每个方向都停这么久，窗口里什么也收不到。
const STALLED: Duration = Duration::from_secs(10);
/// 慢中继上的建连比回环直连慢，放宽等待；正常在几秒内完成。
const SLOW_WAIT: Duration = Duration::from_secs(60);

/// 可调延迟的中继前门。`stale_addrs` 的版本只在测试结束时丢掉，这里要在跑的时候把路“掐静”，
/// 所以延迟放在锁里，已经连着的转发也一起变。
struct GatedRelay {
    url: iroh::RelayUrl,
    addr: SocketAddr,
    delay: Arc<Mutex<Duration>>,
    /// 延迟改了，正在睡的转发任务重算到期时间；否则换成快挡也要等队头的旧期限走完。
    changed: Arc<Notify>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl GatedRelay {
    async fn start(upstream: SocketAddr) -> Self {
        // 转发放在专用线程自己的 runtime 上：测试 runtime 被并发的隧道测试挤住时，225 毫秒的延迟和
        // 3 秒窗口会跟着抖。stop 置位后用一个哑连接唤醒 accept，线程自己退出。
        let delay = Arc::new(Mutex::new(FAST));
        let changed = Arc::new(Notify::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let thread = {
            let delay = delay.clone();
            let changed = changed.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("gated-relay".into())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("转发线程建 runtime 失败");
                    runtime.block_on(async move {
                        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
                        addr_tx.send(listener.local_addr().unwrap()).unwrap();
                        while !stop.load(Ordering::SeqCst) {
                            let Ok((client, _)) = listener.accept().await else {
                                continue;
                            };
                            let Ok(server) = TcpStream::connect(upstream).await else {
                                continue;
                            };
                            let (client_read, client_write) = client.into_split();
                            let (server_read, server_write) = server.into_split();
                            tokio::spawn(pipe(
                                client_read,
                                server_write,
                                delay.clone(),
                                changed.clone(),
                            ));
                            tokio::spawn(pipe(
                                server_read,
                                client_write,
                                delay.clone(),
                                changed.clone(),
                            ));
                        }
                    });
                })
                .unwrap()
        };
        let addr = addr_rx.recv().expect("转发线程没起来");
        let url = format!("https://{addr}/").parse().unwrap();
        Self {
            url,
            addr,
            delay,
            changed,
            stop,
            thread: Some(thread),
        }
    }

    async fn set_delay(&self, delay: Duration) {
        *self.delay.lock().await = delay;
        self.changed.notify_waiters();
    }
}

impl Drop for GatedRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 哑连接把阻塞在 accept 上的线程叫醒；未收完的转发随线程的 runtime 一起丢掉。
        std::net::TcpStream::connect(self.addr).ok();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 一个方向的延迟线：读到的每段数据按“读到的时刻 + 当前延迟”排队，到期才转发。
/// 不在读循环里直接睡——那样同一方向的多段数据会一段接一段各睡一个延迟，吞吐塌成几千字节每秒，
/// 一次 ping 往返会磨成几十秒，测试就测不到窗口。延迟改小时，排队中的包也按新延迟重算，不挡着重拨。
async fn pipe(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    delay: Arc<Mutex<Duration>>,
    changed: Arc<Notify>,
) {
    let (sender, mut queue) =
        tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    let reader_delay = delay.clone();
    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let read = match from.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let due = tokio::time::Instant::now() + *reader_delay.lock().await;
            if sender.send((due, buffer[..read].to_vec())).is_err() {
                break;
            }
        }
    });
    while let Some((due, bytes)) = queue.recv().await {
        loop {
            let now = tokio::time::Instant::now();
            let at = due.min(now + *delay.lock().await);
            if at <= now {
                break;
            }
            tokio::select! {
                () = tokio::time::sleep_until(at) => break,
                () = changed.notified() => {}
            }
        }
        if to.write_all(&bytes).await.is_err() {
            break;
        }
    }
    reader.abort();
    to.shutdown().await.ok();
}

struct Rig {
    relay: GatedRelay,
    _relay_server: iroh_relay::server::Server,
    _upstream: Upstream,
    server: TunnelServer,
    phone: Phone,
    client: Endpoint,
}

/// 两端都只经（慢）中继。连接信息里没有直连地址，选中路径只能是中继。
async fn rig() -> Rig {
    init_tracing();
    let (_map, _url, relay_server) = run_relay_server().await.unwrap();
    let relay = GatedRelay::start(relay_server.https_addr().unwrap()).await;
    let relays: RelayMap = RelayConfig::new(relay.url.clone(), None).into();
    let device = SecretKey::generate();
    let server_endpoint = relay_endpoint(SecretKey::generate(), relays.clone()).await;
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        server_endpoint,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    timeout(SLOW_WAIT, server.endpoint().online())
        .await
        .expect("服务端要连上中继");
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(relay.url.clone());
    let client = relay_endpoint(device, relays).await;
    timeout(SLOW_WAIT, client.online())
        .await
        .expect("客户端要连上中继");
    relay.set_delay(SLOW_EACH_WAY).await;
    let phone = Phone::on(client.clone(), address, ProxyAccess::Open).await;
    Rig {
        relay,
        _relay_server: relay_server,
        _upstream: upstream,
        server,
        phone,
        client,
    }
}

/// 只经中继的 endpoint：不带任何 IP transport，直连地址无从宣告，选中的只能是中继。
/// `bind_addr` 会重新加一条 IP transport，所以这里不能用它。
async fn relay_endpoint(key: SecretKey, relays: RelayMap) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relays))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .bind()
        .await
        .unwrap()
}

fn opened_connections(server: &TunnelServer) -> u64 {
    server.endpoint().metrics().socket.num_conns_opened.get()
}

async fn connected_phone(rig: &Rig) -> Duration {
    let started = Instant::now();
    timeout(SLOW_WAIT, rig.phone.client.ping())
        .await
        .expect("慢中继上应该能建连")
        .unwrap();
    assert_eq!(rig.phone.client.path().unwrap().kind, PathKind::Relay);
    started.elapsed()
}

impl Rig {
    async fn close(self) {
        self.phone.client.close();
        self.client.close().await;
        self.server.shutdown().await;
    }
}

/// 一次 ping 往返 1.8 秒、连接仍然可用：按中继窗口等，连接留住，请求不用重来。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_relay_that_answers_is_not_replaced() {
    let rig = rig().await;
    connected_phone(&rig).await;
    assert_eq!(
        rig.phone.client.next_revalidate_window(),
        Duration::from_secs(3)
    );

    rig.phone.client.revalidate().await.unwrap();

    assert_eq!(opened_connections(&rig.server), 1);
    assert_eq!(rig.phone.get("/after").await.body, "/after");
    rig.close().await;
}

/// 同样慢、但真的没回应：3 秒窗口到点就换连接，不等 30 秒空闲超时。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_slow_relay_is_replaced_within_its_window() {
    let rig = rig().await;
    connected_phone(&rig).await;
    rig.relay.set_delay(STALLED).await;
    // 延迟线里最多还有四跳 225 毫秒的包要落地，先排空，窗口后半段再开始算。
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let mut status = rig.phone.client.watch_status();
    let started = Instant::now();
    let client = rig.phone.client.clone();
    let revalidating = tokio::spawn(async move { client.revalidate().await });

    timeout(
        SLOW_WAIT,
        status.wait_for(|status| *status == TunnelStatus::Connecting),
    )
    .await
    .expect("应该在窗口内判死并重拨")
    .unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(3),
        "中继窗口是 3 秒，不能更早换：{:?}",
        started.elapsed()
    );
    rig.relay.set_delay(FAST).await;
    timeout(SLOW_WAIT, revalidating)
        .await
        .expect("换连接应该完成")
        .unwrap()
        .unwrap();

    assert_eq!(opened_connections(&rig.server), 2);
    assert_eq!(rig.phone.get("/after").await.body, "/after");
    rig.close().await;
}
