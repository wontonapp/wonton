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

use crate::support::{Phone, Timeouts, WAIT, loopback_endpoint};

/// 一个来一条连接就拒绝一条的服务端，比如正在关闭的核心。数一数手机来了多少次。
pub(crate) struct Refusing {
    pub(crate) addr: EndpointAddr,
    dials: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Refusing {
    pub(crate) async fn start() -> Self {
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

    pub(crate) fn dials(&self) -> usize {
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

    // 负载只会让建连变少、不会变多，“不比退避更快”只有上限是性质本身；下限只证明确实在拨。
    let dials = server.dials();
    assert!(dials <= 4, "3 秒内建连 {dials} 次，快过了退避");
    assert!(dials >= 1, "3 秒内一次建连都没有");
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

    // 先等第 4 次建连：之后失败的退避名义 8 秒，窗口离自然重试足够远。
    let deadline = tokio::time::Instant::now() + WAIT;
    while server.dials() < 4 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "退避期内只建连 {} 次",
            server.dials()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let before = server.dials();

    phone.client.retry_now();
    // 第 4 次失败后的退避最少 8 × 0.8 秒，这 3 秒内的新建连只能来自 retry_now 清零退避。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while server.dials() <= before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "清零退避后没有马上再试"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    asking.abort();
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
