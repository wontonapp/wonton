//! 请求先于壳的前台事件恢复：只推进检测用的钟，QUIC 仍走真实回环网络。

use std::{
    net::{Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use iroh::{EndpointAddr, SecretKey};
use tokio::task::JoinSet;
use wonton_tunnel::{Allowlist, LocalProxy, ProxyAccess, SuspendClock, TunnelClient, TunnelStatus};

use crate::{
    stall::{DROP_ALL, Flaky, PASS},
    support::{Home, Phone, Timeouts, Upstream, WAIT, endpoint_on},
};

#[derive(Debug)]
struct Clock(Mutex<(Instant, SystemTime)>);

impl Clock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new((Instant::now(), SystemTime::now()))))
    }

    fn advance(&self, monotonic: Duration, wall: Duration) {
        let mut now = self.0.lock().unwrap();
        now.0 += monotonic;
        now.1 += wall;
    }
}

impl SuspendClock for Clock {
    fn now(&self) -> (Instant, SystemTime) {
        *self.0.lock().unwrap()
    }
}

async fn rig() -> (Upstream, Home, Flaky, Phone, Arc<Clock>) {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let port = home.server().endpoint().bound_sockets()[0].port();
    let flaky = Flaky::start((Ipv4Addr::LOCALHOST, port).into()).await;
    let server = EndpointAddr::new(home.addr().id).with_ip_addr(flaky.addr);
    let endpoint = endpoint_on((Ipv6Addr::LOCALHOST, 0).into(), device, Timeouts::Default).await;
    let clock = Clock::new();
    let client = TunnelClient::with_suspend_clock(endpoint, server, clock.clone());
    let proxy = LocalProxy::bind(
        client.clone(),
        (Ipv4Addr::LOCALHOST, 0).into(),
        ProxyAccess::Open,
    )
    .await
    .unwrap();
    (upstream, home, flaky, Phone { client, proxy }, clock)
}

fn connections(home: &Home) -> u64 {
    home.server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get()
}

async fn restore_when_redialling(flaky: &Flaky, client: &TunnelClient) {
    let mut status = client.watch_status();
    tokio::time::timeout(WAIT, status.wait_for(|s| *s == TunnelStatus::Connecting))
        .await
        .expect("恢复检查没有发起重拨")
        .unwrap();
    flaky.drop_above(PASS);
}

#[tokio::test]
async fn the_first_proxy_request_after_suspension_does_not_need_a_foreground_event() {
    // 分开验证普通挂起（单调钟在走）和设备休眠（只有墙上钟在走）。
    for (monotonic, wall) in [
        (Duration::from_secs(10), Duration::ZERO),
        (Duration::ZERO, Duration::from_secs(10)),
    ] {
        let (upstream, mut home, flaky, phone, clock) = rig().await;
        assert_eq!(phone.get("/before").await.status, 200);
        phone.client.set_suspendable(true);
        flaky.drop_above(DROP_ALL);
        home.crash();
        home.restart().await;
        clock.advance(monotonic, wall);
        flaky.drop_above(PASS);
        // 不等计时任务，也不发前台事件；服务端已经没有旧连接，首请求必须在开流前发现恢复。
        let response = phone.get("/first-after-resume").await;
        assert_eq!(response.status, 200, "{response:?}");
        assert_eq!(response.body, "/first-after-resume");
        assert_eq!(connections(&home), 1, "重启后的服务端只应接受一次新连接");
        assert_eq!(upstream.hits(), 2, "请求不得重放");
        phone.client.close();
    }
}

#[tokio::test]
async fn requests_timer_and_foreground_share_one_replacement_and_its_recent_result() {
    let (upstream, home, flaky, phone, clock) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    phone.client.set_suspendable(true);
    flaky.drop_above(DROP_ALL);
    clock.advance(Duration::from_secs(10), Duration::from_secs(10));
    let first = phone.get("/first");
    let foreground = async {
        phone.client.retry_now();
        phone.client.revalidate().await.unwrap();
        phone.client.set_suspendable(false);
    };
    let batch = async {
        let mut requests = JoinSet::new();
        for n in 0..12 {
            let proxy = phone.proxy.local_addr();
            requests.spawn(async move {
                let path = format!("/waiting/{n}");
                let response = crate::support::http_get(proxy, &path).await;
                assert_eq!(response.status, 200, "{response:?}");
                assert_eq!(response.body, path);
            });
        }
        while let Some(result) = requests.join_next().await {
            result.unwrap();
        }
    };
    let (first, (), (), ()) = tokio::join!(
        first,
        foreground,
        batch,
        restore_when_redialling(&flaky, &phone.client),
    );
    assert_eq!(first.status, 200, "{first:?}");
    assert_eq!(first.body, "/first");
    assert_eq!(connections(&home), 2);
    assert_eq!(upstream.hits(), 14);

    // 完成后的迟到前台事件必须直接用结果。若又检查，全丢包会迫使它第三次建连。
    flaky.drop_above(DROP_ALL);
    phone.client.revalidate().await.unwrap();
    assert_eq!(connections(&home), 2);
    flaky.drop_above(PASS);
    assert_eq!(phone.get("/after").await.body, "/after");
    // 一秒之外不能继续沿用旧结论；下一次明确的前台检查要真的检测失效路径。
    clock.advance(Duration::from_secs(2), Duration::from_secs(2));
    flaky.drop_above(DROP_ALL);
    let (checked, ()) = tokio::join!(
        phone.client.revalidate(),
        restore_when_redialling(&flaky, &phone.client),
    );
    checked.unwrap();
    assert_eq!(connections(&home), 3);
    assert_eq!(
        phone.get("/after-expired-check").await.body,
        "/after-expired-check"
    );
}

