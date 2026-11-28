//! 三档时第 1 档恢复：撤档一次只撤一档，探针跟着启用档数走。

use std::sync::Arc;

use iroh::test_utils::run_relay_server;
use iroh::{EndpointAddr, SecretKey, TransportAddr, tls::CaTlsConfig};
use iroh_relay::server::AllowAll;
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer};

use super::{
    FALLBACK_AFTER, Forwarder, SharedToken, print_net_metrics, relay_endpoint, spawn_relay,
    timeout_get, wait_disconnected, wait_reestablished, wait_relay_connected,
};
use crate::support::{Phone, Upstream, WAIT};

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
