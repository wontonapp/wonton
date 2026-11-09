//! 底栏行为测试：只经由 ClientCore 和 Action 驱动，插件管理接口由本机假核心提供。

mod bar_settings;
mod default_bar;
mod load;
mod polling;
mod revision;
mod state;
mod stopped;
mod support;

use std::time::{Duration, Instant};

use wonton_client::{
    Action, CoreError, HomeStatus, InstalledPlugin, PlacedTab, PluginHealth, PluginImage,
    PluginState, PluginTab, TabDisplay, TabDisplayChoice, TabIcon, TabLayout, TabLoadFailure,
    TabRef, TabState, TabsViewModel,
};

use crate::plugins::rig::{FakeHome, Rig, installed};
use support::{assert_invalid, item, page, tab, web_plugin};

async fn wait_tabs(rig: &Rig, mut ready: impl FnMut(&TabsViewModel) -> bool) -> TabsViewModel {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let view = rig.app.tabs();
        if ready(&view) {
            return view;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待底栏状态超时：{view:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_plugin_status(rig: &Rig, id: &str, status: PluginState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let view = rig.app.plugins();
        if view.installed.as_ref().is_some_and(|installed| {
            installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == id)
                .is_some_and(|plugin| plugin.status == status)
        }) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待插件 {id} 状态 {status:?} 超时：{view:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn tabs_subscription_reports_current_changes_and_cancel() {
    let rig = Rig::new().await;
    let subscription = rig.app.subscribe_tabs();
    let first = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("底栏订阅没有返回首份状态")
        .expect("底栏核心已释放");
    assert_eq!(first, rig.app.tabs());

    rig.app
        .dispatch(Action::UnpinTab {
            tab: TabRef::Settings,
        })
        .unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("底栏订阅没有返回变更")
        .expect("底栏核心已释放");
    assert!(
        !changed
            .items
            .iter()
            .any(|item| item.tab == TabRef::Settings)
    );

    subscription.cancel();
    assert!(subscription.next().await.is_none());
}

#[tokio::test]
async fn defaults_selection_overlay_and_builtin_invariants() {
    let rig = Rig::new().await;
    let initial = rig.app.tabs();
    assert_eq!(
        initial
            .items
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![TabRef::Chat, TabRef::Apps, TabRef::Settings]
    );
    assert_eq!(initial.selected, TabRef::Apps);
    assert!(!initial.full);

    let plugin_id = "app.web";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    )]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        view.overlay.is_none() && rig.app.plugins().installed.is_some()
    })
    .await;

    let plugin_tab = tab(plugin_id, None);
    rig.app
        .dispatch(Action::SelectTab {
            tab: plugin_tab.clone(),
        })
        .unwrap();
    let overlay = rig.app.tabs();
    assert_eq!(overlay.selected, TabRef::Chat);
    assert_eq!(
        overlay.overlay.as_ref().map(|item| &item.tab),
        Some(&plugin_tab)
    );
    let page = overlay.overlay.unwrap().page.unwrap();
    assert!(matches!(page.icon, TabIcon::Plugin { .. }));
    assert_eq!(page.layout, TabLayout::Inset);
    assert_eq!(page.display, TabDisplayChoice::Default);

    assert_invalid(rig.app.dispatch(Action::UnpinTab { tab: TabRef::Chat }));
    assert_invalid(rig.app.dispatch(Action::UnpinTab { tab: TabRef::Apps }));
    // 配好之后设置不再默认在栏上：拿不掉（不在栏上），可以从候选固定。
    assert_invalid(rig.app.dispatch(Action::UnpinTab {
        tab: TabRef::Settings,
    }));
    assert_invalid(rig.app.dispatch(Action::SelectTab {
        tab: TabRef::Settings,
    }));
    rig.app
        .dispatch(Action::PinTab {
            tab: TabRef::Settings,
            replacing: None,
        })
        .unwrap();
    assert_invalid(rig.app.dispatch(Action::PinTab {
        tab: TabRef::Settings,
        replacing: None,
    }));
    rig.app
        .dispatch(Action::PinTab {
            tab: plugin_tab.clone(),
            replacing: None,
        })
        .unwrap();
    assert_invalid(rig.app.dispatch(Action::PinTab {
        tab: plugin_tab,
        replacing: None,
    }));
    assert_invalid(rig.app.dispatch(Action::MoveTab {
        tab: TabRef::Chat,
        index: 4,
    }));
}

