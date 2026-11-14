//! 建连时 iroh 的绑定卡住不返回（x86 模拟器上跑 arm64 包时 netwatch fork 出的 `ip` 子进程会卡死）：
//! 不能一直停在“连接中”、断开 0 次、没有重试入口。绑定到点放弃，如实报连不上；之后自己按退避再试，
//! 主人点“重试”时马上再试，绑定一好就连上，不用关掉 App 重开。

use std::time::Duration;

use wonton_client::{Action, ConnectionNotice, HomeFailure, HomeStatus, HomeViewModel};

use super::support::{Gate, Home, Phone, WEB_URL, get};

/// 测试里的绑定时限：真的绑定在回环上几毫秒就好，卡住的那次等多久都一样。
const BIND: Duration = Duration::from_millis(300);
/// 比整个测试还长的退避：连上只能是“重试”促成的。
const NEVER: Duration = Duration::from_secs(600);

fn unreachable(view: &HomeViewModel) -> bool {
    matches!(
        view.status,
        HomeStatus::Disconnected {
            reason: HomeFailure::TransportUnavailable,
            ..
        }
    )
}

#[tokio::test]
async fn a_stuck_bind_is_reported_and_retried_until_the_connection_comes_up() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let gate = Gate::default();
    phone.test_set_bind_hook(Some(gate.hook()));
    phone.test_set_establish_timing(BIND, Duration::from_millis(500));
    phone.connect(&home);

    // 绑定一直卡着：几轮之后如实说连不上、给重试入口，失败次数在涨，不停在“连接中”。
    let down = phone
        .wait_for(|view| {
            unreachable(view)
                && matches!(
                    view.notice,
                    Some(ConnectionNotice::Unreachable { retrying: false })
                )
        })
        .await;
    assert!(down.details.failed_dials >= 2, "{:?}", down.details);
    assert!(down.details.last_failure.is_some(), "{:?}", down.details);
    assert_eq!(down.proxy, None);

    // 绑定好了：不做任何操作，下一轮自己连上，代理能用。
    gate.open();
    let up = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    assert_eq!(up.notice, None);
    let (status, body) = get(&up.proxy.unwrap(), WEB_URL, "/hello", true).await;
    assert_eq!((status, body.as_str()), (200, "/hello"));
}

#[tokio::test]
async fn retry_sets_up_the_connection_at_once_instead_of_waiting_out_the_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let gate = Gate::default();
    phone.test_set_bind_hook(Some(gate.hook()));
    phone.test_set_establish_timing(BIND, NEVER);
    phone.connect(&home);
    phone.wait_for(unreachable).await;

    gate.open();
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RetryConnection,
        })
        .unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
}
