//! 中继表的行为测试（ADR-0048）：两个本地测试中继，第一档开 shared_token 准入。带令牌能经中继
//! 连上，不带令牌被拒；第一档停掉以后在阈值内经第二档连上；第一档恢复后撤掉第二档。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{QuicTransportConfig, presets};
use iroh::test_utils::run_relay_server;
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr, Watcher,
    tls::CaTlsConfig,
};
use iroh_relay::server::{
    Access, AccessControl, CertConfig, ClientRequest, DynAccessControl,
    RelayConfig as RelayServerConfig, Server as RelayServer, ServerConfig, TlsConfig,
};
use tokio::net::TcpListener;
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelServer, TunnelStatus};

use crate::support::{Phone, Upstream, WAIT};

/// 测试里的回退阈值。iroh 第一次连上家中继多半在几百毫秒内，2 秒的阈值足够它先连上第一档。
const FALLBACK_AFTER: Duration = Duration::from_secs(2);

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

/// 在 `port`（0 为系统分配）上起一个本地测试中继。
///
/// 先把端口握在手里再生成证书：证书要几毫秒，先放手的话端口会被并发的 `bind(0)` 抢走；握住，
/// 生成完立刻 `drop` 再让 iroh 绑，空窗只有微秒级。
async fn spawn_relay(
    port: u16,
    access: Arc<dyn DynAccessControl>,
) -> anyhow::Result<(RelayUrl, RelayServer)> {
    let held = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let port = held.local_addr()?.port();
    let (_certs, server_config) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let tls = TlsConfig::new(
        (Ipv4Addr::LOCALHOST, port),
        CertConfig::Manual { server_config },
    );
    // TLS 和明文 HTTP（captive portal 用）是两个监听器：HTTP 给个随机端口，别和 TLS 抢。
    let mut relay = RelayServerConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(tls);
    relay.key_cache_capacity = Some(1024);
    relay.access = access;
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    drop(held);
    let server = RelayServer::spawn(config).await?;
    let url = format!("https://{}", server.https_addr().expect("配置了 TLS 地址"))
        .parse()
        .unwrap();
    Ok((url, server))
}

/// 放手到 iroh 绑之间的空窗被同进程的别的 `bind(0)` 抢走时重来一次（最多 50 次；固定端口时顺便
/// 等旧的监听器关掉）。
async fn spawn_relay_retrying(
    port: u16,
    access: Arc<dyn DynAccessControl>,
) -> anyhow::Result<(RelayUrl, RelayServer)> {
    for _ in 0..50 {
        match spawn_relay(port, access.clone()).await {
            Ok(spawned) => return Ok(spawned),
            Err(err) if format!("{err:#}").contains("in use") => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(err) => return Err(err),
        }
    }
    anyhow::bail!("端口 {port} 一直绑不上")
}

/// 只走本地测试中继的 endpoint：不做地址发现，不走 UDP 直连，中继表由表项给。
async fn relay_endpoint(key: SecretKey, relays: RelayMap) -> Endpoint {
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

#[tokio::test]
async fn a_relay_with_a_shared_token_admits_endpoints_that_carry_it() -> anyhow::Result<()> {
    let (url, _relay) = spawn_relay_retrying(0, Arc::new(SharedToken::new("tok"))).await?;
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
    let (url, _relay) = spawn_relay_retrying(0, Arc::new(SharedToken::new("tok"))).await?;
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
    // 第一档的端口固定，停掉以后能在同一个地址上恢复。挑内核临时端口段之外的端口：中继停掉以后
    // 临时端口马上会被别的 bind(0) 拿走，撤档用的 TCP 探测就会把无关服务当成“低档恢复”。
    let port = {
        let mut found = None;
        for candidate in 24100..24200u16 {
            if TcpListener::bind((Ipv4Addr::LOCALHOST, candidate))
                .await
                .is_ok()
            {
                found = Some(candidate);
                break;
            }
        }
        found.ok_or_else(|| anyhow::anyhow!("24100..24200 里找不到空端口"))?
    };
    let (first_url, first_relay) =
        spawn_relay_retrying(port, Arc::new(SharedToken::new("tok"))).await?;
    let (_second_map, second_url, second_relay) = run_relay_server().await.unwrap();
    let table = RelayTable::parse(&format!("https://tok@127.0.0.1:{port};{second_url}")).unwrap();
    assert_eq!(table.tier_count(), 2);

    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint = relay_endpoint(SecretKey::generate(), relay_map(table.relays(), 1)).await;
    let mut server_fallback =
        spawn_tier_fallback(server_endpoint.clone(), table.clone(), FALLBACK_AFTER);
    let server = TunnelServer::spawn(
        server_endpoint.clone(),
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let phone_endpoint = relay_endpoint(device, relay_map(table.relays(), 1)).await;
    let mut phone_fallback =
        spawn_tier_fallback(phone_endpoint.clone(), table.clone(), FALLBACK_AFTER);

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
    // 停掉第一档：要显式关。旧连接的唯一路径随第一档的传输一起断，QUIC 的空闲超时会让它自己
    // 断开：订阅客户端状态等它断（事件），再重拨。不主动探杀：探测返回后马上重拨会撞上 iroh
    // 还没处理完的路径记忆，握手只发往死路径（实测）。
    first_relay.shutdown().await.ok();
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

    // 第一档在同一个地址上恢复：低档的端口连得上，两侧撤掉第二档，请求回到第一档。
    let (_recovered_url, recovered_relay) =
        spawn_relay_retrying(port, Arc::new(SharedToken::new("tok"))).await?;
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
