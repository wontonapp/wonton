//! 卡住的路径：连接没断，QUIC 也没判定超时，但请求到不了对端（#154）。

use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{EndpointAddr, SecretKey};
use tokio::{net::UdpSocket, task::JoinSet};
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelError, TunnelStatus};

use crate::support::{Home, Phone, Timeouts, Upstream, endpoint_on};

/// 判定连接卡住的时限：包全丢了的测试用，到时限才算结果，机器再忙也不会提前。
const DEADLINE: Duration = Duration::from_secs(1);
/// 连接活着的测试用：机器忙时一次往返也可能超过 [`DEADLINE`]，那会被当成卡住换掉连接。
const PATIENT: Duration = Duration::from_secs(30);

// 本组测试的时限都是墙钟（[`DEADLINE`]、[`PATIENT`]、revalidate 的 8 秒、support 的 15 秒），而
// 手机和服务端两端、本地代理、上游和 [`Flaky`] 转发全在同一个 runtime 上。单线程 runtime 只有一个
// 调度单位，CI 机器上别的任务把这一颗核占住几秒，所有任务（包括每包 900ms 的延迟转发）就停几秒，
// 一次往返会被拖过 8 秒和 15 秒（2026-11-06 CI 偶发：慢路径保持连接那条测试的 GET 15 秒没读完）。
// 多线程 runtime 只要还有一个 worker 被调度，转发和驱动就照常前进，延迟回到 900ms×2 加毫秒级处理，
// 各时限的余量重新成立。每个测试四个 worker，任务基本都是等待 I/O，全套并发时线程数也可控。

/// 手机和服务端之间的一段 UDP 转发，可以丢包或延迟。
struct Flaky {
    addr: SocketAddr,
    /// 大于这么多字节的包丢掉。
    limit: Arc<AtomicUsize>,
    delay_ms: Arc<AtomicUsize>,
    /// 还要丢掉多少个上行（手机→服务端）方向的包，把丢包钉在指定的请求上。
    drop_up_next: Arc<AtomicUsize>,
    /// 转发线程的停止标记，Drop 时置位。
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

const PASS: usize = usize::MAX;
const DROP_ALL: usize = 0;

impl Flaky {
    async fn start(target: SocketAddr) -> Self {
        // 转发放到专用线程自己的 runtime 上：测试的 runtime 在高负载下会被 OS 饿几秒（CI 偶发的根因），
        // 每包 900ms 的延迟转发跟着失准，一次往返被拖过测试的墙钟时限。专用线程由 OS 独立调度，
        // 延迟精度不再依赖测试 runtime 的忙碌程度。Drop 时发一个哑包唤醒 recv_from，线程看到
        // 停止标记就退出。
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let limit = Arc::new(AtomicUsize::new(PASS));
        let shared = limit.clone();
        let delay_ms = Arc::new(AtomicUsize::new(0));
        let delay = delay_ms.clone();
        let drop_up_next = Arc::new(AtomicUsize::new(0));
        let dropping_up = drop_up_next.clone();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("flaky-forwarder".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("flaky 转发线程建 runtime 失败");
                runtime.block_on(async move {
                    let front = Arc::new(UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap());
                    let back = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
                    addr_tx.send(front.local_addr().unwrap()).expect("flaky addr");
                    let mut phone = None;
                    let (mut up, mut down) = (vec![0u8; 65536], vec![0u8; 65536]);
                    let mut delayed = JoinSet::new();
                    loop {
                        if stopping.load(Ordering::SeqCst) {
                            break;
                        }
                        tokio::select! {
                            received = front.recv_from(&mut up) => {
                                if stopping.load(Ordering::SeqCst) {
                                    break;
                                }
                                let (len, from) = received.unwrap();
                                phone = Some(from);
                                if dropping_up.swap(0, Ordering::SeqCst) > 0 {
                                    continue;
                                }
                                if len <= shared.load(Ordering::SeqCst) {
                                    forward_packet(&mut delayed, &back, target, &up[..len], &delay)
                                        .await;
                                }
                            }
                            received = back.recv_from(&mut down) => {
                                if stopping.load(Ordering::SeqCst) {
                                    break;
                                }
                                let (len, _) = received.unwrap();
                                if let Some(phone) = phone && len <= shared.load(Ordering::SeqCst) {
                                    forward_packet(&mut delayed, &front, phone, &down[..len], &delay)
                                        .await;
                                }
                            }
                            Some(result) = delayed.join_next() => { result.unwrap(); }
                        }
                    }
                });
            })
            .expect("flaky 转发线程启动失败");
        let addr = addr_rx.recv().expect("flaky 转发线程没回地址");
        Self {
            addr,
            limit,
            delay_ms,
            drop_up_next,
            stop,
            thread: Some(thread),
        }
    }

