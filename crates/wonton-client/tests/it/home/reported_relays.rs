//! 服务器报告的中继表：带坏项整份不用、留旧表；换了令牌下次建 endpoint 用新的。

use std::net::SocketAddr;
use std::time::Duration;

use wonton_client::{Action, HomeStatus};

use super::support::{Home, Phone, WEB_URL, wait_until_ticket_contains};

fn ticket_with_relay(home: &Home, relays: serde_json::Value) -> String {
    let addr = home.server().endpoint().bound_sockets()[0];
    serde_json::json!({
        "server": {"endpoint_id": home.key.public(), "relays": relays, "direct_addrs": [addr]},
        "base_domain": "wonton.localhost",
        "web_url": WEB_URL,
    })
    .to_string()
}

/// 核心报来的中继表带着坏项：整份不用、留旧表，过时标记亮着等重试，连接不受影响。
#[tokio::test]
async fn an_invalid_reported_relay_keeps_the_saved_table() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::ConnectHome {
                ticket: ticket_with_relay(
                    &home,
                    serde_json::json!([{"url": "https://a.example/", "token": "tok", "tier": 0}]),
                ),
            },
        })
        .unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    let moved: SocketAddr = "203.0.113.9:18482".parse().unwrap();
    *home.tunnel_info.lock().unwrap() = Some(
        serde_json::json!({
            "endpoint_id": home.key.public(),
            "relays": [{"url": "http://b.example/", "tier": 0}],
            "direct_addrs": [moved],
        })
        .to_string(),
    );
    let view = phone.wait_for(|view| view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let text = std::fs::read_to_string(dir.path().join("servers/1/home-ticket.json")).unwrap();
    assert!(text.contains("https://a.example/"), "{text}");
    assert!(text.contains("tok"), "{text}");
    assert!(!text.contains(&moved.to_string()), "{text}");
}

/// 核心换了中继令牌：下一次报告就存进连接信息，下次建 endpoint 用新的。
#[tokio::test]
async fn a_reported_relay_token_replaces_the_saved_one() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let addr = home.server().endpoint().bound_sockets()[0];
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::ConnectHome {
                ticket: ticket_with_relay(
                    &home,
                    serde_json::json!([{"url": "https://a.example/", "tier": 0}]),
                ),
            },
        })
        .unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    *home.tunnel_info.lock().unwrap() = Some(
        serde_json::json!({
            "endpoint_id": home.key.public(),
            "relays": [{"url": "https://a.example/", "token": "new", "tier": 0}],
            "direct_addrs": [addr],
        })
        .to_string(),
    );
    wait_until_ticket_contains(dir.path(), "\"token\":\"new\"").await;
    let view = phone.wait_for(|view| !view.details.addresses_stale).await;
    assert_eq!(view.status, HomeStatus::Connected);
}
