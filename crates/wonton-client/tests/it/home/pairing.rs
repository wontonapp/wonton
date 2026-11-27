//! 未登记设备的对接待办：如实说“要配对”、停下重试，配好之后“重试”能连上。

use std::time::Duration;

use wonton_client::{Action, ConnectionNotice, HomeFailure, HomeStatus};

use super::support::{Home, Phone};

#[tokio::test]
async fn an_unregistered_device_is_told_it_needs_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    assert!(
        matches!(
            view.status,
            HomeStatus::Disconnected {
                reason: HomeFailure::Unauthorized,
                ..
            }
        ),
        "{view:?}"
    );
    assert_eq!(view.notice, Some(ConnectionNotice::DeviceRejected));
}

#[tokio::test]
async fn an_unregistered_device_stops_trying_until_it_is_paired() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    let before = opened.get();

    // 比定时检查的间隔长：重试也没用，状态也不该在“连接中”和“要配对”之间跳。
    tokio::time::sleep(Duration::from_secs(7)).await;

    assert_eq!(opened.get(), before);
    assert!(matches!(
        phone.home().status,
        HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            ..
        }
    ));
}
