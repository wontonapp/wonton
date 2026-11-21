//! 连不上、被拒、重试：连不上由 `HomeViewModel` 说，不进插件页；换服务器时不提前报“已连接”。

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use iroh::SecretKey;
use wonton_client::{
    Action, ConnectionNotice, HomeFailure, HomeStatus, PluginFailure, PluginTaskState,
};

use super::rig::{FakeHome, Rig, running, ticket};

/// 建连尝试可能要等到握手超时。
const DIAL: Duration = Duration::from_secs(90);

/// 等第一次建连失败，返回隧道报的原因。
async fn first_failure(rig: &Rig) -> HomeFailure {
    let deadline = Instant::now() + DIAL;
    loop {
        if let HomeStatus::Disconnected { reason, .. } = rig.app.home().status {
            return reason;
        }
        assert!(Instant::now() < deadline, "{:?}", rig.app.home());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 服务器拒绝了这台设备，重连也没用，状态不该再回到“连接中”。
async fn stays_rejected(rig: &Rig) {
    assert_eq!(first_failure(rig).await, HomeFailure::Unauthorized);
    assert_eq!(
        rig.app.home().notice,
        Some(ConnectionNotice::DeviceRejected)
    );
    let quiet = Instant::now() + Duration::from_secs(8);
    while Instant::now() < quiet {
        let status = rig.app.home().status;
        assert!(
            matches!(status, HomeStatus::Disconnected { .. }),
            "被拒绝以后又去重连了：{status:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 连不上可能只是网络断了，要自己再试；插件页上不放隧道的错误。
#[tokio::test]
async fn an_unreachable_server_is_tried_again_and_stays_out_of_the_plugin_page() {
    let rig = Rig::new().await;
    // 地址上有人占着，但从不应答。
    let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    rig.connect_to(ticket(
        SecretKey::generate().public(),
        silent.local_addr().unwrap(),
    ));
    assert_eq!(first_failure(&rig).await, HomeFailure::TransportUnavailable);

    let deadline = Instant::now() + Duration::from_secs(20);
    let reconnecting = loop {
        let home = rig.app.home();
        if home.status == HomeStatus::Connecting {
            break home;
        }
        assert!(Instant::now() < deadline, "没有再试：{home:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(reconnecting.notice, Some(ConnectionNotice::Connecting));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    assert_eq!(rig.app.plugins().problem, None);
}

#[tokio::test]
async fn a_server_that_rejects_this_device_is_reported_without_redialing() {
    let rig = Rig::new().await;
    let home = FakeHome::start(Vec::new()).await;
    rig.connect_to(home.ticket());
    stays_rejected(&rig).await;
}

/// 被拒绝时不自动重连，所以主人的重试要能接回来；连上之后两个列表都要取到。
#[tokio::test]
async fn retrying_reconnects_and_fetches_the_lists_again() {
    let rig = Rig::new().await;
    let home = FakeHome::start(Vec::new()).await;
    home.core.set_lists(vec![vec![running("app.notes")]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    rig.connect_to(home.ticket());
    stays_rejected(&rig).await;

    home.allowed.allow(rig.device());
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let view = rig
        .wait_for(|view| view.problem.is_none() && view.installed.is_some() && view.store.is_some())
        .await;
    assert_eq!(view.installed.unwrap().plugins[0].id, "app.notes");
    assert_eq!(rig.app.home().status, HomeStatus::Connected);
}

#[tokio::test]
async fn without_a_server_a_task_fails_at_once() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UninstallPlugin {
                id: "app.photos".to_owned(),
                delete_data: false,
            },
        })
        .unwrap();
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(
            state,
            PluginTaskState::Failed {
                reason: PluginFailure::TransportUnavailable,
                ..
            }
        ),
        "{state:?}"
    );
}
