//! 中继表的行为测试（ADR-0048）。需要“同一地址上停掉再恢复 / 换成拒绝令牌的中继”的档，走测试
//! 自己握着的 [`Forwarder`]：表里的地址是转发器前门，从头到尾不变，后端随时切（无后端 = 中继挂了
//! 反代还活着）。中继本身一律 bind(0) 随机端口，没有先绑后放的竞态，也不重试。
//!
//! 覆盖：带令牌能经中继连上、不带令牌被拒；第一档停掉后在阈值内经第二档连上；第一档恢复后撤掉
//! 第二档；第一档换成拒绝令牌的中继后不撤回（#641）；三档时第 1 档恢复撤回一档。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use iroh::endpoint::{QuicTransportConfig, presets};
use iroh::test_utils::run_relay_server;
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr, Watcher,
    tls::CaTlsConfig,
};
use iroh_relay::server::{
    Access, AccessControl, AllowAll, CertConfig, ClientRequest, DynAccessControl,
    RelayConfig as RelayServerConfig, Server as RelayServer, ServerConfig, TlsConfig,
};
use tokio::net::{TcpListener, TcpStream};
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelServer, TunnelStatus};

use crate::support::{Phone, Upstream, WAIT, init_tracing};

/// 测试里的回退阈值。iroh 第一次连上家中继多半在几百毫秒内，2 秒的阈值足够它先连上第一档。
const FALLBACK_AFTER: Duration = Duration::from_secs(2);

/// 断言“不撤回”要等的窗口。修前的 TCP 探测在拒绝中继出现后 1-3 秒内就会错判恢复并撤档
/// （1 秒轮询 + 2 秒探测限时），8 秒的窗口足够把误判等出来；修对以后这个窗口白等 8 秒。
const FLAP_WINDOW: Duration = Duration::from_secs(8);

/// 只认 `token` 的准入：和 iroh-relay 二进制的 `shared_token` 一个做法。
#[derive(Debug)]
struct SharedToken(String);

impl SharedToken {
    fn new(token: &str) -> Self {
        Self(token.to_owned())
    }
}

impl AccessControl for SharedToken {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        match request.auth_token() {
            Some(token) if token == self.0 => Access::Allow,
            _ => Access::Deny { reason: None },
        }
    }
}

/// 在随机端口上起一个本地测试中继：bind(0) 由内核原子分配，没有先绑后放的竞态。
async fn spawn_relay(access: Arc<dyn DynAccessControl>) -> anyhow::Result<(RelayUrl, RelayServer)> {
    let (_certs, server_config) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let tls = TlsConfig::new(
        (Ipv4Addr::LOCALHOST, 0),
        CertConfig::Manual { server_config },
    );
    // TLS 和明文 HTTP（captive portal 用）是两个监听器：HTTP 给个随机端口，别和 TLS 抢。
    let mut relay = RelayServerConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(tls);
    relay.key_cache_capacity = Some(1024);
    relay.access = access;
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    let server = RelayServer::spawn(config).await?;
    let url = format!("https://{}", server.https_addr().expect("配置了 TLS 地址"))
        .parse()
        .unwrap();
    Ok((url, server))
}

/// 测试自己握着的 TCP 转发器：前门绑一次、整条测试不放，表里的地址从头到尾不变；后端随时切。
///
/// - 无后端时接受连接后立即关：中继进程挂了、反向代理还活着，正是 #641 说的那种“端口连得上、
///   中继不收我们”。
/// - 后端换成另一台中继：同一个地址上“停掉再恢复”或“换成拒绝令牌的中继”，不用固定端口，
///   也不用重试（#648 的 `spawn_relay_retrying` 是拿重试盖竞态，这里整个不成立：竞态不存在了）。
struct Forwarder {
    /// 前门地址：放进中继表的就是它。
    front: SocketAddr,
    backend: tokio::sync::watch::Sender<Option<SocketAddr>>,
    accepted: Arc<AtomicUsize>,
}

