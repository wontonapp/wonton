//! 被服务器拒绝：报出来再说，不自动重连；主人的重试要能接回来。

use wonton_client::{Action, HomeStatus};

use super::super::rig::{FakeHome, Rig, running};
use super::stays_rejected;

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
