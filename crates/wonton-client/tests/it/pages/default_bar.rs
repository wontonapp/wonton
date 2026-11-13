//! 根页面：没配对时是应用网格页（配对页在它上面），配上之后是聊天；栈空时页面都在侧边栏里。

use super::*;
use wonton_client::Action;

#[tokio::test]
async fn root_is_the_apps_page_unpaired_and_chat_once_paired() {
    let rig = Rig::new().await;
    // 没配对时根页面是“应用”：网格页就是配对页。
    let unpaired = rig.app.pages();
    assert_eq!(unpaired.root, TabRef::Apps);
    assert!(unpaired.stack.is_empty());
    assert!(!unpaired.shows_apps);

    rig.connected().await;
    // 配上了：根页面回到聊天，栈还是空的。
    let paired = wait_pages(&rig, |view| view.root == TabRef::Chat).await;
    assert_eq!(paired.root, TabRef::Chat);
    assert!(paired.stack.is_empty());
    assert!(!paired.shows_apps);
    // 候选和我的应用都在核心里算好，侧边栏从那里渲染。
    assert!(paired.my_app_candidates.is_empty());
}

#[tokio::test]
async fn an_old_config_with_bar_items_starts_from_the_new_defaults() {
    let mut rig = Rig::new().await;
    rig.connected().await;
    // 老安装的 layout.json 是底栏的形状（items/displays/labels）：读不出来就按默认值。
    std::fs::write(
        rig.data_dir().join("layout.json"),
        serde_json::json!({
            "server_id": "home",
            "items": [{"tab": {"type": "chat"}}, {"tab": {"type": "settings"}}],
            "labels": false,
        })
        .to_string(),
    )
    .unwrap();
    rig.reopen();
    let reopened = rig.app.pages();
    // items/labels 不再是字段：整份配置退回默认，原因照旧说清。
    assert!(reopened.problem.is_some());
    assert!(reopened.auto_add);
    assert!(reopened.my_app_candidates.is_empty());
}

#[tokio::test]
async fn sidebar_and_deep_link_open_the_apps_grid_over_chat() {
    let rig = Rig::new().await;
    rig.connected().await;
    wait_pages(&rig, |view| view.root == TabRef::Chat).await;

    let plugin_id = "app.grid";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    )]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some("home"));
    rig.app
        .dispatch(Action::OpenPage {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert_eq!(rig.app.pages().stack.len(), 1);

    // 侧边栏“全部”发同一个 OpenPage(Apps) 操作。
    rig.app
        .dispatch(Action::OpenPage { tab: TabRef::Apps })
        .unwrap();
    assert!(rig.app.pages().stack.is_empty());
    assert_eq!(rig.app.pages().root, TabRef::Chat);
    assert!(rig.app.pages().shows_apps);
    // 插件刷新不吞掉网格的打开请求。
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    assert_eq!(rig.app.pages().root, TabRef::Chat);
    assert!(rig.app.pages().shows_apps);

    rig.app.dispatch(Action::ClosePage).unwrap();
    assert!(!rig.app.pages().shows_apps);
    let wonton_protocol::AppLink::Tab { tab } =
        wonton_client::parse_app_link("wonton://tab/apps".to_owned()).unwrap()
    else {
        panic!("expected a page link");
    };
    rig.app.dispatch(Action::OpenPage { tab }).unwrap();
    let linked = rig.app.pages();
    assert_eq!(linked.root, TabRef::Chat);
    assert!(linked.stack.is_empty());
    assert!(linked.shows_apps);

    rig.app
        .dispatch(Action::OpenPage { tab: TabRef::Chat })
        .unwrap();
    assert!(!rig.app.pages().shows_apps);
}