impl Forwarder {
    async fn spawn() -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let front = listener.local_addr()?;
        let (backend, mut backend_rx) = tokio::sync::watch::channel(None);
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = backend_rx.changed() => continue,
                };
                let (mut downstream, _) = match accepted {
                    Ok(accepted) => accepted,
                    // accept 的良性错误（连接在 accept 前就被对端重置等，争抢压测下很常见）不退出：
                    // 前门一关，表里的地址就再也连不上，回退判定全盘失效。歇一下再收，避免持续
                    // 出错时热转。
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                };
                counter.fetch_add(1, Ordering::Relaxed);
                let Some(backend) = *backend_rx.borrow() else {
                    continue; // 无后端：刚接受的连接随变量一起关掉
                };
                if let Ok(mut upstream) = TcpStream::connect(backend).await {
                    tokio::spawn(async move {
                        let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                    });
                }
            }
        });
        Ok(Self {
            front,
            backend,
            accepted,
        })
    }

    fn set_backend(&self, backend: Option<SocketAddr>) {
        self.backend.send_replace(backend);
    }

    fn connections(&self) -> usize {
        self.accepted.load(Ordering::Relaxed)
    }

    /// 前门的中继地址（表里用的形式）。
    fn relay_url(&self) -> RelayUrl {
        format!("https://{}", self.front).parse().unwrap()
    }
}

/// 只走本地测试中继的 endpoint：不做地址发现，不走 UDP 直连，中继表由表项给。
async fn relay_endpoint(key: SecretKey, relays: RelayMap) -> Endpoint {
    init_tracing();
    // 短空闲超时：第一档死掉后，旧连接的唯一路径跟着断，连接要在几秒内自己断开（事件），
    // 不能等 iroh 默认的 30 秒。
    let transport = QuicTransportConfig::builder()
        .max_idle_timeout(Some(Duration::from_secs(3).try_into().unwrap()))
        .keep_alive_interval(Duration::from_secs(1))
        .build();
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relays))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .transport_config(transport)
        .bind()
        .await
        .unwrap()
}

/// 中继拒绝测试的 TCP 足迹：把内核的 TIME_WAIT 和 conntrack 用量打到 stderr。CI 机上没有 shell，
/// 前后对比只能在测试进程里读 `/proc`（`--nocapture` 才看得到，通过时被测试框架收着）。
fn print_net_metrics(label: &str) {
    let time_wait = std::fs::read_to_string("/proc/net/sockstat")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("TCP:"))
                .and_then(|line| {
                    line.split_whitespace()
                        .skip_while(|word| *word != "tw")
                        .nth(1)
                        .map(str::to_owned)
                })
        })
        .unwrap_or_else(|| "?".to_owned());
    let conntrack = std::fs::read_to_string("/proc/sys/net/netfilter/nf_conntrack_count")
        .map(|text| text.trim().to_owned())
        .unwrap_or_else(|_| "?".to_owned());
    eprintln!("net metrics {label}: time_wait={time_wait} conntrack={conntrack}");
}

