use axum::http::Method;
use wonton_client::PluginFailure;

use super::*;
use crate::plugins::rig::{Canned, CoreState, core_error};

const ID: &str = "app.stopped";
const START: &str = "/_wonton/api/plugins/app.stopped/start";
const LIST: &str = "/_wonton/api/plugins";

fn plugin(status: PluginState) -> InstalledPlugin {
    web_plugin(ID, status, vec![page("home", TabDisplay::Standard)])
}

fn running() -> InstalledPlugin {
    plugin(PluginState::Running {
        health: PluginHealth::Healthy,
    })
}

async fn stopped_rig() -> (Rig, TabRef) {
    // 定时探测在繁忙时会误杀健康连接、触发整轮重连和额外取列表，这里测的不是隧道恢复。
    let rig = Rig::new().await.without_periodic_checks();
    rig.core.set_lists(vec![vec![plugin(PluginState::Stopped)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    let page_tab = tab(ID, Some("home"));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    rig.wait_for(|view| !view.refreshing).await;
    (rig, page_tab)
}

fn start(rig: &Rig) -> Result<(), CoreError> {
    rig.app.dispatch(Action::StartPlugin {
        plugin_id: ID.to_owned(),
    })
}

async fn start_when_idle(rig: &Rig) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match start(rig) {
            Ok(()) => return,
            Err(CoreError::NotReady { .. }) => {
                assert!(Instant::now() < deadline, "启动请求未完成");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("启动失败：{error}"),
        }
    }
}

async fn request_count(core: &CoreState, method: Method, path: &str, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while core.seen(&method, path).len() < count {
        assert!(Instant::now() < deadline, "未收到 {method} {path}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn stopped_page_can_be_pinned_selected_and_started_without_optimistic_state() {
    let (rig, page_tab) = stopped_rig().await;
    let stopped = rig.app.tabs();
    assert_eq!(stopped.selected, page_tab);
    let stopped_page = item(&stopped, &page_tab).page.as_ref().unwrap();
    assert_eq!(stopped_page.state, TabState::Stopped);
    assert!(stopped_page.dimmed);
    assert!(!item(&stopped, &page_tab).badge);
    let reload = stopped_page.reload;
    rig.app
        .dispatch(Action::SetLanguages {
            languages: vec!["zh-Hans-CN".to_owned(), "en".to_owned()],
        })
        .unwrap();
    rig.wait_for(|view| !view.refreshing).await;
    rig.core.answer(
        Method::POST,
        START,
        Canned(200, serde_json::to_string(&running()).unwrap()),
    );
    rig.core.hold(Method::POST, START);
    start(&rig).unwrap();
    request_count(&rig.core, Method::POST, START, 1).await;
    assert!(matches!(start(&rig), Err(CoreError::NotReady { .. })));
    assert_eq!(rig.app.tabs(), stopped);
    assert!(rig.app.plugins().tasks.is_empty());
    let sent = rig.core.seen(&Method::POST, START);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].language.as_deref(), Some("zh-Hans-CN,en"));
    assert!(!sent[0].from_page);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&sent[0].body).unwrap(),
        serde_json::json!({})
    );

    // POST 已成功也不直接把响应当列表；等真实 GET 完成才进入运行状态。
    let lists_before = rig.core.seen(&Method::GET, LIST).len();
    rig.core.hold(Method::GET, LIST);
    rig.core.release(Method::POST, START);
    request_count(&rig.core, Method::GET, LIST, lists_before + 1).await;
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .state,
        TabState::Stopped
    );
    rig.core.set_lists(vec![vec![running()]]);
    rig.core.release(Method::GET, LIST);
    let ready = wait_tabs(&rig, |view| {
        let page = item(view, &page_tab).page.as_ref().unwrap();
        page.state == TabState::Ready && page.reload == reload + 1
    })
    .await;
    assert_eq!(ready.selected, page_tab);
    assert!(!item(&ready, &page_tab).page.as_ref().unwrap().dimmed);
    assert!(rig.app.plugins().problem.is_none());
}

