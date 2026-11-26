use super::*;

#[tokio::test]
async fn page_urls_keep_web_entry_and_resolve_declared_paths_from_origin() {
    let rig = Rig::new().await;
    let plugin_id = "app.url-origin";
    let mut declared = page("feed");
    declared.path = "/feed".to_owned();
    let mut plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![declared],
    );
    plugin.web_url = Some("https://x/app".to_owned());
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let web_tab = tab(plugin_id, None);
    let page_tab = tab(plugin_id, Some("feed"));
    for open in [&web_tab, &page_tab] {
        rig.app
            .dispatch(Action::Pages {
                action: wonton_protocol::PageAction::OpenPage { tab: open.clone() },
            })
            .unwrap();
    }
    assert_eq!(
        item(&rig.app.pages(), &web_tab).url.as_deref(),
        Some("https://x/app")
    );
    assert_eq!(
        item(&rig.app.pages(), &page_tab).url.as_deref(),
        Some("https://x/feed")
    );
}