#[tokio::test]
async fn a_refusing_relay_bounds_main_and_probe_reconnects() -> anyhow::Result<()> {
    let (_url, refusing) = spawn_relay(Arc::new(SharedToken::new("other"))).await?;
    let backend = refusing.https_addr().expect("配置了 TLS 地址");
    let main_front = Forwarder::spawn().await?;
    main_front.set_backend(Some(backend));
    // 探针要覆盖两档被撤的中继：一个都不连上时 `home_relay_status` 是空的，修前的实现会把
    // 它们当成“没插过”每拍重插。
    let probe_front = Forwarder::spawn().await?;
    probe_front.set_backend(Some(backend));
    let second_front = Forwarder::spawn().await?;
    second_front.set_backend(Some(backend));
    // 对照：同样连这台被拒中继、但没有任何回退机制的 endpoint。它的连接全是 iroh 自己的退避和
    // 本机的网络变化报告，用来把“机器当时的噪声”从探针的连接数里除掉。
    let control_front = Forwarder::spawn().await?;
    control_front.set_backend(Some(backend));
    let (_map, healthy_url, healthy) = run_relay_server().await?;
    let single = RelayTable::parse(&format!("https://tok@{}", main_front.front)).unwrap();
    let tiered = RelayTable::parse(&format!(
        "https://tok@{};https://tok@{};{healthy_url}",
        probe_front.front, second_front.front
    ))
    .unwrap();
    assert_eq!(tiered.tier_count(), 3);
    let control_table = RelayTable::parse(&format!("https://tok@{}", control_front.front)).unwrap();
    let main = relay_endpoint(SecretKey::generate(), relay_map(single.relays(), 1)).await;
    let _main_fallback = spawn_tier_fallback(
        main.clone(),
        single,
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let tiered_endpoint =
        relay_endpoint(SecretKey::generate(), relay_map(tiered.relays(), 1)).await;
    let control = relay_endpoint(SecretKey::generate(), relay_map(control_table.relays(), 1)).await;
    let mut fallback = spawn_tier_fallback(
        tiered_endpoint.clone(),
        tiered,
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    // 探针只在回退激活后存在；落到第三档、它开始覆盖两档以后才数。
    tokio::time::timeout(WAIT, fallback.wait_enabled_tiers(3))
        .await
        .expect("拒绝令牌应回退到可用的第三档");
    let probes = || probe_front.connections() + second_front.connections();
    print_net_metrics("start");
    // 前 5 秒有建连的突发，噪声大；断言只数第 5–15 秒的稳态。
    let mut main_baseline = 0;
    let mut probe_baseline = 0;
    let mut control_baseline = 0;
    for second in 1..=15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if second == 5 {
            main_baseline = main_front.connections();
            probe_baseline = probes();
            control_baseline = control_front.connections();
        }
        eprintln!(
            "relay attempts second={second} main={} probes={} control={} main_status={:?}",
            main_front.connections(),
            probes(),
            control_front.connections(),
            main.home_relay_status().get()
        );
    }
    print_net_metrics("end");
    let main_connections = main_front.connections() - main_baseline;
    let probe_connections = probes() - probe_baseline;
    let control_connections = control_front.connections() - control_baseline;
    let main_total = main_front.connections();
    let probe_total = probes();
    main.close().await;
    control.close().await;
    tiered_endpoint.close().await;
    refusing.shutdown().await?;
    healthy.shutdown().await?;
    // 稳态窗口只断言上限：16 秒封顶的退避可能让这十秒里没有新拨号。全程计数另断言至少一次，
    // 确认转发器确实观察到了主 endpoint 和探针侧的连接。
    // 稳态窗口理论上最多几次：iroh 的退避从 10 ms 起、每次翻倍、封顶 16 s；封顶后每次间隔在 8–16 秒抖动，
    // 十秒窗口最多两次计划内重拨。61 上 netmon 的 LinkChangeMajor 会重置退避并立即重拨，这部分
    // 次数由环境决定，无法从参数推出。对照 endpoint 跑同一个 iroh actor、共享同一批网络事件，
    // 所以它这十秒的计数就是本轮噪声基线；主 endpoint 和它同样是单档单中继，只多给两秒采样
    // 对齐的余量。20 是安静机器上的绝对下限，基线更大时按 control + 2 放宽。
    let main_limit = (control_connections + 2).max(20);
    assert!(
        main_connections <= main_limit,
        "单档主 endpoint 稳态十秒连接数 {main_connections} 超过 {main_limit}（对照 {control_connections} + 2，下限 20）"
    );
    assert!(
        probe_connections <= 60,
        "恢复探针稳态十秒连接数 {probe_connections} 超过 60"
    );
    assert!(
        probe_connections <= 4 * control_connections.max(3),
        "恢复探针连接数 {probe_connections} 超过对照 endpoint（{control_connections}）的四倍"
    );
    assert!(main_total >= 1, "观察窗口内主 endpoint 没有连接");
    assert!(probe_total >= 1, "观察窗口内探针侧没有连接");
    Ok(())
}

#[tokio::test]
async fn a_relay_with_a_shared_token_admits_endpoints_that_carry_it() -> anyhow::Result<()> {
    let (url, _relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    let table = RelayTable::parse(&format!("https://tok@{}", url.authority())).unwrap();
    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(url.clone());
    let endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let phone = Phone::on(endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout_get(&phone, "/with-token").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.kind, PathKind::Relay);
    assert_eq!(path.remote, TransportAddr::Relay(url.clone()));
    phone.client.close();
    endpoint.close().await;
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_relay_with_a_shared_token_rejects_endpoints_without_it() -> anyhow::Result<()> {
    let (url, _relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    let table = RelayTable::parse(&url.to_string()).unwrap();
    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(url.clone());
    let endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let phone = Phone::on(endpoint.clone(), address, ProxyAccess::Open).await;
    // 没带令牌：中继拒绝，请求到不了服务端。
    let response = timeout_get(&phone, "/without-token").await;
    assert_ne!(response.status, 200, "{}", response.body);
    let mut status = phone.client.watch_status();
    tokio::time::timeout(
        WAIT,
        status.wait_for(|status| matches!(status, TunnelStatus::Disconnected(_))),
    )
    .await
    .expect("隧道应停在断开状态")
    .unwrap();
    // 中继连接也没建起来。
    let statuses = endpoint.home_relay_status().get();
    assert!(
        statuses.iter().all(|relay| !relay.is_connected()),
        "{statuses:?}"
    );
    phone.client.close();
    endpoint.close().await;
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_stopped_top_tier_falls_back_and_recovers_back() -> anyhow::Result<()> {
    // 第一档走转发器：表里的地址是前门，整条测试不变；中继在转发器后面，停掉和恢复都切后端。
    let first = Forwarder::spawn().await?;
    let first_url = first.relay_url();
    let (_first_relay_url, first_relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    first.set_backend(Some(first_relay.https_addr().expect("配置了 TLS 地址")));
    let (_second_map, second_url, second_relay) = run_relay_server().await.unwrap();
    let table = RelayTable::parse(&format!("https://tok@{};{second_url}", first.front)).unwrap();
    assert_eq!(table.tier_count(), 2);

    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await;
    let mut server_fallback = spawn_tier_fallback(
        server_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let server = TunnelServer::spawn(
        server_endpoint.clone(),
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let phone_endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let mut phone_fallback = spawn_tier_fallback(
        phone_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );

    // 先只拿第一档当拨号提示：连上的路径只能是第一档。拨号前等两端的 R1 传输真正连上：端点刚
    // 绑定时传输还在连，拨号撞上没就绪的传输会白等一个 DIAL_DEADLINE。
    wait_relay_connected(&phone_endpoint, &first_url).await;
    wait_relay_connected(&server_endpoint, &first_url).await;
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(first_url.clone());
    let phone = Phone::on(phone_endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout_get(&phone, "/via-first").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(first_url.clone()));
    // 第一档连上以后，两侧都该收敛在只启用第一档：起步慢、误加过第二档的话也会自己撤掉。
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(1))
        .await
        .expect("手机侧应收敛在第一档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(1))
        .await
        .expect("服务端侧应收敛在第一档");

    // 拨号提示补上第二档。第一档停掉以后：阈值一过，两侧都把第二档加进来，请求经第二档走通。
    phone.client.set_server_addr(
        EndpointAddr::new(server.endpoint().id())
            .with_relay_url(first_url.clone())
            .with_relay_url(second_url.clone()),
    );
    // 停掉第一档：后端中继关掉（旧连接的唯一路径随它断，QUIC 空闲超时会让连接自己断开，事件），
    // 前门留着一个没后端的转发器——中继进程挂了、反向代理还活着，新的拨号连上前门就被关。
    // 不主动探杀：探测返回后马上重拨会撞上 iroh 还没处理完的路径记忆，握手只发往死路径（实测）。
    first_relay.shutdown().await.ok();
    first.set_backend(None);
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(2))
        .await
        .expect("手机侧应加进第二档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(2))
        .await
        .expect("服务端侧应加进第二档");
    // 家里继换到第二档：两侧都真正连上第二档的中继。
    wait_relay_connected(&phone_endpoint, &second_url).await;
    wait_relay_connected(&server_endpoint, &second_url).await;
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    let response = timeout_get(&phone, "/via-second").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(second_url.clone()));

    // 第一档在同一个地址上恢复：后端换回一台认令牌的中继，前门没变，两侧撤掉第二档，请求回到
    // 第一档。
    let (_recovered_url, recovered_relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    first.set_backend(Some(recovered_relay.https_addr().expect("配置了 TLS 地址")));
    // iroh 重连中继有退避：等两侧都真正连上第一档（家里继状态里 R1 是 Connected）再撤档、发请求。
    wait_relay_connected(&phone_endpoint, &first_url).await;
    wait_relay_connected(&server_endpoint, &first_url).await;
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(1))
        .await
        .expect("手机侧应撤掉第二档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(1))
        .await
        .expect("服务端侧应撤掉第二档");
    // 拨号提示收回第一档：撤掉第二档以后，连上的路径只能是第一档。
    phone.client.set_server_addr(
        EndpointAddr::new(server.endpoint().id()).with_relay_url(first_url.clone()),
    );
    // iroh 的 remove_relay 只改表，不断开已经建立的连接，客户端还握着经第二档的连接。生产里高档
    // 本来就是临时兜底：把第二档的中继也停掉，旧连接的唯一路径随它一起断，QUIC 的空闲超时会让它
    // 自己断开（事件），重拨走恢复的第一档。
    second_relay.shutdown().await.ok();
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    let response = timeout_get(&phone, "/via-recovered-first").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(first_url.clone()));

    drop(recovered_relay);
    drop(first);
    phone.client.close();
    phone_endpoint.close().await;
    server_endpoint.close().await;
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_refusing_top_tier_does_not_flap() -> anyhow::Result<()> {
    // 第一档的地址活着但不收我们：中继进程在听（端口连得上），却不认我们的令牌（配额用完、令牌
    // 被吊销）。回退到第二档以后不许撤回来：端口连得上不等于中继恢复。
    // 表里放的是转发器前门：先是认令牌的中继，回退后切成拒绝令牌的中继——同一地址换成不收我们
    // 的中继，正是 #641 的来回切场景。
    let first = Forwarder::spawn().await?;
    let first_url = first.relay_url();
    let (_first_relay_url, first_relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    first.set_backend(Some(first_relay.https_addr().expect("配置了 TLS 地址")));
    let (_second_map, second_url, second_relay) = run_relay_server().await.unwrap();
    let table = RelayTable::parse(&format!("https://tok@{};{second_url}", first.front)).unwrap();
    assert_eq!(table.tier_count(), 2);

    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await;
    let mut server_fallback = spawn_tier_fallback(
        server_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let server = TunnelServer::spawn(
        server_endpoint.clone(),
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let phone_endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let mut phone_fallback = spawn_tier_fallback(
        phone_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );

    // 先经第一档连上：连接建立在第二档之前，后面断言它一直不断。
    wait_relay_connected(&phone_endpoint, &first_url).await;
    wait_relay_connected(&server_endpoint, &first_url).await;
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(first_url.clone());
    let phone = Phone::on(phone_endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout_get(&phone, "/via-first").await;
    assert_eq!(response.status, 200);

    // 第一档停掉（后端中继关掉；转发器还在听，像反代活着但后端没了），回退到第二档。
    phone.client.set_server_addr(
        EndpointAddr::new(server.endpoint().id())
            .with_relay_url(first_url.clone())
            .with_relay_url(second_url.clone()),
    );
    first_relay.shutdown().await.ok();
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(2))
        .await
        .expect("手机侧应加进第二档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(2))
        .await
        .expect("服务端侧应加进第二档");
    wait_relay_connected(&phone_endpoint, &second_url).await;
    wait_relay_connected(&server_endpoint, &second_url).await;
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    let response = timeout_get(&phone, "/via-second").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(second_url.clone()));

    // 第一档的地址上换成一个不认我们令牌的中继：端口连得上（转发器在听），握手被拒。
    // 不许判成“恢复”。
    let (_refusing_url, refusing_relay) = spawn_relay(Arc::new(SharedToken::new("other"))).await?;
    first.set_backend(Some(refusing_relay.https_addr().expect("配置了 TLS 地址")));
    let started = first.connections();
    eprintln!("refusing transition initial connections={started}");
    print_net_metrics("refusing-start");

    // 撤回判定在几秒内就该发生一次（修前的 TCP 探测就是这样错判的），等一个比它宽的窗口：
    // 窗口内两侧都不许把启用档数降回 1。预期超时，真的撤回了就是来回切。
    let no_flap = tokio::time::timeout(FLAP_WINDOW, phone_fallback.wait_enabled_tiers(1)).await;
    assert!(no_flap.is_err(), "手机侧不应撤回第二档：第一档只是端口活着");
    eprintln!(
        "refusing transition after eight seconds connections={}",
        first.connections()
    );
    let no_flap = tokio::time::timeout(FLAP_WINDOW, server_fallback.wait_enabled_tiers(1)).await;
    assert!(
        no_flap.is_err(),
        "服务端侧不应撤回第二档：第一档只是端口活着"
    );
    // 连接一直不断：隧道不许在这段时间里掉过线，窗口过后同一个连接还能发请求。
    let dropped = tokio::time::timeout(FLAP_WINDOW, wait_disconnected(&phone)).await;
    assert!(dropped.is_err(), "回退期间隧道不该断开");
    let response = timeout_get(&phone, "/via-second-still").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(second_url.clone()));
    eprintln!(
        "refusing transition final connections={} delta={}",
        first.connections(),
        first.connections() - started
    );
    print_net_metrics("refusing-end");

    phone.client.close();
    phone_endpoint.close().await;
    server_endpoint.close().await;
    server.shutdown().await;
    drop(first);
    refusing_relay.shutdown().await.ok();
    second_relay.shutdown().await.ok();
    Ok(())
}

#[tokio::test]
async fn a_recovered_middle_tier_pulls_back_one_level() -> anyhow::Result<()> {
    // 三档：0、1 档都停掉落到第 2 档，再让第 1 档恢复。撤档一次只撤一档：探针得跟着启用档数
    // 变，把新被撤的第 1 档也探起来——修前探针建好就复用，第 1 档恢复没人探，撤不回来。
    let tier0 = Forwarder::spawn().await?;
    let tier1 = Forwarder::spawn().await?;
    let (_r0_url, r0) = spawn_relay(Arc::new(SharedToken::new("tok0"))).await?;
    tier0.set_backend(Some(r0.https_addr().expect("配置了 TLS 地址")));
    let (_r1_url, r1) = spawn_relay(Arc::new(AllowAll)).await?;
    tier1.set_backend(Some(r1.https_addr().expect("配置了 TLS 地址")));
    let (_third_map, third_url, third_relay) = run_relay_server().await.unwrap();
    let table = RelayTable::parse(&format!(
        "https://tok0@{};https://{};{third_url}",
        tier0.front, tier1.front
    ))
    .unwrap();
    assert_eq!(table.tier_count(), 3);

    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await;
    let mut server_fallback = spawn_tier_fallback(
        server_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    let server = TunnelServer::spawn(
        server_endpoint.clone(),
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let phone_endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let mut phone_fallback = spawn_tier_fallback(
        phone_endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );

    // 先经第 0 档连上。
    wait_relay_connected(&phone_endpoint, &tier0.relay_url()).await;
    wait_relay_connected(&server_endpoint, &tier0.relay_url()).await;
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(tier0.relay_url());
    let phone = Phone::on(phone_endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout_get(&phone, "/via-tier0").await;
    assert_eq!(response.status, 200);

    // 0、1 档都停（后端关、前门留着没后端的转发器），两次落档到第 2 档。
    phone.client.set_server_addr(
        EndpointAddr::new(server.endpoint().id())
            .with_relay_url(tier0.relay_url())
            .with_relay_url(tier1.relay_url())
            .with_relay_url(third_url.clone()),
    );
    r0.shutdown().await.ok();
    r1.shutdown().await.ok();
    tier0.set_backend(None);
    tier1.set_backend(None);
    print_net_metrics("fallback-start");
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(3))
        .await
        .expect("手机侧应落到第三档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(3))
        .await
        .expect("服务端侧应落到第三档");
    wait_relay_connected(&phone_endpoint, &third_url).await;
    wait_relay_connected(&server_endpoint, &third_url).await;
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    let response = timeout_get(&phone, "/via-third").await;
    assert_eq!(response.status, 200);

    // 第 1 档恢复（同一前门换上新后端）：撤档一次只撤一档，第 2 档撤掉、第 1 档的路径接上。
    let (_r1b_url, r1b) = spawn_relay(Arc::new(AllowAll)).await?;
    tier1.set_backend(Some(r1b.https_addr().expect("配置了 TLS 地址")));
    tokio::time::timeout(WAIT, phone_fallback.wait_enabled_tiers(2))
        .await
        .expect("手机侧应撤回到启用两档");
    tokio::time::timeout(WAIT, server_fallback.wait_enabled_tiers(2))
        .await
        .expect("服务端侧应撤回到启用两档");
    wait_relay_connected(&phone_endpoint, &tier1.relay_url()).await;
    wait_relay_connected(&server_endpoint, &tier1.relay_url()).await;
    // 旧连接还握在经第 2 档的中继上（remove_relay 不断开已建立的连接）：把它停掉，重拨走第 1 档。
    third_relay.shutdown().await.ok();
    wait_disconnected(&phone).await;
    wait_reestablished(&phone).await;
    print_net_metrics("fallback-end");
    let response = timeout_get(&phone, "/via-recovered-tier1").await;
    assert_eq!(response.status, 200);
    let path = phone.client.path().expect("请求应有选中路径");
    assert_eq!(path.remote, TransportAddr::Relay(tier1.relay_url()));

    drop(tier0);
    drop(tier1);
    phone.client.close();
    phone_endpoint.close().await;
    server_endpoint.close().await;
    server.shutdown().await;
    Ok(())
}

async fn timeout_get(phone: &Phone, path: &str) -> crate::support::Response {
    tokio::time::timeout(WAIT, phone.get(path))
        .await
        .expect("请求超时")
}

/// 等一个中继连上：iroh 重连有退避，端口恢复可达和家里继真正连上是两件事。
async fn wait_relay_connected(endpoint: &Endpoint, url: &RelayUrl) {
    tokio::time::timeout(WAIT, async {
        loop {
            let statuses = endpoint.home_relay_status().get();
            if statuses
                .iter()
                .any(|relay| relay.url() == url && relay.is_connected())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("等中继重连超时");
}

/// 等客户端的隧道连接自己断开（状态订阅）：中继停掉后旧连接的唯一路径跟着断，QUIC 的空闲超时
/// 会让连接关闭，客户端状态翻成 Disconnected。重拨要等这件事：iroh 在最后一条连接关闭时才清掉
/// 「记住的路径」，清掉之前重拨只会把握手发往记住的路径。
async fn wait_disconnected(phone: &Phone) {
    let mut status = phone.client.watch_status();
    tokio::time::timeout(WAIT, async move {
        loop {
            if matches!(&*status.borrow_and_update(), TunnelStatus::Disconnected(_)) {
                return;
            }
            if status.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .expect("等旧连接自己断开超时");
}

/// 重验断开旧连接后，等客户端重拨连上再发请求：iroh 重拨可能先撞上它记住的死路径（旧连接的
/// 关闭事件还没传播完，iroh 这时只往记住的路径发握手），而失败的拨号会让客户端的退避一路增长，
/// 光重复 ping 只会在退避里越等越久。所以每次先 [`retry_now`](TunnelClient::retry_now) 清零退避
/// 再 ping：撞上死路径的那次超时返回后，关闭事件已传播完，下一次拨号走全路径复制收敛。
/// 订阅客户端状态等结果，不靠 sleep，也不把单次限时调长。
async fn wait_reestablished(phone: &Phone) {
    let mut status = phone.client.watch_status();
    tokio::time::timeout(WAIT, async move {
        loop {
            phone.client.retry_now();
            let _ = phone.client.ping().await;
            if matches!(&*status.borrow_and_update(), TunnelStatus::Connected) {
                return;
            }
        }
    })
    .await
    .expect("重拨应在限时内连上");
}