    fn drop_above(&self, bytes: usize) {
        self.limit.store(bytes, Ordering::SeqCst);
    }

    fn delay_each_way(&self, millis: usize) {
        self.delay_ms.store(millis, Ordering::SeqCst);
    }

    /// 丢掉接下来第一个上行（手机→服务端）方向的包。在 revalidate 之后布下，就把丢包钉在
    /// 下一个请求的首包上。
    fn drop_next_up(&self) {
        self.drop_up_next.store(1, Ordering::SeqCst);
    }
}

impl Drop for Flaky {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 哑包唤醒 front 的 recv_from，让转发线程走到循环顶部的停止检查。
        if let Ok(waker) = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)) {
            let _ = waker.send_to(&[0u8], self.addr);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

async fn forward_packet(
    delayed: &mut JoinSet<()>,
    socket: &Arc<UdpSocket>,
    target: SocketAddr,
    bytes: &[u8],
    delay: &AtomicUsize,
) {
    let millis = delay.load(Ordering::SeqCst);
    if millis == 0 {
        socket.send_to(bytes, target).await.ok();
        return;
    }
    let socket = socket.clone();
    let bytes = bytes.to_vec();
    delayed.spawn(async move {
        tokio::time::sleep(Duration::from_millis(millis as u64)).await;
        socket.send_to(&bytes, target).await.ok();
    });
}

/// 手机只绑 IPv6，服务端只绑 IPv4，两边只能经 [`Flaky`] 互通：服务端会经 QUIC 把自己绑定的地址告诉手机，同一族的话手机
/// 打洞直接连过去，绕开丢包，请求照样通（CI 上偶发，机器忙时路径选择才偏向它）。
async fn rig() -> (Upstream, Home, Flaky, Phone) {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let home = Home::start(SecretKey::generate(), allowlist, upstream.addr).await;
    let port = home.server().endpoint().bound_sockets()[0].port();
    let flaky = Flaky::start((Ipv4Addr::LOCALHOST, port).into()).await;
    let server = EndpointAddr::new(home.addr().id).with_ip_addr(flaky.addr);
    let endpoint = endpoint_on((Ipv6Addr::LOCALHOST, 0).into(), device, Timeouts::Default).await;
    let phone = Phone::on(endpoint, server, ProxyAccess::Open).await;
    (upstream, home, flaky, phone)
}

/// 网络回来时手上还有一次注定失败的握手：改之前要等它重传或超时，实测恢复后 32 秒才通（ADR-0032）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retrying_does_not_wait_for_a_dial_started_on_the_dead_network() {
    let (_upstream, _home, flaky, phone) = rig().await;
    flaky.drop_above(DROP_ALL);
    let client = phone.client.clone();
    let dialling = tokio::spawn(async move { client.ping().await });
    // QUIC 的初始包重传间隔加倍，这时离下一次重传还有一阵。
    let on_the_dead_network = Duration::from_secs(5);
    tokio::time::sleep(on_the_dead_network).await;

    flaky.drop_above(PASS);
    phone.client.retry_now();
    dialling.await.unwrap().unwrap();

    // 断网时开始的那次握手，到现在至少用了这么久；比它短，说明连上的是重试新开的那次。
    let handshake = phone.client.stats().handshake.unwrap();
    assert!(handshake < on_the_dead_network, "{handshake:?}");
}

/// 包全丢了，QUIC 要 30 秒才判定超时，这期间连接算活着，路径还是最后一次估的“直连 2 ms”。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_connection_stops_claiming_to_be_connected() {
    let (_upstream, home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    assert_eq!(phone.client.path().unwrap().kind, PathKind::Direct);

    flaky.drop_above(DROP_ALL);
    let err = phone.client.probe(DEADLINE).await.unwrap_err();

    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    assert!(
        matches!(phone.client.status(), TunnelStatus::Disconnected(_)),
        "{:?}",
        phone.client.status()
    );
    assert_eq!(phone.client.path(), None);

    // 网络恢复：下一次检查换上新连接，状态和路径回来。
    flaky.drop_above(PASS);
    phone.client.probe(PATIENT).await.unwrap();
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(phone.client.path().unwrap().kind, PathKind::Direct);
    assert_eq!(phone.get("/after").await.status, 200);
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 2);
}

