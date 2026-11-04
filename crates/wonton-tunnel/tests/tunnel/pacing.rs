//! 连不上时的节奏：失败后等待加倍，每次建连有时限，主人或网络变化可以跳过等待（ADR-0032）。

use std::{
    net::{Ipv4Addr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::{EndpointAddr, SecretKey};
use tokio::task::JoinHandle;
use wonton_tunnel::{ALPN, TunnelError};

use crate::support::{Phone, Timeouts, loopback_endpoint};

/// 一个来一条连接就拒绝一条的服务端，比如正在关闭的核心。数一数手机来了多少次。
struct Refusing {
    addr: EndpointAddr,
    dials: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Refusing {
    async fn start() -> Self {
        let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
        endpoint.set_alpns(vec![ALPN.to_vec()]);
        let addr = endpoint.addr();
        let dials = Arc::new(AtomicUsize::new(0));
        let counted = dials.clone();
        let task = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                incoming.refuse();
            }
        });
        Self { addr, dials, task }
    }

    fn dials(&self) -> usize {
        self.dials.load(Ordering::SeqCst)
    }
}

impl Drop for Refusing {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 不停发请求，相当于 App 里各处都在等连接。
fn keep_asking(phone: &Phone) -> JoinHandle<()> {
    let client = phone.client.clone();
    tokio::spawn(async move {
        loop {
            client.ping().await.ok();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
}

/// 改之前每个请求都可能在上一次失败后立刻建连，核心关闭时 1 秒内连了 5 次。
#[tokio::test]
async fn requests_do_not_dial_faster_than_the_backoff() {
    let server = Refusing::start().await;
    let phone = Phone::start(
        SecretKey::generate(),
        server.addr.clone(),
        Timeouts::Default,
    )
    .await;
    let asking = keep_asking(&phone);

    // 名义上第 0、1、3 秒各一次，最多浮动 20%。
    tokio::time::sleep(Duration::from_secs(3)).await;
    asking.abort();

    assert!(
        (2..=4).contains(&server.dials()),
        "3 秒内建连 {} 次",
        server.dials()
    );
}

#[tokio::test]
async fn retrying_skips_the_wait() {
    let server = Refusing::start().await;
    let phone = Phone::start(
        SecretKey::generate(),
        server.addr.clone(),
        Timeouts::Default,
    )
    .await;
    let asking = keep_asking(&phone);
    tokio::time::sleep(Duration::from_secs(4)).await;
    let before = server.dials();

    // 下一次要等 4 秒左右。
    phone.client.retry_now();
    tokio::time::sleep(Duration::from_millis(500)).await;
    asking.abort();

    assert!(server.dials() > before, "清零退避后没有马上再试");
}

#[tokio::test]
async fn a_dial_gives_up_at_its_deadline() {
    // 地址上有人占着，但从不应答：QUIC 自己要 30 秒才放弃。
    let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let server = EndpointAddr::new(SecretKey::generate().public())
        .with_ip_addr(silent.local_addr().unwrap());
    let phone = Phone::start(SecretKey::generate(), server, Timeouts::Default).await;

    let started = Instant::now();
    let err = phone.client.ping().await.unwrap_err();

    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(10) && took < Duration::from_secs(16),
        "{took:?}"
    );
}
