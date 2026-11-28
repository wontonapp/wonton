//! 建连：连不上可能只是网络断了，要自己再试；没有服务器时任务直接失败，不留在插件页。

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use iroh::SecretKey;
use wonton_client::{
    Action, ConnectionNotice, HomeFailure, HomeStatus, PluginFailure, PluginTaskState,
};

use super::super::rig::{Rig, ticket};
use super::first_failure;

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
async fn without_a_server_a_task_fails_at_once() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UninstallPlugin {
                id: "app.photos".to_owned(),
                delete_data: false,
                providers: Vec::new(),
                dependents: Vec::new(),
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
                reason: PluginFailure::NotPaired,
                ..
            }
        ),
        "{state:?}"
    );
}
