//! iroh 的绑定卡在同步阻塞里不返回时（Android 上慢设备的网卡枚举），配对不能一直停在“正在配对”：交换的时限
//! 照常触发；运行时整个被拖住时，看门线程在它自己的时限上报失败，而且不盖掉后来又起的一次配对。

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use wonton_client::{HomeStatus, PairingFailure, PairingStatus};

use super::{Home, Phone};

/// 测试里的时限，取短的免得等满生产的 30 秒；用不上的那一道取得比整个测试还长。
const SHORT: Duration = Duration::from_millis(300);
const NEVER: Duration = Duration::from_secs(600);
/// 只给调度留的余量：没有修好时状态永远停在“正在配对”，等多久都一样。
const SLACK: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_bind_that_never_returns_fails_at_the_exchange_deadline() {
    let home_side = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let gate = Gate::default();
    phone.test_set_bind_hook(Some(gate.hook()));
    // 看门线程等得比测试还久：失败只能来自交换自己的时限，证明绑定卡住时计时器照常在走。
    phone.test_set_pair_deadlines(SHORT, NEVER);
    phone.pair(&home_side.code(None)).unwrap();
    assert_eq!(phone.home().pairing, Some(PairingStatus::InProgress));
    let (failure, message) = tokio::time::timeout(SLACK, phone.pairing_failure())
        .await
        .unwrap_or_else(|_| panic!("绑定卡住时交换的时限没触发：{:?}", phone.home()));
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    gate.open();
}

#[tokio::test]
async fn the_watchdog_fails_a_stuck_pairing_and_leaves_a_newer_one_alone() {
    let home_side = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let gate = Gate::default();
    phone.test_set_bind_hook(Some(gate.hook()));
    // 交换的时限够不着：失败只能来自看门线程。
    phone.test_set_pair_deadlines(NEVER, SHORT);
    phone.pair(&home_side.code(None)).unwrap();
    let (failure, message) = tokio::time::timeout(SLACK, phone.pairing_failure())
        .await
        .unwrap_or_else(|_| panic!("看门线程没报失败：{:?}", phone.home()));
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");

    // 旧的一次还卡着、看门线程还在等时，又起了一次：旧看门线程到点不能把新的一次报成失败。
    let old_watchdog = Duration::from_secs(1);
    phone.test_set_pair_deadlines(NEVER, old_watchdog);
    phone.pair(&home_side.code(None)).unwrap();
    phone.test_set_pair_deadlines(NEVER, NEVER);
    phone.pair(&home_side.code(None)).unwrap();
    tokio::time::sleep(old_watchdog + SHORT).await;
    assert_eq!(phone.home().pairing, Some(PairingStatus::InProgress));

    // 放行以后旧的几次也做完了交换，它们迟到的结果不算数，新的一次照常配上、连通。
    gate.open();
    let view = phone
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;
    assert_eq!(
        view.server_id,
        Some(home_side.server.endpoint().id().to_string())
    );
}

/// 挡住绑定的闸：打开之前进来的阻塞线程都停在这里。
#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn hook(&self) -> Arc<dyn Fn() + Send + Sync> {
        let gate = self.clone();
        Arc::new(move || {
            let (open, wake) = &*gate.0;
            let mut open = open.lock().unwrap();
            while !*open {
                open = wake.wait(open).unwrap();
            }
        })
    }

    fn open(&self) {
        let (open, wake) = &*self.0;
        *open.lock().unwrap() = true;
        wake.notify_all();
    }
}