#[tokio::test]
async fn start_success_refreshes_the_running_list_and_can_be_repeated() {
    let (rig, page_tab) = stopped_rig().await;
    rig.core.answer_always(
        Method::POST,
        START,
        Canned(200, serde_json::to_string(&running()).unwrap()),
    );
    rig.core.set_lists(vec![vec![running()]]);
    start(&rig).unwrap();
    let ready = wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
    })
    .await;
    assert!(!item(&ready, &page_tab).page.as_ref().unwrap().dimmed);
    let lists = rig.core.seen(&Method::GET, LIST).len();
    start_when_idle(&rig).await;
    request_count(&rig.core, Method::POST, START, 2).await;
    request_count(&rig.core, Method::GET, LIST, lists + 1).await;
    assert_eq!(rig.app.tabs(), ready);
}

#[tokio::test]
async fn start_rejections_remain_visible_after_unrelated_list_refreshes() {
    for (response, reason) in [
        (
            core_error(403, "unauthenticated", "设备不能管理插件"),
            PluginFailure::Unauthorized,
        ),
        (
            core_error(409, "not_ready", "插件还在卸载"),
            PluginFailure::Upstream,
        ),
        (
            core_error(500, "start_failed", "服务健康检查失败"),
            PluginFailure::Upstream,
        ),
        (
            Canned(502, String::new()),
            PluginFailure::TransportUnavailable,
        ),
        (Canned(200, "{}".to_owned()), PluginFailure::Protocol),
    ] {
        let (rig, page_tab) = stopped_rig().await;
        rig.core.answer(Method::POST, START, response);
        start(&rig).unwrap();
        let failed = rig.wait_for(|view| view.problem.is_some()).await;
        assert_eq!(failed.problem.as_ref().unwrap().reason, reason);
        assert!(!failed.problem.as_ref().unwrap().message.is_empty());
        assert!(failed.tasks.is_empty());
        let lists = rig.core.seen(&Method::GET, LIST).len();
        rig.app.dispatch(Action::RefreshPlugins).unwrap();
        request_count(&rig.core, Method::GET, LIST, lists + 1).await;
        let refreshed = rig.wait_for(|view| !view.refreshing).await;
        assert_eq!(refreshed.problem, failed.problem);
        let view = rig.app.tabs();
        let page = item(&view, &page_tab).page.as_ref().unwrap();
        assert_eq!(page.state, TabState::Stopped);
        assert!(page.dimmed);
        assert_eq!(rig.core.seen(&Method::POST, START).len(), 1);

        rig.core.set_lists(vec![vec![running()]]);
        rig.core.answer(
            Method::POST,
            START,
            Canned(200, serde_json::to_string(&running()).unwrap()),
        );
        start(&rig).unwrap();
        wait_tabs(&rig, |view| {
            item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
        })
        .await;
        assert!(rig.app.plugins().problem.is_none());
    }
}

#[tokio::test]
async fn start_without_connection_reports_a_problem_without_http_or_fake_tasks() {
    let rig = Rig::new().await;
    let before = rig.app.tabs();
    start(&rig).unwrap();
    let failed = rig.wait_for(|view| view.problem.is_some()).await;
    assert_eq!(
        failed.problem.unwrap().reason,
        PluginFailure::TransportUnavailable
    );
    assert!(failed.installed.is_none());
    assert!(failed.tasks.is_empty());
    assert_eq!(rig.app.tabs(), before);
    assert!(rig.core.seen(&Method::POST, START).is_empty());
}

#[tokio::test]
async fn start_list_refresh_failure_is_not_swallowed() {
    let (rig, page_tab) = stopped_rig().await;
    rig.core.answer(
        Method::POST,
        START,
        Canned(200, serde_json::to_string(&running()).unwrap()),
    );
    rig.core
        .answer(Method::GET, LIST, Canned(502, String::new()));
    start(&rig).unwrap();
    let failed = rig.wait_for(|view| view.problem.is_some()).await;
    assert_eq!(
        failed.problem.unwrap().reason,
        PluginFailure::TransportUnavailable
    );
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .state,
        TabState::Stopped
    );
    assert_eq!(rig.core.seen(&Method::POST, START).len(), 1);
}

