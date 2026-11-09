use super::*;

fn reload(view: &TabsViewModel, page_tab: &TabRef) -> u64 {
    item(view, page_tab).page.as_ref().unwrap().reload
}

// 改标题作为响应已进入 tabs ViewModel 的屏障，避免“没刷新”断言早于响应处理。
async fn refresh_page(rig: &Rig, page_tab: &TabRef, mut plugin: InstalledPlugin, marker: &str) {
    plugin.tabs[0].title = marker.to_owned();
    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_tabs(rig, |view| {
        item(view, page_tab).page.as_ref().unwrap().title == marker
    })
    .await;
}

#[tokio::test]
async fn same_version_content_change_reloads_cached_pinned_and_overlay_pages_once() {
    let (rig, pinned, mut plugin) = connected_page_rig("app.revision", "home").await;
    assert_eq!(reload(&rig.app.tabs(), &pinned), 0);
    rig.app
        .dispatch(Action::SelectTab {
            tab: pinned.clone(),
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    let overlay = tab(&plugin.id, None);
    rig.app
        .dispatch(Action::SelectTab {
            tab: overlay.clone(),
        })
        .unwrap();
    let before = rig.app.tabs();
    let pinned_before = reload(&before, &pinned);
    let overlay_before = before
        .overlay
        .as_ref()
        .unwrap()
        .page
        .as_ref()
        .unwrap()
        .reload;
    assert_eq!(overlay_before, 0);
    let version = plugin.version.clone();
    plugin.revision = "opaque/new-content".to_owned();
    refresh_page(&rig, &pinned, plugin.clone(), "changed").await;
    let changed = rig.app.tabs();
    assert_eq!(reload(&changed, &pinned), pinned_before + 1);
    assert_eq!(changed.selected, TabRef::Chat);
    assert_eq!(changed.overlay.as_ref().unwrap().tab, overlay);
    assert_eq!(
        changed
            .overlay
            .as_ref()
            .unwrap()
            .page
            .as_ref()
            .unwrap()
            .reload,
        overlay_before + 1
    );
    assert_eq!(
        rig.app.plugins().installed.unwrap().plugins[0].version,
        version
    );

    refresh_page(&rig, &pinned, plugin, "same-content").await;
    let unchanged = rig.app.tabs();
    assert_eq!(reload(&unchanged, &pinned), pinned_before + 1);
    assert_eq!(
        unchanged
            .overlay
            .as_ref()
            .unwrap()
            .page
            .as_ref()
            .unwrap()
            .reload,
        overlay_before + 1
    );
}

#[tokio::test]
async fn upgrading_then_rollback_with_unchanged_revision_does_not_reload() {
    let (rig, pinned, running) = connected_page_rig("app.rollback", "home").await;
    let before = reload(&rig.app.tabs(), &pinned);
    let mut upgrading = running.clone();
    upgrading.status = PluginState::Upgrading;
    refresh_page(&rig, &pinned, upgrading, "upgrading").await;
    assert_eq!(reload(&rig.app.tabs(), &pinned), before);
    refresh_page(&rig, &pinned, running, "rolled-back").await;
    let rolled_back = rig.app.tabs();
    assert_eq!(
        item(&rolled_back, &pinned).page.as_ref().unwrap().state,
        TabState::Ready
    );
    assert_eq!(reload(&rolled_back, &pinned), before);
}

#[tokio::test]
async fn first_appearance_and_another_servers_revision_do_not_reload() {
    let (rig, pinned, mut plugin) = connected_page_rig("app.first-revision", "home").await;
    assert_eq!(reload(&rig.app.tabs(), &pinned), 0);
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
    wait_tabs(&rig, |view| {
        !view.items.iter().any(|item| item.tab == pinned)
    })
    .await;
    rig.app
        .dispatch(Action::PinTab {
            tab: pinned.clone(),
            replacing: None,
        })
        .unwrap();
    let other_page = wait_tabs(&rig, |view| {
        item(view, &pinned).page.as_ref().unwrap().title == "other-server"
    })
    .await;
    assert_eq!(reload(&other_page, &pinned), 0);
}
