use super::*;

use wonton_client::PanelScope;

fn main_scope() -> PanelScope {
    PanelScope::Main
}

fn reload(view: &PagesViewModel, page_tab: &TabRef) -> u64 {
    item(view, page_tab).reload
}

// 改标题作为响应已进入 tabs ViewModel 的屏障，避免“没刷新”断言早于响应处理。
async fn refresh_page(rig: &Rig, page_tab: &TabRef, mut plugin: InstalledPlugin, marker: &str) {
    plugin.tabs[0].title = marker.to_owned();
    rig.core.set_lists(vec![vec![plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_pages(rig, |view| item(view, page_tab).title == marker).await;
}

#[tokio::test]
async fn same_version_content_change_reloads_stack_and_panel_instances_once() {
    let (rig, pinned, mut plugin) = connected_page_rig("app.revision", "home").await;
    assert_eq!(reload(&rig.app.pages(), &pinned), 0);
    // 同一个页面在栈里和面板里是两份实例：都重新加载，各记各的计数。
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::AddPanelPage {
                scope: main_scope(),
                tab: pinned.clone(),
            },
        })
        .unwrap();
    let before = rig.app.pages();
    let stack_before = reload(&before, &pinned);
    let panel_before = before.panel[0].page.reload;
    assert_eq!(panel_before, 0);
    let version = plugin.version.clone();
    plugin.revision = "opaque/new-content".to_owned();
    refresh_page(&rig, &pinned, plugin.clone(), "changed").await;
    let changed = rig.app.pages();
    assert_eq!(reload(&changed, &pinned), stack_before + 1);
    assert_eq!(changed.panel[0].page.reload, panel_before + 1);
    assert_eq!(
        rig.app.plugins().installed.unwrap().plugins[0].version,
        version
    );

    refresh_page(&rig, &pinned, plugin, "same-content").await;
    let unchanged = rig.app.pages();
    assert_eq!(reload(&unchanged, &pinned), stack_before + 1);
    assert_eq!(unchanged.panel[0].page.reload, panel_before + 1);
}

#[tokio::test]
async fn upgrading_then_rollback_with_unchanged_revision_does_not_reload() {
    let (rig, pinned, running) = connected_page_rig("app.rollback", "home").await;
    let before = reload(&rig.app.pages(), &pinned);
    let mut upgrading = running.clone();
    upgrading.status = PluginState::Upgrading;
    refresh_page(&rig, &pinned, upgrading, "upgrading").await;
    assert_eq!(reload(&rig.app.pages(), &pinned), before);
    refresh_page(&rig, &pinned, running, "rolled-back").await;
    let rolled_back = rig.app.pages();
    assert_eq!(item(&rolled_back, &pinned).state, TabState::Ready);
    assert_eq!(reload(&rolled_back, &pinned), before);
}

#[tokio::test]
async fn first_appearance_and_another_servers_revision_do_not_reload() {
    let (rig, pinned, mut plugin) = connected_page_rig("app.first-revision", "home").await;
    assert_eq!(reload(&rig.app.pages(), &pinned), 0);
    plugin.revision = "different-server-content".to_owned();
    plugin.tabs[0].title = "other-server".to_owned();
    let other = FakeHome::start(vec![rig.device()]).await;
    other.core.set_lists(vec![vec![plugin]]);
    rig.connect_to(other.ticket());
    rig.wait_for(|view| {
        view.installed.as_ref().is_some_and(|installed| {
            installed
                .plugins
                .iter()
                .any(|plugin| plugin.revision == "different-server-content")
        })
    })
    .await;
    // 换服务器把插件行和栈都清了；重新打开按另一台的清单来，计数从零开始。
    wait_pages(&rig, |view| {
        view.stack.iter().all(|open| open.tab != pinned)
    })
    .await;
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage {
                tab: pinned.clone(),
            },
        })
        .unwrap();
    let other_page = wait_pages(&rig, |view| item(view, &pinned).title == "other-server").await;
    assert_eq!(reload(&other_page, &pinned), 0);
}
