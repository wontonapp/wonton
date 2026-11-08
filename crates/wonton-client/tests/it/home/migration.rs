use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use iroh::{Endpoint, SecretKey, endpoint::presets, test_utils::run_relay_server};
use serde_json::json;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_client::{Action, HomeStatus, Network};

use super::support::{Home, Phone, WEB_URL, get, wait_until_ticket_has};

const WAIT: Duration = Duration::from_secs(30);

struct DirectGate {
    addr: SocketAddr,
    enabled: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl DirectGate {
    async fn start(server: SocketAddr) -> Self {
        let front = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let back = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        back.connect(server).await.unwrap();
        let addr = front.local_addr().unwrap();
        let (enabled, mut state) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut remote = None;
            let mut incoming = [0u8; 65536];
            let mut outgoing = [0u8; 65536];
            loop {
                tokio::select! {
                    packet = front.recv_from(&mut incoming) => {
                        let (len, peer) = packet.unwrap();
                        remote = Some(peer);
                        if *state.borrow() {
                            back.send(&incoming[..len]).await.unwrap();
                        }
                    }
                    packet = back.recv(&mut outgoing) => {
                        let len = packet.unwrap();
                        if let Some(peer) = remote.filter(|_| *state.borrow()) {
                            front.send_to(&outgoing[..len], peer).await.unwrap();
                        }
                    }
                    changed = state.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Self {
            addr,
            enabled,
            task,
        }
    }
}

impl Drop for DirectGate {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn a_direct_path_selected_after_the_report_survives_a_relay_outage_and_restart() {
    later_direct_path_survives_restart(false).await;
}

#[tokio::test]
async fn an_empty_report_keeps_a_later_direct_path_across_restart() {
    later_direct_path_survives_restart(true).await;
}

async fn later_direct_path_survives_restart(empty_report: bool) {
    let (relays, relay_url, relay) = run_relay_server().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::with_network(dir.path(), Network::TestRelay);
    let key = SecretKey::generate();
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .clear_ip_transports()
        .bind_addr((Ipv6Addr::LOCALHOST, 0))
        .unwrap()
        .relay_mode(iroh::RelayMode::Custom(relays))
        .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
        .bind()
        .await
        .unwrap();
    tokio::time::timeout(WAIT, endpoint.online()).await.unwrap();
    let gate = DirectGate::start(endpoint.bound_sockets()[0]).await;
    endpoint.add_external_addr(gate.addr).await;
    let home = Home::on(&[phone.device_id()], key, endpoint).await;
    let dead = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let reported = dead.local_addr().unwrap();
    *home.tunnel_info.lock().unwrap() = Some(
        json!({
            "endpoint_id": home.key.public(),
            "relays": [{"url": relay_url, "tier": 0}],
            "direct_addrs": [reported],
        })
        .to_string(),
    );
    let ticket = json!({
        "server": {"endpoint_id": home.key.public(), "relays": [{"url": relay_url, "tier": 0}], "direct_addrs": []},
        "base_domain": "wonton.localhost", "web_url": WEB_URL,
    });
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket.to_string(),
        })
        .unwrap();
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected && view.path.is_some_and(|path| path.relay)
        })
        .await;
    wait_until_ticket_has(dir.path(), reported).await;
    phone.wait_for(|view| !view.details.addresses_stale).await;
    let path = dir.path().join("servers/1/home-ticket.json");
    let initial: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(initial["server"]["direct_addrs"], json!([reported]));
    if empty_report {
        *home.tunnel_info.lock().unwrap() = Some(
            json!({
                "endpoint_id": home.key.public(),
                "relays": [{"url": relay_url, "tier": 0}],
                "direct_addrs": [],
            })
            .to_string(),
        );
    }

    gate.enabled.send_replace(true);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected && view.path.is_some_and(|path| !path.relay)
        })
        .await;
    wait_until_ticket_has(dir.path(), gate.addr).await;
    if empty_report {
        let updated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(updated["server"]["direct_addrs"][0], json!(gate.addr));
        assert!(
            updated["server"]["direct_addrs"]
                .as_array()
                .unwrap()
                .contains(&json!(reported))
        );
    }
    relay.shutdown().await.unwrap();
    drop(phone);
    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    assert!(view.path.is_some_and(|path| !path.relay));
    let (status, body) = get(&view.proxy.unwrap(), WEB_URL, "/after-relay-outage", true).await;
    assert_eq!((status, body.as_str()), (200, "/after-relay-outage"));
    drop(restarted);
    home.server().endpoint().close().await;
}