/// 只推进 Tokio 的计时调度；检测的钟由 Clock 单独推进，不把 QUIC 挂起十秒。
async fn tick() {
    tokio::task::yield_now().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
}

#[tokio::test]
async fn the_timer_can_revalidate_before_any_request_or_foreground_event() {
    let (_upstream, mut home, flaky, phone, clock) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    phone.client.set_suspendable(true);
    flaky.drop_above(DROP_ALL);
    home.crash();
    home.restart().await;
    clock.advance(Duration::ZERO, Duration::from_secs(10));

    tick().await;
    restore_when_redialling(&flaky, &phone.client).await;
    phone.wait_for(|s| *s == TunnelStatus::Connected).await;
    assert_eq!(phone.get("/after-timer").await.body, "/after-timer");
    assert_eq!(connections(&home), 1);
}

#[tokio::test]
async fn an_executing_background_timer_does_not_probe_or_replace_the_connection() {
    let (_upstream, home, flaky, phone, clock) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    phone.client.set_suspendable(true);
    let confirmed = phone.client.stats().last_confirmed;
    for _ in 0..6 {
        clock.advance(Duration::from_secs(1), Duration::from_secs(1));
        tick().await;
    }
    assert_eq!(
        phone.client.stats().last_confirmed,
        confirmed,
        "后台未挂起时不应主动检查"
    );

    // 请求在旧路径上等待时不应触发检查重拨；旧路径一恢复，同一请求直接成功。
    flaky.drop_above(DROP_ALL);
    let get = phone.get("/background-music");
    let restore = async {
        tick().await;
        flaky.drop_above(PASS);
    };
    let (response, ()) = tokio::join!(get, restore);
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.body, "/background-music");
    assert_eq!(connections(&home), 1);
}

#[tokio::test]
async fn disabling_suspend_detection_leaves_new_requests_on_the_live_connection() {
    let (_upstream, home, flaky, phone, clock) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    phone.client.set_suspendable(true);
    phone.client.set_suspendable(false);
    clock.advance(Duration::from_secs(10), Duration::from_secs(10));
    tick().await;
    flaky.drop_above(DROP_ALL);
    let get = phone.get("/foreground");
    let restore = async {
        tick().await;
        flaky.drop_above(PASS);
    };
    let (response, ()) = tokio::join!(get, restore);
    assert_eq!(response.status, 200, "{response:?}");
    assert_eq!(response.body, "/foreground");
    assert_eq!(connections(&home), 1);
}

#[tokio::test]
async fn late_resume_signals_share_a_failed_check_until_the_reuse_window_expires() {
    let (_upstream, _home, flaky, phone, clock) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    let refusing = crate::pacing::Refusing::start().await;
    // 手机只绑 IPv6；拒绝端只绑 IPv4，和主测试一样经双栈转发器才能到达。
    let refusal_path = Flaky::start(*refusing.addr.ip_addrs().next().unwrap()).await;
    phone
        .client
        .set_server_addr(EndpointAddr::new(refusing.addr.id).with_ip_addr(refusal_path.addr));
    flaky.drop_above(DROP_ALL);

    let failure = phone.client.revalidate().await.unwrap_err();
    assert_eq!(refusing.dials(), 1);
    assert_eq!(phone.client.revalidate().await.unwrap_err(), failure);
    assert_eq!(phone.get("/late-request").await.status, 502);
    assert_eq!(refusing.dials(), 1, "迟到的恢复信号不应重复重拨");

    clock.advance(Duration::from_secs(2), Duration::from_secs(2));
    phone.client.revalidate().await.unwrap_err();
    assert_eq!(refusing.dials(), 2, "过期后必须重新尝试，不能永久缓存失败");
}
