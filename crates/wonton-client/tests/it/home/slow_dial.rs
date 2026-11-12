//! 慢设备上第一次建连（翻译层跑 arm64 包、冷缓存）：一次握手要十几二十秒，正常的 12 秒建连时限每次都把它掐在
//! 快成功的时候，客户端一轮一轮重来、服务端一次次记 aborted by peer。冷启动的头两次建连放宽，用完之后还是按
//! 正常的时限放弃。绑定不跟着放宽：绑定时限一过基本就是 fork 卡死，等更久也回不来（绑定的钩子测试见下）。
//!
//! 慢用回环上一个两个方向都晚一拍的 UDP 转发造出来；时限用测试钩子缩短，不等生产的那几十秒。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use wonton_client::{Action, HomeStatus};

use super::support::{Home, Phone, SlowUdp};

/// 每包晚这么久：一次握手要几个来回，总共一两秒。
const LAG: Duration = Duration::from_millis(300);

/// 等连上，顺手记下中途报出过的最多失败次数。
async fn connect(phone: &Phone, home: &Home, slow: &SlowUdp) -> u32 {
    phone
        .dispatch(Action::ConnectHome {
            ticket: home.ticket_at(slow.addr()),
        })
        .unwrap();
    let most = Arc::new(AtomicU32::new(0));
    let seen = most.clone();
    phone
        .wait_for(|view| {
            seen.fetch_max(view.details.failed_dials, Ordering::SeqCst);
            view.status == HomeStatus::Connected
        })
        .await;
    most.load(Ordering::SeqCst)
}

#[tokio::test]
async fn a_slow_first_handshake_is_not_cut_by_the_normal_dial_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let slow = SlowUdp::start(home.socket(), LAG).await;
    // 正常时限比一次握手短：第一次建连不放宽就会被掐掉重来。
    phone.test_set_cold_allowance(Duration::from_secs(4), Duration::from_millis(50));

    assert_eq!(connect(&phone, &home, &slow).await, 0);
    assert_eq!(
        home.server()
            .endpoint()
            .metrics()
            .socket
            .num_conns_opened
            .get(),
        1,
        "第一次握手被掐掉重来过"
    );
}

#[tokio::test]
async fn only_the_first_dials_are_given_the_extra_time() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let slow = SlowUdp::start(home.socket(), LAG).await;
    // 两次冷启动的时限都远不够一次握手，第三次用正常时限（够）。10 毫秒比一次握手短得多，第二次不会因为
    // 前一次把 socket 和密钥热起来就赶得上。
    phone.test_set_cold_allowance(Duration::from_millis(10), Duration::from_secs(4));

    assert_eq!(connect(&phone, &home, &slow).await, 2);
}

#[tokio::test]
async fn a_bind_that_misses_the_deadline_is_retried_without_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let slow = SlowUdp::start(home.socket(), LAG).await;
    // 头一次绑定比时限慢，之后正常：那一次如实算失败，马上换一次新的绑定，不停在连接中。
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let sleepy = first.clone();
    phone.test_set_bind_hook(Some(Arc::new(move || {
        if sleepy.swap(false, Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(300));
        }
    })));
    phone.test_set_establish_timing(Duration::from_millis(100), Duration::from_millis(100));
    phone.test_set_cold_allowance(Duration::from_secs(4), Duration::from_secs(4));

    assert_eq!(connect(&phone, &home, &slow).await, 1);
}
