//! 连不上、换服务器时的行为：连不上由 `HomeViewModel` 说，不进插件页；不把上一台的结果算到这一台头上。

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use axum::http::Method;
use iroh::SecretKey;
use wonton_client::{
    Action, ConnectionNotice, HomeFailure, HomeStatus, PluginFailure, PluginState, PluginTaskState,
};

use super::rig::{Canned, FakeHome, Rig, core_error, installed, preview, running, ticket};

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
    while rig.app.home().status != HomeStatus::Connecting {
        assert!(Instant::now() < deadline, "没有再试：{:?}", rig.app.home());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    assert_eq!(rig.app.plugins().problem, None);
    assert_eq!(rig.app.home().notice, Some(ConnectionNotice::Connecting));
}

/// 冷启动没网也能显示主屏：一打开就有上次取到的列表，不等隧道。
#[tokio::test]
async fn the_installed_list_is_there_as_soon_as_the_app_opens() {
    let mut rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running("app.notes")]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    // 列表是取到之后才放进 ViewModel 的，这时已经存下了。

    rig.reopen();

    let installed = rig.app.plugins().installed.expect("没有上次的列表");
    assert_eq!(installed.plugins[0].id, "app.notes");
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
        .dispatch(Action::RefreshStore { force: false })
        .unwrap();
    rig.connect_to(home.ticket());
    stays_rejected(&rig).await;

    home.allowed.allow(rig.device());
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
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
        .dispatch(Action::UninstallPlugin {
            id: "app.photos".to_owned(),
            delete_data: false,
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

#[tokio::test]
async fn a_list_only_clears_its_own_problem() {
    let rig = Rig::new().await;
    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Installing)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    // 每 3 秒取一次已安装列表，商店取不到的原因不能因此消失。
    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        core_error(503, "source_unavailable", "连不上 GitHub"),
    );
    rig.app
        .dispatch(Action::RefreshStore { force: false })
        .unwrap();
    rig.wait_for(|view| view.problem.is_some()).await;
    let polled = rig.core.seen(&Method::GET, "/_wonton/api/plugins").len();
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.core.seen(&Method::GET, "/_wonton/api/plugins").len() == polled {
        assert!(Instant::now() < deadline, "没有在轮询");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.app.plugins().problem.map(|problem| problem.reason),
        Some(PluginFailure::SourceUnavailable)
    );

    // 主屏上的重试连商店一起取，问题才清得掉。
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let view = rig
        .wait_for(|view| view.problem.is_none() && view.store.is_some())
        .await;
    assert!(view.installed.is_some());

    // 轮询失败也要说。
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins",
        core_error(401, "unauthenticated", "不认这台设备"),
    );
    let view = rig.wait_for(|view| view.problem.is_some()).await;
    assert_eq!(view.problem.unwrap().reason, PluginFailure::Unauthorized);
}

#[tokio::test]
async fn a_state_from_a_newer_core_is_not_taken_for_success() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Unknown)]]);
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::InstallPlugin {
            preview: preview("app.photos"),
            secrets: Default::default(),
        })
        .unwrap();
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
}

/// 它本来就是坏的：请求还没到时列表里也是这样，不能算卸载失败。
#[tokio::test]
async fn uninstalling_a_broken_plugin_over_a_dropped_connection_is_uncertain() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        "app.photos",
        PluginState::Degraded {
            reason: "容器起不来".to_owned(),
        },
    )]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: "app.photos".to_owned(),
            delete_data: false,
        })
        .unwrap();
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
}

/// 上一台上的卸载没有结果：新服务器的列表里没有那个插件，不能算卸载成功。
#[tokio::test]
async fn pairing_with_another_server_forgets_the_previous_one() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        "app.photos",
        PluginState::Uninstalling,
    )]]);
    rig.connected().await;
    rig.app
        .dispatch(Action::RefreshStore { force: false })
        .unwrap();
    rig.wait_for(|view| view.installed.is_some() && view.store.is_some())
        .await;
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: "app.photos".to_owned(),
            delete_data: false,
        })
        .unwrap();
    rig.wait_for(|view| !view.tasks.is_empty()).await;

    let next = FakeHome::start(vec![rig.device()]).await;
    next.core.set_lists(vec![vec![running("app.notes")]]);
    rig.connect_to(next.ticket());

    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
    let view = rig
        .wait_for(|view| {
            view.installed
                .as_ref()
                .is_some_and(|installed| !installed.plugins.is_empty())
        })
        .await;
    assert_eq!(view.installed.unwrap().plugins[0].id, "app.notes");
    assert_eq!(view.tasks.len(), 1);
    // 商店页可能正开着，接着取新服务器的。
    rig.wait_for(|view| view.store.is_some()).await;
    assert_eq!(next.core.seen(&Method::GET, "/_wonton/api/store").len(), 1);
}
