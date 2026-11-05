use axum::http::Method;

use super::*;

const PLUGINS: &str = "/_wonton/api/plugins";
// ClientCore 自有 runtime，测试 runtime 的暂停时钟不能驱动它；观察跨过完整轮询周期。
const QUIET: Duration = Duration::from_secs(12);

fn requests(rig: &Rig) -> usize {
    rig.core.seen(&Method::GET, PLUGINS).len()
}

async fn next_request(rig: &Rig, before: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while requests(rig) == before {
        assert!(Instant::now() < deadline, "没有收到插件列表请求");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(requests(rig), before + 1, "重复发出了插件列表请求");
}

async fn quiet(rig: &Rig, disconnected: bool) {
    let before = requests(rig);
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        assert_eq!(requests(rig), before, "不活跃时仍在取插件列表");
        if disconnected {
            assert!(matches!(
                rig.app.home().status,
                HomeStatus::Disconnected { .. }
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(requests(rig), before);
}

async fn wait_title(rig: &Rig, pinned: &TabRef, title: &str) {
    wait_tabs(rig, |view| {
        item(view, pinned).page.as_ref().unwrap().title == title
    })
    .await;
}

async fn enter(
    rig: &Rig,
    pinned: &TabRef,
    plugin: &mut InstalledPlugin,
    title: &str,
    action: Action,
) {
    plugin.tabs[0].title = title.to_owned();
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    rig.core.hold(Method::GET, PLUGINS);
    let before = requests(rig);
    rig.app.dispatch(action).unwrap();
    next_request(rig, before, Duration::from_secs(3)).await;
    rig.core.release(Method::GET, PLUGINS);
    wait_title(rig, pinned, title).await;
}

async fn periodic_change(rig: &Rig, pinned: &TabRef, plugin: &mut InstalledPlugin) {
    let reload = item(&rig.app.tabs(), pinned).page.as_ref().unwrap().reload;
    plugin.revision.push_str("-changed");
    plugin.tabs[0].title = "periodic-response".to_owned();
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    let before = requests(rig);
    let started = Instant::now();
    next_request(rig, before, Duration::from_secs(15)).await;
    assert!(
        started.elapsed() >= Duration::from_secs(7),
        "稳定页面误用了任务的 3 秒轮询"
    );
    wait_title(rig, pinned, "periodic-response").await;
    assert_eq!(
        item(&rig.app.tabs(), pinned).page.as_ref().unwrap().reload,
        reload + 1
    );
}

#[tokio::test]
async fn chat_is_idle_pinned_page_polls_and_background_pauses_until_foreground() {
    let (rig, pinned, mut plugin) = connected_page_rig("app.poll-pinned", "home").await;
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    quiet(&rig, false).await;
    enter(
        &rig,
        &pinned,
        &mut plugin,
        "entered",
        Action::SelectTab {
            tab: pinned.clone(),
        },
    )
    .await;
    periodic_change(&rig, &pinned, &mut plugin).await;
    rig.app.dispatch(Action::AppBackgrounded).unwrap();
    quiet(&rig, false).await;
    enter(
        &rig,
        &pinned,
        &mut plugin,
        "foreground",
        Action::AppForegrounded,
    )
    .await;
}

#[tokio::test]
async fn overlay_polls_but_a_disconnected_page_does_not_request_or_redial() {
    let rig = Rig::new().await;
    let home = FakeHome::start(vec![rig.device()]).await;
    let mut plugin = web_plugin(
        "app.poll-overlay",
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home", TabDisplay::Standard)],
    );
    // 这里使用可吊销设备的家；计数和请求屏障也指向这台假核心。
    let mut rig = rig;
    rig.core = home.core.clone();
    home.core.set_lists(vec![vec![plugin.clone()]]);
    rig.connect_to(home.ticket());
    rig.wait_for(|view| {
        view.installed
            .as_ref()
            .is_some_and(|list| !list.plugins.is_empty())
    })
    .await;
    let pinned = tab(&plugin.id, Some("home"));
    rig.app
        .dispatch(Action::PinTab {
            tab: pinned.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    let overlay = tab(&plugin.id, None);
    enter(
        &rig,
        &pinned,
        &mut plugin,
        "overlay-entered",
        Action::SelectTab {
            tab: overlay.clone(),
        },
    )
    .await;
    assert_eq!(rig.app.tabs().selected, TabRef::Chat);
    let before = rig.app.tabs().overlay.unwrap().page.unwrap().reload;
    periodic_change(&rig, &pinned, &mut plugin).await;
    assert_eq!(
        rig.app.tabs().overlay.unwrap().page.unwrap().reload,
        before + 1
    );

    home.allowed.revoke(&rig.device());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(rig.app.home().status, HomeStatus::Disconnected { .. }) {
        assert!(Instant::now() < deadline, "连接没有断开");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 重新授权后如错误轮询触发重拨，将能真正连上，不能被拒绝响应掩盖。
    home.allowed.allow(rig.device());
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab { tab: overlay })
        .unwrap();
    quiet(&rig, true).await;
}
