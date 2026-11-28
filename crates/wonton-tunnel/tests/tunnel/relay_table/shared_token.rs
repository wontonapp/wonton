//! 中继的共享令牌准入：带令牌能经中继连上，不带令牌被拒。

use std::sync::Arc;

use iroh::{EndpointAddr, SecretKey, TransportAddr, Watcher};
use wonton_tunnel::relays::{RelayTable, relay_map};
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelServer, TunnelStatus};

use super::{SharedToken, relay_endpoint, spawn_relay, timeout_get};
use crate::support::{Phone, Upstream, WAIT};

#[tokio::test]
async fn a_relay_with_a_shared_token_admits_endpoints_that_carry_it() -> anyhow::Result<()> {
    let (url, _relay) = spawn_relay(Arc::new(SharedToken::new("tok"))).await?;
    let table = RelayTable::parse(&format!("https://tok@{}", url.authority())).unwrap();
    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(url.clone());
    let endpoint = relay_endpoint(device, relay_map(&table.views(), 1)).await;
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
        relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let address = EndpointAddr::new(server.endpoint().id()).with_relay_url(url.clone());
    let endpoint = relay_endpoint(device, relay_map(&table.views(), 1)).await;
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