#[tokio::test]
async fn stopped_state_overrides_stale_load_failure_and_running_reloads() {
    for intermediate in [None, Some(PluginState::Installing)] {
        let (rig, page_tab, running_plugin) = connected_page_rig(ID, "home").await;
        rig.app
            .dispatch(Action::TabLoadFailed {
                page: PlacedTab::Bar {
                    tab: page_tab.clone(),
                },
                status: Some(502),
            })
            .unwrap();
        let reload = item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .reload;
        rig.core.set_lists(vec![vec![plugin(PluginState::Stopped)]]);
        rig.app.dispatch(Action::RefreshPlugins).unwrap();
        let stopped = wait_tabs(&rig, |view| {
            item(view, &page_tab).page.as_ref().unwrap().state == TabState::Stopped
        })
        .await;
        assert!(item(&stopped, &page_tab).page.as_ref().unwrap().dimmed);
        // 晚到的 WebView 失败不能把已停止盖掉。
        rig.app
            .dispatch(Action::TabLoadFailed {
                page: PlacedTab::Bar {
                    tab: page_tab.clone(),
                },
                status: None,
            })
            .unwrap();
        assert_eq!(
            item(&rig.app.tabs(), &page_tab)
                .page
                .as_ref()
                .unwrap()
                .state,
            TabState::Stopped
        );
        if let Some(status) = intermediate {
            rig.core.set_lists(vec![vec![plugin(status.clone())]]);
            rig.app.dispatch(Action::RefreshPlugins).unwrap();
            wait_plugin_status(&rig, ID, status).await;
            wait_tabs(&rig, |view| {
                !item(view, &page_tab).page.as_ref().unwrap().dimmed
            })
            .await;
        }
        rig.core.set_lists(vec![vec![running_plugin]]);
        rig.app.dispatch(Action::RefreshPlugins).unwrap();
        let ready = wait_tabs(&rig, |view| {
            let page = item(view, &page_tab).page.as_ref().unwrap();
            page.state == TabState::Ready && page.reload == reload + 1
        })
        .await;
        assert!(!item(&ready, &page_tab).page.as_ref().unwrap().dimmed);
        rig.app
            .dispatch(Action::ReloadTab {
                page: PlacedTab::Bar {
                    tab: page_tab.clone(),
                },
            })
            .unwrap();
        assert_eq!(
            item(&rig.app.tabs(), &page_tab)
                .page
                .as_ref()
                .unwrap()
                .reload,
            reload + 2
        );
    }
}

