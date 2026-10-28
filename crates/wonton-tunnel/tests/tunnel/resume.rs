//! App 从挂起中恢复时的连接检查：服务端已经悄悄放弃的连接要尽快换掉，还活着的连接照常复用。

use std::time::{Duration, Instant};

use iroh::SecretKey;
use wonton_tunnel::{Allowlist, TunnelError, TunnelStatus};

use crate::support::{Home, Phone, Timeouts, Upstream};

#[tokio::test]
async fn revalidating_replaces_a_connection_the_server_silently_dropped() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(SecretKey::generate(), allowlist, upstream.addr).await;
    // iroh 默认 30 秒空闲超时：不主动检查，客户端要过 30 秒才发现连接没了。
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;
    assert_eq!(phone.get("/before").await.status, 200);

    // 手机挂起期间服务端放弃了连接，关闭帧到不了手机：服务端不发关闭帧就丢掉连接，手机一无所知。
    home.crash();
    home.restart().await;

    let started = Instant::now();
    let client = phone.client.clone();
    let revalidating = tokio::spawn(async move { client.revalidate().await });
    tokio::task::yield_now().await;
    // 检查期间发出的请求等检查结果，经新连接完成，不会挂到空闲超时。
    let response = phone.get("/during").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/during");
    revalidating.await.unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "换连接用了 {:?}",
        started.elapsed()
    );
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(phone.get("/after").await.status, 200);
}

#[tokio::test]
async fn requests_already_on_the_dropped_connection_fail_fast() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(SecretKey::generate(), allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;
    assert_eq!(phone.get("/before").await.status, 200);
    home.crash();
    home.restart().await;

    // 回到前台的瞬间、检查开始之前发出的请求，落在服务端已经放弃的连接上，等不到回应。
    let started = Instant::now();
    let client = phone.client.clone();
    let stuck = tokio::spawn(async move { client.ping().await });
    tokio::task::yield_now().await;
    phone.client.revalidate().await.unwrap();

    // 换掉连接时它立刻失败，归为传输问题（本地代理回 502，可以重试），不挂到空闲超时。
    let err = stuck.await.unwrap().unwrap_err();
    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn revalidating_keeps_a_live_connection() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let home = Home::start(SecretKey::generate(), allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;
    assert_eq!(phone.get("/before").await.status, 200);

    phone.client.revalidate().await.unwrap();

    assert_eq!(phone.get("/after").await.status, 200);
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    let opened = home
        .server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get();
    assert_eq!(opened, 1);
}
