//! 插件网页要摄像头/麦克风的询问与记住（ADR-0033）：回答落盘、重载后不再问。

use super::*;
use crate::TabMediaPermission;

/// 给测试断言用的数据目录：插件决定记在槽位目录的 `plugin-media.json`。
impl Rig {
    fn dir(&self) -> &std::path::Path {
        self._dir.path()
    }

    fn placed_page(&self) -> PlacedTab {
        PlacedTab::Page { tab: page_tab() }
    }

    fn origin() -> &'static str {
        "http://app.shared.wonton.localhost/"
    }
}

#[tokio::test]
async fn first_request_asks_and_allow_is_remembered_across_reload() {
    let rig = Rig::new();
    rig.plugins.send_replace(plugins("1"));
    rig.open(page_tab());

    // 没问过：出询问，网页那头还没拿到结果。
    let first = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(first, TabMediaPermission::Prompt));
    assert!(rig.view().media_prompt.is_some());

    // 回答允许：落盘，询问清掉。
    let answered = rig.tabs.decide_media_permission(true).unwrap();
    assert!(matches!(answered, TabMediaPermission::Allow));
    assert!(rig.view().media_prompt.is_none());
    let file = rig.dir().join("plugin-media.json");
    assert!(file.exists(), "the decision must be written to disk");
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(
        saved.contains("app.shared"),
        "the decision is per plugin id"
    );

    // 重载（新一次 didFinish 重新请求）：记住的决定直接放行，不再询问。
    let again = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(again, TabMediaPermission::Allow));
    assert!(rig.view().media_prompt.is_none());
}

#[tokio::test]
async fn deny_is_remembered_and_reload_is_refused_without_asking() {
    let rig = Rig::new();
    rig.plugins.send_replace(plugins("1"));
    rig.open(page_tab());

    let first = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(first, TabMediaPermission::Prompt));

    let answered = rig.tabs.decide_media_permission(false).unwrap();
    assert!(matches!(answered, TabMediaPermission::Deny));

    let again = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(again, TabMediaPermission::Deny));
    assert!(rig.view().media_prompt.is_none());
}

#[tokio::test]
async fn deciding_without_pending_prompt_errors_and_keeps_state() {
    let rig = Rig::new();
    rig.plugins.send_replace(plugins("1"));
    rig.open(page_tab());

    let nothing = rig.tabs.decide_media_permission(true);
    assert!(matches!(nothing, Err(CoreError::InvalidInput { .. })));
    assert!(rig.view().media_prompt.is_none());
}

#[tokio::test]
async fn plugin_state_update_between_request_and_decide_keeps_the_pending_prompt() {
    let rig = Rig::new();
    // 走查的真实顺序：插件懒启动，页面在 Stopped 时打开（出「已停止/Start」），点了启动容器才转 Running。
    let mut stopped = plugins("1");
    stopped.installed.as_mut().unwrap().plugins[0].status = PluginState::Stopped;
    rig.plugins.send_replace(stopped);
    rig.open(page_tab());

    let first = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(first, TabMediaPermission::Prompt));

    // 弹窗开着时容器起起来了：publish 路径上的 clear_gone_prompt 不能把没答的询问清掉。
    rig.plugins.send_replace(plugins("1"));

    let answered = rig.tabs.decide_media_permission(true).unwrap();
    assert!(matches!(answered, TabMediaPermission::Allow));
    assert!(rig.view().media_prompt.is_none());
    assert!(rig.dir().join("plugin-media.json").exists());
}

#[tokio::test]
async fn same_page_rerequest_replaces_the_pending_and_the_old_request_is_rejected() {
    let rig = Rig::new();
    rig.plugins.send_replace(plugins("1"));
    rig.open(page_tab());
    // 订在第一次请求前：壳看到的每次 ViewModel 更新都能收进来。
    let mut updates = rig.tabs.view.subscribe();

    // 第一份：只要麦克风。
    let first = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), false, true)
        .unwrap();
    assert!(matches!(first, TabMediaPermission::Prompt));
    assert!(updates.has_changed().unwrap(), "第一份的询问该发给壳");
    let published = updates.borrow_and_update().clone();
    assert_eq!(
        published
            .media_prompt
            .map(|prompt| (prompt.camera, prompt.microphone)),
        Some((false, true))
    );

    // 同页再来一份（这次连摄像头一起）：旧的那份按“被新请求替换”收场（壳收到这次的
    // Prompt 就拒掉旧的网页请求），ViewModel 上只剩第二份。
    let second = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), true, true)
        .unwrap();
    assert!(matches!(second, TabMediaPermission::Prompt));
    assert!(updates.has_changed().unwrap(), "替换后该再发一次更新");
    let published = updates.borrow_and_update().clone();
    assert_eq!(
        published
            .media_prompt
            .map(|prompt| (prompt.camera, prompt.microphone)),
        Some((true, true)),
        "挂着的应是第二份的请求"
    );

    // 回答只作用于挂着的第二份：记下决定、清掉询问，落盘里只有这一条。
    let answered = rig.tabs.decide_media_permission(true).unwrap();
    assert!(matches!(answered, TabMediaPermission::Allow));
    assert!(rig.view().media_prompt.is_none());
    let saved = std::fs::read_to_string(rig.dir().join("plugin-media.json")).unwrap();
    assert!(
        saved.contains("\"app.shared\":true"),
        "记下的应是允许: {saved}"
    );

    // 同款再要：第二份记下的决定直接放行，不再询问。
    let again = rig
        .tabs
        .request_media_permission(&rig.placed_page(), Rig::origin(), true, true)
        .unwrap();
    assert!(matches!(again, TabMediaPermission::Allow));
    assert!(rig.view().media_prompt.is_none());
}
