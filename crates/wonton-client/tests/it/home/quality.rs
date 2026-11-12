//! 真实回环 QUIC 流量配合独立的判定时钟；生产窗口、50 包门槛和滞回时长均不缩短。

mod wire;

use std::net::Ipv6Addr;
use std::time::Duration;

use futures_util::{StreamExt, stream};
use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use wonton_client::{Action, ConnectionQuality, HomeStatus, HomeViewModel, Network, ProxySettings};

use super::support::{Home, Phone, WEB_URL, get};
use wire::Wire;

struct Site {
    phone: Phone,
    _home: Home,
    wire: Wire,
    proxy: ProxySettings,
    elapsed: Duration,
    _dir: tempfile::TempDir,
}

impl Site {
    async fn start(delay: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // 客户端只有 IPv4 套接字，服务器的 IPv6 原地址不能被 QNT 拿来绕过代理。
        let phone = Phone::with_network(dir.path(), Network::TestRelay);
        let key = SecretKey::generate();
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .clear_ip_transports()
            .bind_addr((Ipv6Addr::LOCALHOST, 0))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let home = Home::on(&[phone.device_id()], key, endpoint).await;
        let wire = Wire::start(home.server().endpoint().bound_sockets()[0]).await;
        wire.set(0, delay);
        phone
            .dispatch(Action::ConnectHome {
                ticket: home.ticket_at(wire.addr()),
            })
            .unwrap();
        let proxy = phone
            .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
            .await
            .proxy
            .unwrap();
        let view = phone.test_sample_connection_quality(Duration::ZERO).await;
        assert_eq!(view.details.quality, ConnectionQuality::Unknown);
        assert_eq!(view.details.loss_per_mille, None);
        Self {
            phone,
            _home: home,
            wire,
            proxy,
            elapsed: Duration::ZERO,
            _dir: dir,
        }
    }

    async fn sample(&mut self, seconds: u64) -> HomeViewModel {
        self.exchange(64).await;
        self.tick(Duration::from_secs(seconds)).await
    }

    async fn tick(&mut self, elapsed: Duration) -> HomeViewModel {
        self.elapsed += elapsed;
        self.phone
            .test_sample_connection_quality(self.elapsed)
            .await
    }

    async fn exchange(&self, count: usize) {
        // 小请求在慢链路上会合包；每次发送超过一个 MTU，确保 50 包下限来自真实数据。
        let path = format!("/quality/{}", "x".repeat(2_048));
        stream::iter(0..count)
            .map(|_| async {
                let (status, body) = get(&self.proxy, WEB_URL, &path, true).await;
                assert_eq!((status, body.as_str()), (200, path.as_str()));
            })
            .buffer_unordered(32)
            .collect::<Vec<()>>()
            .await;
    }

    async fn good(&mut self) {
        let first = self.sample(1).await;
        assert_eq!(first.details.loss_per_mille, Some(0));
        assert_eq!(first.details.quality, ConnectionQuality::Unknown);
        let before = self.sample(29).await;
        assert_eq!(before.details.quality, ConnectionQuality::Unknown);
        let good = self.sample(1).await;
        assert_eq!(good.details.quality, ConnectionQuality::Good);
        assert!(good.details.rtt_ms.unwrap() < 500);
    }

    async fn poor(&mut self) {
        self.wire.set(200, Duration::ZERO);
        let first = self.sample(1).await;
        assert!(first.details.loss_per_mille.unwrap() >= 50, "{first:?}");
        assert_eq!(first.details.quality, ConnectionQuality::Unknown);
        assert_eq!(
            self.sample(9).await.details.quality,
            ConnectionQuality::Unknown
        );
        let poor = self.sample(1).await;
        assert_eq!(poor.details.quality, ConnectionQuality::Poor);
        assert!(poor.details.loss_per_mille.unwrap() >= 50);
    }
}

#[tokio::test]
async fn a_fresh_connection_is_unknown_while_samples_are_short() {
    let mut site = Site::start(Duration::ZERO).await;
    let view = site.tick(Duration::from_secs(29)).await;
    assert_eq!(view.details.quality, ConnectionQuality::Unknown);
    assert_eq!(view.details.loss_per_mille, None);
}

#[tokio::test]
async fn a_healthy_link_is_good() {
    Site::start(Duration::ZERO).await.good().await;
}

#[tokio::test]
async fn heavy_loss_turns_poor() {
    Site::start(Duration::ZERO).await.poor().await;
}

#[tokio::test]
async fn recovery_needs_the_whole_hold_before_good() {
    let mut site = Site::start(Duration::ZERO).await;
    site.poor().await;
    site.wire.set(0, Duration::ZERO);
    // noq 的丢包计数在学到丢包时才入账（ACK 空洞），不按发包时刻归属：200‰ 期间没学完的账会学进后面
    // 的采样增量，污染紧跟着的干净窗口。这里循环小步采样，等累计丢失数停走（账学完了），再走一秒让它
    // 滑出 30 秒窗口，干净样本的增量里就只剩干净流量。CI 高负载（同进程并跑上千个测试）下记账延迟拖得
    // 很长，不等的话干净窗口实测到过 205‰（run 43115663522）。
    let mut lost = site.phone.home().details.lost_packets;
    for _ in 0..30 {
        let now = site.sample(1).await.details.lost_packets;
        if now == lost {
            break;
        }
        lost = now;
    }
    site.sample(1).await;
    let clean = site.sample(30).await;
    assert!(clean.details.loss_per_mille.unwrap() < 20, "{clean:?}");
    assert_eq!(clean.details.quality, ConnectionQuality::Poor);
    assert_eq!(
        site.sample(29).await.details.quality,
        ConnectionQuality::Poor
    );
    assert_eq!(
        site.sample(1).await.details.quality,
        ConnectionQuality::Good
    );
}

#[tokio::test]
async fn a_short_spike_does_not_flip() {
    let mut site = Site::start(Duration::ZERO).await;
    site.good().await;
    site.wire.set(200, Duration::ZERO);
    site.exchange(256).await;
    let spike = site.tick(Duration::from_secs(3)).await;
    assert!(spike.details.loss_per_mille.unwrap() >= 50, "{spike:?}");
    assert_eq!(spike.details.quality, ConnectionQuality::Good);
    site.wire.set(0, Duration::ZERO);
    assert_eq!(
        site.sample(1).await.details.quality,
        ConnectionQuality::Good
    );
    let later = site.sample(10).await;
    assert!(
        later.details.loss_per_mille.unwrap() >= 50,
        "the old spike must still be in the window"
    );
    assert_eq!(later.details.quality, ConnectionQuality::Good);
    assert_eq!(
        site.sample(20).await.details.quality,
        ConnectionQuality::Good
    );
}

#[tokio::test]
async fn a_slow_link_turns_poor() {
    let mut site = Site::start(Duration::from_millis(450)).await;
    let slow = site.sample(1).await;
    assert!(slow.details.rtt_ms.unwrap() >= 800, "{slow:?}");
    assert_eq!(slow.details.quality, ConnectionQuality::Unknown);
    assert_eq!(
        site.sample(9).await.details.quality,
        ConnectionQuality::Unknown
    );
    assert_eq!(
        site.sample(1).await.details.quality,
        ConnectionQuality::Poor
    );
}
