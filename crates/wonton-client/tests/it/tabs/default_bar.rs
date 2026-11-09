//! 配对后的底栏默认项：只剩"聊天"不显示整条；老配置里残留的"应用"重连时被丢掉。

use super::*;
use wonton_client::Action;

#[tokio::test]
async fn paired_bar_starts_with_chat_only_and_hides_until_something_is_pinned() {
    let rig = Rig::new().await;
    // 没配对时“应用”还在栏上，是配对页，底栏照常显示。
    let unpaired = rig.app.tabs();
    assert_eq!(
        unpaired
            .items
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![TabRef::Chat, TabRef::Apps, TabRef::Settings]
    );
    assert!(unpaired.show_bar);

    rig.connected().await;
    // 配上了：底栏默认只有“聊天”，整条底栏不显示。
    let paired = wait_tabs(&rig, |view| view.items.len() == 1).await;
    assert_eq!(paired.items[0].tab, TabRef::Chat);
    assert!(!paired.show_bar);

    rig.app
        .dispatch(Action::PinTab {
            tab: TabRef::Settings,
            replacing: None,
        })
        .unwrap();
    let pinned = rig.app.tabs();
    assert_eq!(
        pinned
            .items
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![TabRef::Chat, TabRef::Settings]
    );
    assert!(pinned.show_bar);

    rig.app
        .dispatch(Action::UnpinTab {
            tab: TabRef::Settings,
        })
        .unwrap();
    assert!(!rig.app.tabs().show_bar);
}

#[tokio::test]
async fn an_old_config_with_the_apps_page_loses_it_on_reconnect() {
    let mut rig = Rig::new().await;
    rig.connected().await;
    rig.app
        .dispatch(Action::PinTab {
            tab: TabRef::Settings,
            replacing: None,
        })
        .unwrap();
    let path = rig.data_dir().join("tabs.json");
    // 老安装的文件里“应用”还在栏上。
    let mut file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    file["items"]
        .as_array_mut()
        .unwrap()
        .insert(1, serde_json::json!({ "tab": { "type": "apps" } }));
    std::fs::write(&path, file.to_string()).unwrap();

    rig.reopen();
    let reopened = wait_tabs(&rig, |view| {
        !view.items.iter().any(|item| item.tab == TabRef::Apps)
    })
    .await;
    assert_eq!(
        reopened
            .items
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![TabRef::Chat, TabRef::Settings]
    );
    assert!(reopened.show_bar);
}

#[tokio::test]
async fn apps_page_stays_selected_without_being_on_the_bar() {
    let rig = Rig::new().await;
    rig.connected().await;
    wait_tabs(&rig, |view| view.items.len() == 1).await;
    // 网格页从侧边栏进：不在栏上也能选中，整条底栏照旧不显示。
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Apps })
        .unwrap();
    assert_eq!(rig.app.tabs().selected, TabRef::Apps);
    assert!(!rig.app.tabs().show_bar);
    // 刷新时选中不被弹回聊天。
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    assert_eq!(rig.app.tabs().selected, TabRef::Apps);
    // 回聊天仍然正常。
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    assert_eq!(rig.app.tabs().selected, TabRef::Chat);
}
