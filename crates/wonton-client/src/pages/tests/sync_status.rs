//! 同步状态：同步没跟上标“可能不是最新的”，断线沿用缓存并在重连后清除。

use super::*;

/// 同步没跟上如实显示为不确定（不是错误）：对账/推送失败置位，成功清掉（失败即停）。
#[tokio::test]
async fn sync_stale_follows_sync_problem() {
    let rig = Rig::new();
    rig.home.send_replace(home("old-server"));
    assert!(!rig.view().sync_stale);
    rig.tabs
        .sync_problem
        .send_replace(Some("offline".to_owned()));
    rig.tabs.recompute();
    assert!(rig.view().sync_stale, "同步失败该显示“没同步上”");
    rig.tabs.sync_problem.send_replace(None);
    rig.tabs.recompute();
    assert!(!rig.view().sync_stale, "同步成功后清掉");
}

/// 断线时“我的应用”用缓存照常显示（盲测 bug #8）：旧列表保留、只标“可能不是最新的”，
/// 不产生弹窗类状态（problem 留给本机存储故障）。重连对账成功后由同步路径清标记。
#[tokio::test]
async fn disconnected_keeps_cached_my_apps_and_marks_stale_without_a_problem() {
    let rig = Rig::new();
    rig.add_app(MyAppRef::Plugin {
        plugin_id: "app.shared".to_owned(),
    });
    let titles = rig.my_app_titles();
    assert_eq!(titles.len(), 1);
    assert!(!rig.view().sync_stale);

    let mut offline = home("old-server");
    offline.status = HomeStatus::Disconnected {
        reason: HomeFailure::TransportUnavailable,
        message: "offline".to_owned(),
    };
    rig.home.send_replace(offline);
    rig.tabs.recompute();

    assert_eq!(rig.my_app_titles(), titles, "断线不改列表");
    assert!(rig.view().sync_stale, "断线该标“可能不是最新的”");
    assert!(rig.view().problem.is_none(), "断线不该产生弹窗类状态");

    // 断线下的重试是空操作但不报错：提示原样留着，等重连后自动对账。
    rig.tabs.dispatch(PageAction::RetryMyAppSync).unwrap();
    assert_eq!(rig.my_app_titles(), titles);
    assert!(rig.view().sync_stale);
}
