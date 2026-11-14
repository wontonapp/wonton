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
    let message = down.details.last_failure.as_ref().expect("应有失败说明");
    assert!(message.contains("0.3"), "{message}");
    let diagnostic = down.details.diagnostic.as_ref().expect("应能查看原始诊断");
    assert!(diagnostic.contains("300ms"), "{diagnostic}");
    assert!(!message.contains("endpoint binding"), "{message}");
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

/// Pause after dispatch has captured A, switch to B, then resume the language action.
#[test]
fn language_refresh_cannot_publish_from_a_replaced_scope() {
    const CHILD: &str = "WONTON_LANGUAGE_SWITCH_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "home::stuck_bind::language_refresh_cannot_publish_from_a_replaced_scope",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated language test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(language_switch_interleaving());
}

async fn language_switch_interleaving() {
    use std::sync::{Arc, Mutex};
    use wonton_protocol::{HomeAction, SettingsAction};

    struct OpenOnDrop(Gate);
    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            self.0.open();
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let phone = Arc::new(Phone::open(dir.path()));
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    let bind = OpenOnDrop(Gate::default());
    phone.test_set_bind_hook(Some(bind.0.hook()));
    phone.test_set_establish_timing(BIND, NEVER);
    phone.connect(&a);
    let failed = phone.wait_for(unreachable).await;
    assert!(failed.details.diagnostic.is_some());
    assert!(failed.proxy.is_none());
    // Wait until follow_pairing has recorded A, so AddServer is a real switch.
    tokio::time::timeout(Duration::from_secs(30), async {
        let servers = phone.subscribe_servers();
        while let Some(view) = servers.next().await {
            if view.servers.iter().any(|row| row.current) {
                return;
            }
        }
        panic!("server subscription ended");
    })
    .await
    .expect("A was not saved");
    bind.0.open();

    let paused = OpenOnDrop(Gate::default());
    let (arrived, arrival) = tokio::sync::oneshot::channel();
    let arrived = Mutex::new(Some(arrived));
    let wait = paused.0.hook();
    phone.test_set_language_refresh_hook(Some(Arc::new(move || {
        if let Some(arrived) = arrived.lock().unwrap().take() {
            let _ = arrived.send(());
        }
        wait();
    })));
    let pending_phone = phone.clone();
    let pending = tokio::task::spawn_blocking(move || {
        pending_phone.dispatch(Action::Settings {
            action: SettingsAction::SetLanguages {
                languages: vec!["ja".into()],
            },
        })
    });
    tokio::time::timeout(Duration::from_secs(30), arrival)
        .await
        .unwrap()
        .unwrap();
    phone
        .dispatch(Action::Home {
            action: HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    let server_id = b.server().endpoint().id().to_string();
    let connected = phone
        .wait_for(|view| {
            view.server_id.as_deref() == Some(server_id.as_str())
                && view.status == HomeStatus::Connected
                && view.proxy.is_some()
        })
        .await;
    let proxy = connected.proxy.clone().unwrap();
    let subscription = phone.subscribe_home();
    phone.test_set_language_refresh_hook(None);
    paused.0.open();
    tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let subscribed = subscription.next().await.expect("home subscription ended");
    assert_eq!(subscribed.server_id, connected.server_id);
    assert_eq!(subscribed.status, HomeStatus::Connected);
    assert_eq!(subscribed.proxy.as_ref(), Some(&proxy));
    assert_eq!(subscribed.details.diagnostic, connected.details.diagnostic);
    let after = phone.home();
    assert_eq!(after.server_id, connected.server_id);
    assert_eq!(after.status, HomeStatus::Connected);
    assert_eq!(after.proxy.as_ref(), Some(&proxy));
    assert_eq!(after.details.diagnostic, connected.details.diagnostic);
    let (status, body) = get(&proxy, WEB_URL, "/after-language-switch", true).await;
    assert_eq!((status, body.as_str()), (200, "/after-language-switch"));
}
