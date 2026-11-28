//! 第一档停掉后经第二档连上，第一档恢复后撤掉第二档。

use std::sync::Arc;

use iroh::test_utils::run_relay_server;
use iroh::{EndpointAddr, SecretKey, TransportAddr, tls::CaTlsConfig};
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer};

use super::{
    FALLBACK_AFTER, Forwarder, SharedToken, relay_endpoint, spawn_relay, timeout_get,
    wait_disconnected, wait_reestablished, wait_relay_connected,
};
use crate::support::{Phone, Upstream, WAIT};

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
