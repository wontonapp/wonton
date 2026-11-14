//! App 锁的行为测试（ADR-0102）：只经壳会用的接口驱动——`set_app_lock`、前后台 `Action`、订阅、`unlock_app`，
//! 断言 ViewModel。宽限只有整分档，时间用 `expire_app_lock_for_test` 往前拨，不真的等。
//!
//! 冷启动的锁定和恢复密钥的统一验证在壳里，核心这边只验证判定。

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use wonton_client::{Action, AppLockTimeout, ClientCore, Network};

fn open(dir: &TempDir) -> Arc<ClientCore> {
    ClientCore::with_network(dir.path().to_str().unwrap(), Network::DirectOnly).unwrap()
}

fn lock(core: &ClientCore, timeout: AppLockTimeout) {
    core.set_app_lock(true, timeout).unwrap();
}

#[tokio::test]
async fn a_cold_start_with_the_lock_on_comes_up_locked() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::FiveMinutes);
    drop(core);

    let reopened = open(&dir);
    let view = reopened.app_lock();
    assert!(view.enabled);
    assert_eq!(view.timeout, AppLockTimeout::FiveMinutes);
    assert!(view.locked, "每次冷启动都要重新验证");
}

#[tokio::test]
async fn the_lock_is_off_until_it_is_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    assert!(!core.app_lock().enabled);
    // 关着的时候前后台切换不锁。
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    core.expire_app_lock_for_test(Duration::from_secs(3600));
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppForegrounded,
    })
    .unwrap();
    assert!(!core.app_lock().locked);
}

#[tokio::test]
async fn turning_it_on_does_not_cover_the_settings_page() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::Immediate);
    assert!(!core.app_lock().locked);
}

#[tokio::test]
async fn the_immediate_timeout_locks_as_soon_as_the_app_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::Immediate);

    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    assert!(core.app_lock().locked);
}

#[tokio::test]
async fn the_chosen_grace_is_waited_out_before_locking() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::OneMinute);
    let updates = core.subscribe_app_lock();
    updates.next().await.unwrap();

    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    core.expire_app_lock_for_test(Duration::from_secs(30));
    assert!(!core.app_lock().locked, "还没到一分钟");

    core.expire_app_lock_for_test(Duration::from_secs(31));
    let locked = updates.next().await.unwrap();
    assert!(locked.locked, "到点要主动推给壳，通知才知道收着写");
}

#[tokio::test]
async fn coming_back_within_the_grace_does_not_ask_again() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::FiveMinutes);

    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    core.expire_app_lock_for_test(Duration::from_secs(120));
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppForegrounded,
    })
    .unwrap();
    assert!(!core.app_lock().locked);
}

#[tokio::test]
async fn coming_back_after_the_grace_lands_on_the_lock_page() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::OneMinute);

    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    core.expire_app_lock_for_test(Duration::from_secs(61));
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppForegrounded,
    })
    .unwrap();
    assert!(core.app_lock().locked);
}

#[tokio::test]
async fn unlocking_needs_a_verification_only_when_locked() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::Immediate);
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    assert!(core.app_lock().locked);

    core.unlock_app();
    assert!(!core.app_lock().locked);
    // 幂等。
    core.unlock_app();
    assert!(!core.app_lock().locked);
}

#[tokio::test]
async fn turning_the_lock_off_releases_the_lock_page() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::Immediate);
    core.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::AppBackgrounded,
    })
    .unwrap();
    assert!(core.app_lock().locked);

    core.set_app_lock(false, AppLockTimeout::Immediate).unwrap();
    assert!(!core.app_lock().locked);
}

#[tokio::test]
async fn the_setting_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir);
    lock(&core, AppLockTimeout::FifteenMinutes);
    drop(core);

    let reopened = open(&dir);
    let view = reopened.app_lock();
    assert!(view.enabled);
    assert_eq!(view.timeout, AppLockTimeout::FifteenMinutes);
}
