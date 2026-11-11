//! 手动重试要检查现有连接：QUIC 还能确认包，但服务端已经不回答请求时，不能把按钮永远留在进度中。

use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{Connection, presets};
use tokio::task::JoinSet;
use wonton_client::{Action, HomeStatus};
use wonton_tunnel::wire::{ALPN, Request, Status};

use super::Phone;

async fn first_ping_only(connection: Connection) {
    let mut answered = false;
    let mut held = Vec::new();
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let mut kind = [0];
        if recv.read_exact(&mut kind).await.is_err() {
            continue;
        }
        if kind[0] == Request::Ping as u8 && !answered {
            send.write_all(&[Status::Ok as u8]).await.unwrap();
            send.finish().unwrap();
            answered = true;
        } else {
            // 保留流和连接，不发状态字节：传输本身活着不代表新的请求有回应。
            held.push((send, recv));
        }
    }
}

#[tokio::test]
async fn retry_replaces_a_live_transport_that_no_longer_answers_requests() {
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    endpoint.set_alpns(vec![ALPN.to_vec()]);
    let serving = endpoint.clone();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        while let Some(incoming) = serving.accept().await {
            connections.spawn(async move {
                let connection = incoming.accept().unwrap().await.unwrap();
                first_ping_only(connection).await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    phone.dispatch(Action::ConnectHome {
        ticket: format!(
            r#"{{"server":{{"endpoint_id":"{}","direct_addrs":["{}"]}},"base_domain":"wonton.localhost"}}"#,
            endpoint.id(), endpoint.bound_sockets()[0]
        ),
    }).unwrap();
    let before = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    phone.dispatch(Action::RetryConnection).unwrap();
    let recovered = tokio::time::timeout(
        Duration::from_secs(15),
        phone.wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.details.last_connected_at_ms > before.details.last_connected_at_ms
        }),
    )
    .await
    .expect("retry must confirm a fresh connection instead of waiting for the old ping forever");
    assert_eq!(recovered.notice, None);
    assert_eq!(endpoint.metrics().socket.num_conns_opened.get(), 2);
    endpoint.close().await;
    task.abort();
}
