//! 取地址失败：自动补取、过时标记先真后假，补取不重建连接；一直取不到连接照样能用。

use std::net::SocketAddr;
use std::time::Duration;

use wonton_client::HomeStatus;

use super::support::{Home, Phone, WEB_URL, get, wait_until_ticket_has};

/// 取地址失败一次（换网、机器忙）：自动补取，过时标记先真后假，连接不用重建。
#[tokio::test]
async fn a_failed_address_refresh_is_retried_and_clears_the_stale_mark() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let lan: SocketAddr = "192.168.1.2:18482".parse().unwrap();
    let public: SocketAddr = "203.0.113.2:18482".parse().unwrap();
    *home.tunnel_info.lock().unwrap() = Some(
        serde_json::json!({
            "endpoint_id": home.key.public(),
            "relays": [],
            "direct_addrs": [lan, public],
        })
        .to_string(),
    );
    home.fail_next_tunnel(1);
    phone.connect(&home);

    let stale = phone.wait_for(|view| view.details.addresses_stale).await;
    assert_eq!(stale.status, HomeStatus::Connected);

    wait_until_ticket_has(dir.path(), public).await;
    let view = phone.wait_for(|view| !view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1,
        "补取不该重新建连"
    );
}

/// 一直取不到：过时标记一直亮，连接和其它请求照常，退避期间也不会把连接拖下来。
#[tokio::test]
async fn a_failing_address_refresh_keeps_the_connection_usable() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.report_address("203.0.113.2:18482".parse().unwrap());
    home.fail_next_tunnel(usize::MAX);
    phone.connect(&home);

    let view = phone.wait_for(|view| view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
    let proxy = view.proxy.clone().unwrap();
    let (status, body) = get(&proxy, WEB_URL, "/while-stale", true).await;
    assert_eq!((status, body.as_str()), (200, "/while-stale"));

    // 退避是 1、2、4 秒：这段时间里又取过两次，标记不能自己掉。
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let view = phone.home();
    assert!(view.details.addresses_stale, "{view:?}");
    assert_eq!(view.status, HomeStatus::Connected);
    let (status, _) = get(&proxy, WEB_URL, "/still-stale", true).await;
    assert_eq!(status, 200, "{:?}", phone.home());
}