#[tokio::test]
async fn default_display_follows_manifest_and_unknown_values_use_standard() {
    let rig = Rig::new().await;
    let plugin_id = "app.display";
    let page_id = "display";
    let mut declared = page(page_id, TabDisplay::Fullscreen);
    declared.path = "/rss/feed".to_owned();
    let mut unknown = page("unknown", TabDisplay::Other);
    unknown.layout = TabLayout::Other;
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![declared, unknown],
    )]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    let default = item(&rig.app.tabs(), &page_tab)
        .page
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(
        default.url.as_deref(),
        Some("http://app.display.wonton.localhost/rss/feed")
    );
    assert_eq!(default.display, TabDisplayChoice::Default);
    assert_eq!(default.layout, TabLayout::Inset);
    assert!(default.opens_fullscreen);
    assert!(!default.fullscreen);

    rig.app
        .dispatch(Action::SetTabDisplay {
            tab: page_tab.clone(),
            display: TabDisplayChoice::Standard,
        })
        .unwrap();
    let standard_view = rig.app.tabs();
    let standard = item(&standard_view, &page_tab).page.as_ref().unwrap();
    assert!(!standard.opens_fullscreen);
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert!(
        !item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );
    let unknown_tab = tab(plugin_id, Some("unknown"));
    rig.app
        .dispatch(Action::PinTab {
            tab: unknown_tab.clone(),
            replacing: None,
        })
        .unwrap();
    let unknown_view = rig.app.tabs();
    let unknown_page = item(&unknown_view, &unknown_tab).page.as_ref().unwrap();
    assert_eq!(unknown_page.layout, TabLayout::Inset);
    assert!(!unknown_page.opens_fullscreen);
}

#[tokio::test]
async fn pin_limit_replacement_move_and_selection_rules() {
    let rig = Rig::new().await;
    let plugin_id = "app.pages";
    let pages = vec![
        page("one", TabDisplay::Standard),
        page("two", TabDisplay::Standard),
        page("three", TabDisplay::Standard),
        page("four", TabDisplay::Standard),
    ];
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        pages,
    )]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.overlay.is_none()
    })
    .await;

    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    let three = tab(plugin_id, Some("three"));
    let four = tab(plugin_id, Some("four"));
    // 默认栏上只有聊天：四个页面全固定才满。
    for page in [&one, &two, &three, &four] {
        rig.app
            .dispatch(Action::PinTab {
                tab: page.clone(),
                replacing: None,
            })
            .unwrap();
    }
    assert!(rig.app.tabs().full);
    assert_invalid(rig.app.dispatch(Action::PinTab {
        tab: TabRef::Settings,
        replacing: None,
    }));
    assert_invalid(rig.app.dispatch(Action::PinTab {
        tab: TabRef::Settings,
        replacing: Some(TabRef::Chat),
    }));
    rig.app
        .dispatch(Action::PinTab {
            tab: TabRef::Settings,
            replacing: Some(one.clone()),
        })
        .unwrap();
    let full = rig.app.tabs();
    assert!(full.full);
    assert!(!full.items.iter().any(|item| item.tab == one));
    assert!(full.items.iter().any(|item| item.tab == TabRef::Settings));

    rig.app
        .dispatch(Action::SelectTab { tab: two.clone() })
        .unwrap();
    assert_eq!(rig.app.tabs().selected, two);
    assert!(rig.app.tabs().overlay.is_none());
    assert_invalid(rig.app.dispatch(Action::MoveTab {
        tab: two.clone(),
        index: 5,
    }));
    assert_invalid(rig.app.dispatch(Action::PinTab {
        tab: two.clone(),
        replacing: None,
    }));
}

async fn connected_page_rig(plugin_id: &str, page_id: &str) -> (Rig, TabRef, InstalledPlugin) {
    let rig = Rig::new().await;
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    (rig, page_tab, plugin)
}

