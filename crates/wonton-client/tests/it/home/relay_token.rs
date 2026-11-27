//! 中继准入令牌：错、缺令牌时如实报“中继拒绝”，带对了经中继连上（ADR-0048）。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use iroh::test_utils::run_relay_server_with_access;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, SecretKey, endpoint::presets};
use iroh_relay::server::{Access, AccessControl, ClientRequest};
use serde_json::json;
use wonton_client::{Action, ConnectionCheck, ConnectionNotice, HomeStatus, Network};

use super::support::{Home, Phone};

const TOKEN: &str = "relay-secret";
const WAIT: Duration = Duration::from_secs(30);

/// 只放行带正确令牌的连接。
#[derive(Debug)]
struct NeedsToken;

impl AccessControl for NeedsToken {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        if request.auth_token().as_deref() == Some(TOKEN) {
            Access::Allow
        } else {
            Access::Deny {
                reason: Some("bad token".into()),
            }
        }
    }
}

fn ticket(home: &Home, relays: serde_json::Value) -> String {
    json!({
        "server": {"endpoint_id": home.key.public(), "relays": relays, "direct_addrs": []},
        "base_domain": "wonton.localhost",
    })
    .to_string()
}

#[tokio::test]
async fn a_relay_that_rejects_the_token_is_named_and_a_good_token_connects() {
    let (_, relay_url, _relay) = run_relay_server_with_access(false, Arc::new(NeedsToken))
        .await
        .unwrap();
    let key = SecretKey::generate();
    let config = RelayConfig::from(relay_url.clone()).with_auth_token(TOKEN);
    let relays: RelayMap = std::iter::once(Arc::new(config)).collect();
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .relay_mode(RelayMode::Custom(relays))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .bind()
        .await
        .unwrap();
    tokio::time::timeout(WAIT, endpoint.online()).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::with_network(dir.path(), Network::TestRelay);
    let home = Home::on(&[phone.device_id()], key, endpoint).await;
    let url = relay_url.to_string();

    // 票据不带令牌：客户端连不上中继，连接页和提示都说是令牌被拒。
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket(&home, json!([{"url": url, "tier": 0}])),
        })
        .unwrap();
    phone
        .wait_for(|view| view.details.relay == ConnectionCheck::Rejected)
        .await;
    let view = phone
        .wait_for(|view| view.notice == Some(ConnectionNotice::RelayRejected { retrying: false }))
        .await;
    assert!(
        matches!(
            view.status,
            HomeStatus::Connecting | HomeStatus::Disconnected { .. }
        ),
        "{view:?}"
    );

    // 带上正确令牌重连：经中继连上，提示消失。
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket(&home, json!([{"url": url, "token": TOKEN, "tier": 0}])),
        })
        .unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_ne!(view.details.relay, ConnectionCheck::Rejected);
    assert_eq!(view.notice, None);
}