/// 已经挂在卡住的连接上的请求，断开时马上失败，归为传输问题，调用方可以重试。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_on_a_stalled_connection_fail_when_it_is_dropped() {
    let (_upstream, _home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);

    flaky.drop_above(DROP_ALL);
    let client = phone.client.clone();
    let stuck = tokio::spawn(async move { client.ping().await });
    tokio::task::yield_now().await;
    phone.client.probe(DEADLINE).await.unwrap_err();

    let err = stuck.await.unwrap().unwrap_err();
    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
}

/// 检查不拖慢新请求：还在等回应时开的流照常走现有连接。连接活着就不换。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probing_a_live_connection_keeps_it() {
    let (_upstream, home, _flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);

    phone.client.probe(PATIENT).await.unwrap();

    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(phone.get("/after").await.status, 200);
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 1);
}

/// 已经挂在慢证据上的路径，revalidate 要按证据放宽窗口（8 秒）并保住活连接（ADR-0032、#405）。
/// 时序钉法：假延迟只负责让两次 probe 记下慢证据（每趟 ≥1.8 秒，只会更慢不会更快），决策本身用
/// [`TunnelClient::next_revalidate_window`] 断言——它不依赖墙钟，机器再忙也不会失真；撤掉假延迟后
/// 再 revalidate 和 GET，往返回到毫秒级，行为断言（连接数 1、请求走完）不受调度饥饿影响。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revalidating_a_proven_slow_path_keeps_the_connection() {
    let (_upstream, home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    flaky.delay_each_way(900);
    for _ in 0..2 {
        phone.client.probe(PATIENT).await.unwrap();
    }
    assert_eq!(
        phone.client.next_revalidate_window(),
        Duration::from_secs(8)
    );

    flaky.delay_each_way(0);
    phone.client.revalidate().await.unwrap();

    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1
    );
    assert_eq!(phone.get("/after").await.body, "/after");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_proven_slow_path_is_still_replaced_when_it_dies() {
    let (_upstream, home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    flaky.delay_each_way(900);
    for _ in 0..2 {
        phone.client.probe(PATIENT).await.unwrap();
    }
    flaky.delay_each_way(0);
    flaky.drop_above(DROP_ALL);
    let mut status = phone.client.watch_status();
    let client = phone.client.clone();
    let revalidating = tokio::spawn(async move { client.revalidate().await });

    // 不等 QUIC 的 30 秒空闲超时；看到主动重拨再恢复网络，而不是猜握手何时开始。
    tokio::time::timeout(
        Duration::from_secs(20),
        status.wait_for(|s| *s == TunnelStatus::Connecting),
    )
    .await
    .unwrap()
    .unwrap();
    flaky.drop_above(PASS);
    tokio::time::timeout(PATIENT, revalidating)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        2
    );
    assert_eq!(phone.get("/after").await.body, "/after");
}

/// 2026-11-06 CI 偶发的那次失败卡在这里：revalidate 确认了证明过慢的活路径之后，GET 的首包丢了
/// （当时的机器负载把测试的 runtime 饿了约 10 秒，包在转发里滞留、丢失，靠重传 11 秒后才到），请求
/// 15 秒没读完。契约是请求要能在同一条连接上走完（ADR-0032），不是换连接重来。时序用两处钩子钉住：
/// 假延迟只负责让两次 probe 记下慢证据（每趟 ≥1.8 秒，只会更慢不会更快），决策本身用
/// [`TunnelClient::next_revalidate_window`] 断言，不依赖墙钟；之后撤掉假延迟再丢包、发请求——首包
/// 丢了请求要在 [`WAIT`](crate::support::WAIT) 内靠 QUIC 重传走完，且连接不能换。#405 把固定 1.5 秒
/// 改成按证据放宽之前，窗口断言必然失败，之后必然通过。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_on_a_proven_slow_path_complete_after_a_lost_first_packet() {
    let (_upstream, home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    flaky.delay_each_way(900);
    for _ in 0..2 {
        phone.client.probe(PATIENT).await.unwrap();
    }
    assert_eq!(
        phone.client.next_revalidate_window(),
        Duration::from_secs(8)
    );

    flaky.delay_each_way(0);
    phone.client.revalidate().await.unwrap();
    flaky.drop_next_up();

    assert_eq!(phone.get("/after").await.body, "/after");
    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1
    );
}