#[tokio::test]
async fn plugin_states_and_page_removal_map_to_tabs() {
    let (rig, page_tab, plugin) = connected_page_rig("app.states", "state").await;
    let plugin_id = "app.states";
    let page_id = "state";
    let states = [
        (
            PluginState::Running {
                health: PluginHealth::Unknown,
            },
            TabState::Ready,
        ),
        (PluginState::Installing, TabState::Starting),
        (PluginState::Upgrading, TabState::Upgrading),
        (
            PluginState::Degraded {
                reason: "健康检查失败".to_owned(),
            },
            TabState::Failed {
                reason: "健康检查失败".to_owned(),
            },
        ),
        (PluginState::Uninstalling, TabState::Uninstalling),
        (PluginState::Unknown, TabState::Unknown),
    ];
    for (status, expected) in states {
        rig.core.set_lists(vec![vec![web_plugin(
            plugin_id,
            status,
            vec![page(page_id, TabDisplay::Standard)],
        )]]);
        rig.app.dispatch(Action::RefreshPlugins).unwrap();
        let view = wait_tabs(&rig, |view| {
            item(view, &page_tab).page.as_ref().unwrap().state == expected
        })
        .await;
        let tab_page = item(&view, &page_tab).page.as_ref().unwrap();
        assert_eq!(tab_page.state, expected);
        let badge = item(&view, &page_tab).badge;
        assert_eq!(badge, matches!(expected, TabState::Failed { .. }));
    }

    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    )]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::PageRemoved
    })
    .await;
    assert!(item(&removed, &page_tab).badge);

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
    })
    .await;
}

#[tokio::test]
async fn corrupted_config_recovers_and_restart_does_not_restore_runtime_selection() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.persist";
    let page_id = "home";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SetTabDisplay {
            tab: page_tab.clone(),
            display: TabDisplayChoice::Fullscreen,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );

    rig.reopen();
    let restarted = wait_tabs(&rig, |view| {
        view.items.iter().any(|item| item.tab == page_tab) && view.selected == TabRef::Chat
    })
    .await;
    let persisted = item(&restarted, &page_tab).page.as_ref().unwrap();
    assert_eq!(persisted.display, TabDisplayChoice::Fullscreen);
    assert!(!persisted.fullscreen);

    std::fs::write(rig.data_dir().join("tabs.json"), b"not json").unwrap();
    rig.reopen();
    let broken = rig.app.tabs();
    assert!(broken.problem.is_some());
    assert_eq!(
        broken
            .items
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![TabRef::Chat]
    );
    rig.app
        .dispatch(Action::MoveTab {
            tab: TabRef::Chat,
            index: 0,
        })
        .unwrap();
    assert!(rig.app.tabs().problem.is_none());
    rig.reopen();
    assert!(rig.app.tabs().problem.is_none());
}

#[tokio::test]
async fn cold_start_loading_keeps_cached_plugin_until_a_real_list_arrives() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.cold";
    let page_id = "cold";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    let loading = rig.app.tabs();
    assert_eq!(
        item(&loading, &page_tab).page.as_ref().unwrap().state,
        TabState::Loading
    );
    assert!(loading.uninstalled.is_empty());

    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_tabs(&rig, |view| {
        !view.items.iter().any(|item| item.tab == page_tab) && view.uninstalled == ["标题 app.cold"]
    })
    .await;
    assert_eq!(removed.selected, TabRef::Chat);
    assert_eq!(removed.uninstalled, vec!["标题 app.cold"]);
    rig.app.dispatch(Action::DismissUninstalledTabs).unwrap();
    assert!(rig.app.tabs().uninstalled.is_empty());
}

#[tokio::test]
async fn fullscreen_exit_has_manual_and_automatic_paths_and_server_switch_clears_plugins() {
    let rig = Rig::new().await;
    let plugin_id = "app.fullscreen";
    let page_id = "full";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();

    rig.app
        .dispatch(Action::EnterFullscreen {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );
    rig.app
        .dispatch(Action::ExitFullscreen {
            tab: page_tab.clone(),
        })
        .unwrap();
    let manual_exit = rig.app.tabs();
    assert_eq!(manual_exit.selected, page_tab);
    assert!(
        !item(&manual_exit, &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );

    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    rig.app
        .dispatch(Action::SetTabDisplay {
            tab: page_tab.clone(),
            display: TabDisplayChoice::Fullscreen,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );
    rig.app
        .dispatch(Action::ExitFullscreen {
            tab: page_tab.clone(),
        })
        .unwrap();
    let automatic_exit = rig.app.tabs();
    assert_eq!(automatic_exit.selected, TabRef::Chat);
    assert!(
        !item(&automatic_exit, &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );

    rig.connect_to(rig.home_ticket.clone());
    assert!(rig.app.tabs().items.iter().any(|item| item.tab == page_tab));

    let other = FakeHome::start(vec![rig.device()]).await;
    other.core.set_lists(vec![Vec::new()]);
    rig.connect_to(other.ticket());
    let switched = wait_tabs(&rig, |view| {
        !view.items.iter().any(|item| item.tab == page_tab)
            && view.selected == TabRef::Chat
            && view.uninstalled.is_empty()
    })
    .await;
    assert_eq!(switched.items.len(), 1);
}
