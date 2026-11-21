//! 连接断了而请求还吊着：任务先说在等连接、不接受重复操作，连回来接着报结果。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, CoreError, HomeStatus, PluginState, PluginTaskState};

use super::rig::{CONNECT_BACKSTOP, Canned, Rig, WAIT, installed, preview, running};

/// 请求发出后连接断了：任务要说在等连接，不能一直像在做；连回来就接着报结果。
#[tokio::test]
async fn a_dropped_install_waits_for_a_connection_and_then_reports_the_result() {
    let rig = Rig::new().await;
    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Installing)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.photos" && task.waiting_for_connection)
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Running);
    // 等连接时既不能清掉任务，也不能对同一个插件再发一次。
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissPluginTask {
                id: "app.photos".to_owned(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));

    // 列表又问得到了：回到“在做”。
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.photos" && !task.waiting_for_connection)
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Running);

    // 再断一次，重新说在等连接。
    rig.core.answer_always(
        Method::GET,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.wait_for(|view| view.tasks.iter().any(|task| task.waiting_for_connection))
        .await;

    // 连回来时它已经跑起来了：成功。
    rig.core.stop_answering(Method::GET, "/_wonton/api/plugins");
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    let state = rig.task("app.photos", |state| !state.in_progress()).await;
    assert_eq!(state, PluginTaskState::Succeeded);
    assert!(!rig.app.plugins().tasks[0].waiting_for_connection);
}

/// 请求还吊着时断网：不等请求报错就先说在等连接，连回来再回到在做。
#[tokio::test]
async fn losing_the_network_while_the_request_is_out_waits_at_once() {
    // 测的是任务跟着连接状态走，不是隧道恢复：繁忙时定时探测卡满时限会把健康连接断掉、
    // 触发整轮重连，把测试要的断开时点挤乱（见 [`Rig::without_periodic_checks`]）。
    let rig = Rig::new().await.without_periodic_checks();
    // 建连是真实握手加令牌请求，繁忙时实测 16s+（见 [`CONNECT_BACKSTOP`]）：等事件，预算只兜底。
    rig.connected_within(CONNECT_BACKSTOP).await;
    rig.core.hold(Method::POST, "/_wonton/api/plugins");
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();

    // 请求真的出了门（假核心先记 seen 再按住）再断网：断网赶在发请求前面的话，连接信息
    // 已经没了，任务按“没发出去”直接报失败（见 `PluginsLink::install`），测的就是另一条路径。
    let deadline = Instant::now() + WAIT;
    while rig
        .core
        .seen(&Method::POST, "/_wonton/api/plugins")
        .is_empty()
    {
        assert!(
            Instant::now() < deadline,
            "请求没出门：{:?}",
            rig.app.plugins()
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    // 请求还挂着，连接断了：不等请求报错就说在等连接。
    rig.drop_home().await;
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.photos" && task.waiting_for_connection)
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Running);
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissPluginTask {
                id: "app.photos".to_owned(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));

    // 连接回来：回到在做。断开期间隧道自己会按自己的节奏重新发布状态，单次设置会被它
    // 盖掉：一个小任务一直设着“已连接”，直到任务不再等连接；主测等视图事件，不按
    // 固定节奏轮询自己。视图事件本身就是完成条件，wait_for 的预算只兜底。
    let guard = {
        let app = rig.app.clone();
        tokio::spawn(async move {
            loop {
                app.set_home_status_for_test(HomeStatus::Connected);
                if !app
                    .plugins()
                    .tasks
                    .iter()
                    .any(|task| task.waiting_for_connection)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
    };
    let view = rig
        .wait_for(|view| !view.tasks.iter().any(|task| task.waiting_for_connection))
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Running);
    guard.abort();
}