async fn assert_old_start_ignored(rig: &Rig, next: &FakeHome, page_tab: &TabRef) {
    let old_lists = rig.core.seen(&Method::GET, LIST).len();
    let new_lists = next.core.seen(&Method::GET, LIST).len();
    rig.core.release(Method::POST, START);
    // 给旧响应到达的机会，期间持续检查，不能把新家的页面/错误覆盖掉或触发额外 GET。
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        assert!(rig.app.plugins().problem.is_none());
        assert_eq!(
            item(&rig.app.tabs(), page_tab).page.as_ref().unwrap().state,
            TabState::Stopped
        );
        assert_eq!(rig.core.seen(&Method::GET, LIST).len(), old_lists);
        assert_eq!(next.core.seen(&Method::GET, LIST).len(), new_lists);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn switching_home_ignores_old_start_success_and_failure() {
    for response in [
        Canned(200, serde_json::to_string(&running()).unwrap()),
        core_error(500, "start_failed", "旧家的启动失败"),
    ] {
        let (rig, _) = stopped_rig().await;
        rig.core.answer(Method::POST, START, response);
        rig.core.hold(Method::POST, START);
        start(&rig).unwrap();
        request_count(&rig.core, Method::POST, START, 1).await;
        let next = FakeHome::start(vec![rig.device()]).await;
        let mut next_plugin = plugin(PluginState::Stopped);
        next_plugin.version = "2.0.0".to_owned();
        next.core.set_lists(vec![vec![next_plugin]]);
        rig.connect_to(next.ticket());
        rig.wait_for(|view| {
            !view.refreshing
                && view.installed.as_ref().is_some_and(|list| {
                    list.plugins
                        .first()
                        .is_some_and(|plugin| plugin.version == "2.0.0")
                })
        })
        .await;
        wait_tabs(&rig, |view| {
            view.items
                .iter()
                .all(|item| !matches!(item.tab, TabRef::Plugin { .. }))
        })
        .await;
        let page_tab = tab(ID, Some("home"));
        rig.app
            .dispatch(Action::PinTab {
                tab: page_tab.clone(),
                replacing: None,
            })
            .unwrap();
        rig.app
            .dispatch(Action::SelectTab {
                tab: page_tab.clone(),
            })
            .unwrap();
        next.core.answer(
            Method::POST,
            START,
            Canned(200, serde_json::to_string(&running()).unwrap()),
        );
        next.core.hold(Method::POST, START);
        start(&rig).unwrap();
        request_count(&next.core, Method::POST, START, 1).await;
        assert_old_start_ignored(&rig, &next, &page_tab).await;
        assert!(matches!(start(&rig), Err(CoreError::NotReady { .. })));
        next.core.set_lists(vec![vec![running()]]);
        next.core.release(Method::POST, START);
        wait_tabs(&rig, |view| {
            item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
        })
        .await;
        assert_eq!(next.core.seen(&Method::POST, START).len(), 1);
    }
}

#[tokio::test]
async fn stopped_implicit_web_page_is_dimmed_and_removed_pages_keep_precedence() {
    let (rig, page_tab) = stopped_rig().await;
    let implicit = tab(ID, None);
    rig.app
        .dispatch(Action::SelectTab {
            tab: implicit.clone(),
        })
        .unwrap();
    let overlay = rig.app.tabs().overlay.unwrap();
    assert_eq!(overlay.tab, implicit);
    let overlay_page = overlay.page.unwrap();
    assert_eq!(overlay_page.state, TabState::Stopped);
    assert!(overlay_page.dimmed);
    let mut removed = plugin(PluginState::Stopped);
    removed.tabs.clear();
    rig.core.set_lists(vec![vec![removed]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let view = wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::PageRemoved
    })
    .await;
    assert!(item(&view, &page_tab).badge);
    assert!(!item(&view, &page_tab).page.as_ref().unwrap().dimmed);
}

#[tokio::test]
async fn switching_home_ignores_old_start_list_refresh() {
    let (rig, _) = stopped_rig().await;
    rig.core.answer(
        Method::POST,
        START,
        Canned(200, serde_json::to_string(&running()).unwrap()),
    );
    rig.core.hold(Method::GET, LIST);
    let lists = rig.core.seen(&Method::GET, LIST).len();
    start(&rig).unwrap();
    request_count(&rig.core, Method::GET, LIST, lists + 1).await;
    rig.core.set_lists(vec![vec![running()]]);
    let next = FakeHome::start(vec![rig.device()]).await;
    let mut next_plugin = plugin(PluginState::Stopped);
    next_plugin.version = "2.0.0".to_owned();
    next.core.set_lists(vec![vec![next_plugin]]);
    rig.connect_to(next.ticket());
    rig.wait_for(|view| {
        !view.refreshing
            && view.installed.as_ref().is_some_and(|list| {
                list.plugins
                    .first()
                    .is_some_and(|plugin| plugin.version == "2.0.0")
            })
    })
    .await;
    wait_tabs(&rig, |view| {
        view.items
            .iter()
            .all(|item| !matches!(item.tab, TabRef::Plugin { .. }))
    })
    .await;
    let page_tab = tab(ID, Some("home"));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.core.release(Method::GET, LIST);
    assert_old_start_ignored(&rig, &next, &page_tab).await;
    assert_eq!(
        rig.app.plugins().installed.unwrap().plugins[0].version,
        "2.0.0"
    );
}
