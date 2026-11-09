//! 隧道流转发到本机 TCP 上游：授权、连接复用、错误归类、吊销和服务端重启。

use std::net::{Ipv4Addr, SocketAddr};

use iroh::{EndpointAddr, SecretKey};
use wonton_tunnel::{Allowlist, ERROR_HEADER, TunnelError, TunnelStatus};

use crate::support::{Home, Phone, Timeouts, Upstream, http_get};

#[tokio::test]
async fn paired_device_reaches_upstream_over_one_reused_connection() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    for i in 0..5 {
        let response = phone.get(&format!("/sequential/{i}")).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.body, format!("/sequential/{i}"));
    }
    let concurrent: Vec<_> = (0..20)
        .map(|i| {
            let proxy = phone.proxy.local_addr();
            tokio::spawn(async move { http_get(proxy, &format!("/concurrent/{i}")).await })
        })
        .collect();
    for (i, task) in concurrent.into_iter().enumerate() {
        let response = task.await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, format!("/concurrent/{i}"));
    }

    assert_eq!(upstream.hits(), 25);
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    // 25 个 HTTP 请求只建了一条 QUIC 连接。
    let opened = home
        .server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get();
    assert_eq!(opened, 1);
}

#[tokio::test]
async fn unpaired_device_is_rejected_before_reaching_upstream() {
    let upstream = Upstream::spawn().await;
    let paired = SecretKey::generate().public();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([paired]),
        upstream.addr,
    )
    .await;
    let stranger = Phone::start(SecretKey::generate(), home.addr(), Timeouts::Default).await;

    assert_eq!(stranger.client.ping().await, Err(TunnelError::Unauthorized));
    let response = stranger.get("/secret").await;
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(upstream.hits(), 0);
    assert_eq!(
        stranger.client.status(),
        TunnelStatus::Disconnected(TunnelError::Unauthorized)
    );
}

#[tokio::test]
async fn unreachable_server_is_reported_as_transport_unavailable() {
    // 一个从没上线过的服务端：有 endpoint ID 和地址，但地址上没有任何东西在监听。
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let silent = EndpointAddr::new(SecretKey::generate().public())
        .with_ip_addr(socket.local_addr().unwrap());
    drop(socket);
    let phone = Phone::start(SecretKey::generate(), silent, Timeouts::Short).await;

    let err = phone.client.ping().await.unwrap_err();
    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    let response = phone.get("/").await;
    assert_eq!(response.status, 502);
    assert_eq!(response.header(ERROR_HEADER), Some("transport-unavailable"));
}

#[tokio::test]
async fn upstream_failure_is_distinguished_from_tunnel_failure() {
    // 连不上的上游：0 号端口上不会有人监听，连接立刻失败（Linux 上被拒绝，macOS 上报地址不可用）。先绑后关的随机端口
    // 在机器忙的时候可能被别的程序占去，请求就打到了活的服务上；绑着不监听的端口在 macOS 上连接会一直等到超时。
    let dead_upstream = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        dead_upstream,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    let response = phone.get("/").await;
    assert_eq!(response.status, 503);
    assert_eq!(response.header(ERROR_HEADER), Some("upstream-unavailable"));
    // 隧道本身是好的：服务端接受了这台设备。
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
}

#[tokio::test]
async fn revoking_a_device_closes_its_live_connection() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device.clone(), home.addr(), Timeouts::Default).await;
    assert_eq!(phone.get("/before").await.status, 200);

    assert!(home.server().allowlist().revoke(&device.public()));

    // 不发任何请求，客户端也会因为连接被关闭而看到“未授权”。
    phone
        .wait_for(|status| *status == TunnelStatus::Disconnected(TunnelError::Unauthorized))
        .await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test]
async fn client_recovers_after_server_restart() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let server_key = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(server_key, allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Short).await;
    assert_eq!(phone.get("/before").await.status, 200);

    home.stop().await;
    // 服务端正常关闭时会通知客户端，客户端不用等超时就知道连接断了。
    phone
        .wait_for(|status| matches!(status, TunnelStatus::Disconnected(_)))
        .await;
    let response = phone.get("/while-down").await;
    assert_eq!(response.status, 502);
    assert_eq!(response.header(ERROR_HEADER), Some("transport-unavailable"));

    home.restart().await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/after");
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
}

#[tokio::test]
async fn client_recovers_after_server_crash() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let server_key = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(server_key, allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Short).await;
    assert_eq!(phone.get("/before").await.status, 200);

    // 不经关闭直接丢弃服务端，对端收不到任何通知，相当于断电或断网。
    home.crash();
    phone
        .wait_for(|status| matches!(status, TunnelStatus::Disconnected(_)))
        .await;

    home.restart().await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/after");
}
