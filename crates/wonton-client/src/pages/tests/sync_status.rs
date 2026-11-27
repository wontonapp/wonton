//! 同步状态：“改动还没同步上”只在本机真有没写上的改动时显示，断线沿用缓存，同步上了就收掉。

use std::sync::atomic::Ordering;

use super::*;

fn offline() -> HomeViewModel {
    let mut view = home("old-server");
    view.status = HomeStatus::Disconnected {
        reason: HomeFailure::TransportUnavailable,
        message: "offline".to_owned(),
    };
    view
}

/// 本机改过、写或对账没成：显示“改动还没同步上”；写成了（改动推上去、原因清掉）就收掉。
#[tokio::test]
async fn sync_stale_needs_a_local_change_that_did_not_reach_the_server() {
    let rig = Rig::new();
    rig.home.send_replace(home("old-server"));
    rig.add_app(MyAppRef::Plugin {
        plugin_id: "app.shared".to_owned(),
    });
    assert!(rig.tabs.dirty.load(Ordering::SeqCst), "本机改了还没推");
    assert!(!rig.view().sync_stale, "还没试过写，不算没同步上");

    rig.tabs
        .sync_problem
        .send_replace(Some("offline".to_owned()));
    rig.tabs.recompute();
    assert!(rig.view().sync_stale, "写没成该显示“没同步上”");

    // 推上去了：push_layout 先清改动再清原因。
    rig.tabs.dirty.store(false, Ordering::SeqCst);
    rig.tabs.sync_problem.send_replace(None);
    rig.tabs.recompute();
    assert!(!rig.view().sync_stale, "同步上了收掉");
}

/// 只是断了、本机没改过：列表用缓存照常显示（盲测 bug #8），不显示“改动还没同步上”——侧边栏顶上已经说了连不上，
/// 再说“本机已存好”是假话。断着时改了，才显示。
#[tokio::test]
async fn going_offline_alone_is_not_an_unsynced_change() {
    let rig = Rig::new();
    rig.add_app(MyAppRef::Plugin {
        plugin_id: "app.shared".to_owned(),
    });
    // 这次改动已经推上去了。
    rig.tabs.dirty.store(false, Ordering::SeqCst);
    rig.tabs.sync_problem.send_replace(None);
    let titles = rig.my_app_titles();
    assert_eq!(titles.len(), 1);

    rig.home.send_replace(offline());
    rig.tabs.recompute();
    assert_eq!(rig.my_app_titles(), titles, "断线不改列表");
    assert!(!rig.view().sync_stale, "没改过就没有要同步的");
    assert!(rig.view().problem.is_none(), "断线不该产生弹窗类状态");

    rig.add_app(MyAppRef::Page {
        tab: url_tab("https://example.com/report"),
    });
    assert_eq!(rig.my_app_titles().len(), 2, "断着也能改，本机存好");
    assert!(rig.view().sync_stale, "断着改的要说还没同步上");
}
