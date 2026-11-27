//! 切换服务器：切到连不上的那台时第一次失败就说连不上，自动重试不占着“重试”按钮。

use std::time::Duration;

use wonton_client::{Action, ConnectionNotice, HomeStatus};

use super::notices::first_unreachable;
use super::support::{Home, Phone};

/// 切到连不上的服务器：第一次建连失败就说连不上，不等 30 秒；自动重试不占着“重试”按钮。
#[tokio::test]
async fn switching_to_an_unreachable_server_says_so_after_the_first_failure() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let mut b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(a.server().endpoint().id().to_string().as_str())
        })
        .await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::NotConfigured)
        .await;
    phone.connect(&b);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(b.server().endpoint().id().to_string().as_str())
        })
        .await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::SwitchServer { id: "1".into() },
        })
        .unwrap();
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref()
                    == Some(a.server().endpoint().id().to_string().as_str())
        })
        .await;

    b.stop().await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::SwitchServer { id: "2".into() },
        })
        .unwrap();
    // 旧行为要等满 30 秒才换说法；第一次失败就试（对死端口要等到 12 秒的建连限时，上限留出余量但仍小于 30 秒）。
    let view = tokio::time::timeout(Duration::from_secs(20), phone.wait_for(first_unreachable))
        .await
        .expect("第一次建连失败后就要显示连不上");
    assert!(view.details.last_failure.is_some(), "{view:?}");
    assert!(
        matches!(
            view.notice,
            Some(ConnectionNotice::Unreachable { retrying: false })
                | Some(ConnectionNotice::ServerOffline { retrying: false })
        ),
        "{view:?}"
    );
}
