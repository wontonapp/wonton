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
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer, TunnelStatus};

use crate::support::{Phone, Upstream, WAIT, capture_logs, init_tracing, isolated_log_test};

mod refusing_tier;
mod shared_token;
mod stopped_tier;

/// 测试里的回退阈值。iroh 第一次连上家中继多半在几百毫秒内，2 秒的阈值足够它先连上第一档。
const FALLBACK_AFTER: Duration = Duration::from_secs(2);

/// 断言“不撤回”要等的窗口。修前的 TCP 探测在拒绝中继出现后 1-3 秒内就会错判恢复并撤档
/// （1 秒轮询 + 2 秒探测限时），8 秒的窗口足够把误判等出来；修对以后这个窗口白等 8 秒。
const FLAP_WINDOW: Duration = Duration::from_secs(8);

/// 等中继重连的窗口。iroh 重连有退避，托管 CI 只有 2–4 核、测试把线程占满时偶尔超过
/// [`WAIT`] 的 15 秒（实测），这一处放得宽一些。
const RECONNECT_WAIT: Duration = Duration::from_secs(60);

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
async fn relay_fallback_log_omits_url_query() -> anyhow::Result<()> {
    const CANARY: &str = "query-canary-707";
    if isolated_log_test("relay_table::relay_fallback_log_omits_url_query") {
        return Ok(());
    }
    let logs = capture_logs("wonton_tunnel::relays=info");
    let first = Forwarder::spawn().await?;
    let second = Forwarder::spawn().await?;
    let table = RelayTable::parse(&format!(
        "{};{}?access_token={CANARY}",
        first.relay_url(),
        second.relay_url()
    ))
    .expect("synthetic relay table should parse");
    let endpoint = relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await;
    let mut fallback = spawn_tier_fallback(endpoint.clone(), table, FALLBACK_AFTER, None);
    tokio::time::timeout(WAIT, fallback.wait_enabled_tiers(2)).await?;
    let log = logs.text();
    endpoint.close().await;
    assert!(log.contains("中继表：这一档全断超阈值"));
    assert!(log.contains(&second.front.to_string()));
    assert!(
        !log.contains(CANARY),
        "relay query leaked into fallback log"
    );
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
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await;
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
    let phone_endpoint = relay_endpoint(device, relay_map(&table.views(), 1)).await;
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
/// 用比 [`WAIT`] 更宽的窗口：托管 CI 只有 2–4 核、测试并发占满时，重连偶尔超过 15 秒（实测）。
async fn wait_relay_connected(endpoint: &Endpoint, url: &RelayUrl) {
    tokio::time::timeout(RECONNECT_WAIT, async {
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

#[tokio::test]
async fn dropping_the_fallback_handle_does_not_stop_the_fallback() -> anyhow::Result<()> {
    init_tracing();
    // 第一档没有人听，第二档是健康中继；句柄丢掉以后任务还得活着，超时后把第二档加进来。
    let (_map, healthy_url, healthy) = run_relay_server().await?;
    let dead: RelayUrl = "https://127.0.0.1:1/".parse().unwrap();
    let table = RelayTable::parse(&format!("{dead};{healthy_url}")).unwrap();
    let endpoint = relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await;
    let fallback = spawn_tier_fallback(
        endpoint.clone(),
        table,
        Duration::from_millis(300),
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    drop(fallback);
    let connected = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|status| status.url() == &healthy_url && status.is_connected())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    endpoint.close().await;
    healthy.shutdown().await?;
    connected.expect("句柄 drop 后回退任务被 abort，第二档一直没加进来");
    Ok(())
}
