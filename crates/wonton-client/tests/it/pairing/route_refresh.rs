use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde_json::json;
use wonton_client::HomeStatus;
use wonton_tunnel::pair::PairingCode;

use super::support::{Home, Phone, WAIT, paired};

#[tokio::test]
async fn reported_addresses_do_not_erase_the_working_route_after_pairing() {
    reconnects_after_report(1).await;
}

#[tokio::test]
async fn a_full_report_cannot_evict_the_working_route_after_pairing() {
    reconnects_after_report(8).await;
}

async fn reconnects_after_report(count: usize) {
    let home = Home::start().await;
    let unreachable: Vec<_> = (0..count)
        .map(|_| std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
        .collect();
    let reported: Vec<_> = unreachable
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect();
    let mut code = PairingCode::parse(&home.code(None)).unwrap();
    let connected = code.direct_addrs[0];
    let stale: SocketAddr = "192.0.2.10:18482".parse().unwrap();
    code.direct_addrs.push(stale);
    *home.codes.reported_addrs.lock().unwrap() = reported.clone();
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&code.to_uri()).unwrap();
    phone.wait_for(paired).await;
    let ticket_path = dir.path().join("servers/1/home-ticket.json");
    tokio::time::timeout(WAIT, async {
        loop {
            let saved = std::fs::read_to_string(&ticket_path).unwrap();
            if saved.contains(&reported[0].to_string()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("核心报来的地址没有存进连接信息");
    let device = phone.home().device_id;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ticket_path).unwrap()).unwrap();
    drop(phone);

    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();
    let response = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(format!("http://{}:{}", proxy.host, proxy.port))
                .unwrap()
                .basic_auth(&proxy.username, &proxy.password),
        )
        .timeout(WAIT)
        .build()
        .unwrap()
        .get("http://wonton.localhost:18480/after-restart")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap(),
        json!({"device": device})
    );
    assert!(saved["server"]["relays"].as_array().unwrap().is_empty());
    assert!(
        !saved["server"]["direct_addrs"]
            .as_array()
            .unwrap()
            .contains(&json!(stale))
    );
    let expected: Vec<_> = std::iter::once(connected)
        .chain(reported.iter().copied())
        .take(8)
        .collect();
    assert_eq!(saved["server"]["direct_addrs"], json!(expected));
}
