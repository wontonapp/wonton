//! 换服务器：上一台没做完的事不算到这一台头上，旧连接的收尾事件也不能把视图盖回“已连接”。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, HomeStatus, PluginState, PluginTaskState};

use super::rig::{CONNECT_BACKSTOP, Canned, FakeHome, Rig, WAIT, installed, running};

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
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    rig.wait_for(|view| view.installed.is_some() && view.store.is_some())
        .await;
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(502, String::new()),
    );
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
    rig.wait_for(|view| !view.tasks.is_empty()).await;
    // 请求真的出了门（假核心先记 seen）再换服务器：出门前就换，新连接还没建起来时发送任务
    // 会按“没发出去”判失败，测不到“上一台的卸载没有结果”这一条。
    let deadline = Instant::now() + WAIT;
    while rig
        .core
        .seen(&Method::DELETE, "/_wonton/api/plugins/app.photos")
        .is_empty()
    {
        assert!(Instant::now() < deadline, "卸载请求没有出门");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let next = FakeHome::start(vec![rig.device()]).await;
    next.core.set_lists(vec![vec![running("app.notes")]]);
    rig.connect_to(next.ticket());

    let state = rig.task("app.photos", |state| !state.in_progress()).await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
    assert!(!rig.app.plugins().tasks[0].waiting_for_connection);
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

/// 换服务器时，旧连接的收尾事件不能把视图盖回“已连接”：新连接的本地代理还没进视图（`proxy`
/// 是 `None`）时，视图必须停在“连接中”。盖回去的话，这个窗口里发出的请求全部按“连不上”失败，
/// 插件页还会把假问题记下来，连上后连带多取一轮（#826 的两条偶发）。
#[tokio::test]
async fn switching_servers_does_not_report_connected_before_the_new_proxy() {
    let rig = Rig::new().await;
    rig.connected().await;
    let next = FakeHome::start(vec![rig.device()]).await;
    let home = rig.app.subscribe_home();
    home.next().await.unwrap();
    rig.connect_to(next.ticket());
    // `RetryConnection` 触发的重算会唤醒还没被收尾的旧连接 watcher：没有守卫时，它按旧客户端
    // 仍是“已连接”的状态重算视图，把刚写下的“连接中”盖回“已连接”。
    rig.app
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RetryConnection,
        })
        .unwrap();
    let deadline = Instant::now() + CONNECT_BACKSTOP;
    loop {
        // 等待本身限时：连接挂着不动时视图不来，deadline 要能兜底。
        let remaining = deadline.saturating_duration_since(Instant::now());
        let view = tokio::time::timeout(remaining, home.next())
            .await
            .unwrap_or_else(|_| panic!("新服务器没连上：{:?}", rig.app.home()))
            .expect("连接订阅提前结束");
        if view.status == HomeStatus::Connected {
            assert!(view.proxy.is_some(), "新连接还没就位就报了已连接：{view:?}");
        }
        if view.status == HomeStatus::Connected && view.proxy.is_some() {
            return;
        }
    }
}
