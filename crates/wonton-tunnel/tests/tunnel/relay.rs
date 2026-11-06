use std::time::{Duration, Instant};

use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, SecretKey, TransportAddr, endpoint::presets,
    test_utils::run_relay_server, tls::CaTlsConfig,
};
use tokio::time::timeout;
use wonton_tunnel::{Allowlist, PathKind, ProxyAccess, TunnelServer};

use crate::support::{Phone, Upstream};

const CONNECT_WITHIN: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_stale_relay_first_still_connects_through_the_current_home_relay() {
    let (first_map, first_url, _first_relay) = run_relay_server().await.unwrap();
    let (second_map, second_url, _second_relay) = run_relay_server().await.unwrap();
    // EndpointAddr 用有序集合保存路径，让错误的中继在实际迭代顺序里也排第一。
    let (stale, current, server_map) = if first_url < second_url {
        (first_url, second_url, second_map)
    } else {
        (second_url, first_url, first_map)
    };
    let device = SecretKey::generate();
    let endpoint = relay_endpoint(SecretKey::generate(), server_map).await;
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(endpoint, Allowlist::new([device.public()]), upstream.addr);
    timeout(CONNECT_WITHIN, server.endpoint().online())
        .await
        .expect("服务端应连上唯一配置的家中继");
    let announced: Vec<_> = server.endpoint().addr().relay_urls().cloned().collect();
    assert_eq!(announced, vec![current.clone()]);
    assert_eq!(server.endpoint().addr().ip_addrs().count(), 0);

    let address = EndpointAddr::new(server.endpoint().id())
        .with_relay_url(stale.clone())
        .with_relay_url(current.clone());
    let relays: Vec<_> = address.relay_urls().cloned().collect();
    assert_eq!(relays, vec![stale.clone(), current.clone()]);
    assert_eq!(address.ip_addrs().count(), 0);
    let client_endpoint = relay_endpoint(
        device,
        RelayMap::from_iter([stale.clone(), current.clone()]),
    )
    .await;
    let started = Instant::now();
    let phone = Phone::on(client_endpoint.clone(), address, ProxyAccess::Open).await;
    let response = timeout(CONNECT_WITHIN, phone.get("/via-current-relay"))
        .await
        .expect("不能等错误中继超时才尝试正确中继");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/via-current-relay");
    let path = phone.client.path().expect("已完成的请求应有选中路径");
    assert_eq!(path.kind, PathKind::Relay);
    assert_eq!(path.remote, TransportAddr::Relay(current.clone()));
    eprintln!(
        "stale={stale} current={current} connected_in={:?} path={path:?}",
        started.elapsed()
    );

    phone.client.close();
    client_endpoint.close().await;
    server.shutdown().await;
}

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
