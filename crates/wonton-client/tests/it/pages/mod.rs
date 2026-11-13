//! 页面行为测试：只经由 ClientCore 和 Action 驱动，插件管理接口由本机假核心提供。

mod bar_settings;
mod default_bar;
mod load;
mod media_permission;
mod my_apps;
mod polling;
mod revision;
mod state;
mod stopped;
mod support;

use std::time::{Duration, Instant};

use wonton_client::{
    Action, CoreError, HomeStatus, InstalledPlugin, MyAppRef, OpenPage, PagesViewModel, PanelScope,
    PlacedTab, PluginHealth, PluginImage, PluginState, TabIcon, TabLayout, TabLoadFailure, TabRef,
    TabState,
};

use crate::plugins::rig::{FakeHome, Rig, installed};
use support::{assert_invalid, item, page, tab, web_plugin};

async fn wait_pages(rig: &Rig, mut ready: impl FnMut(&PagesViewModel) -> bool) -> PagesViewModel {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let view = rig.app.pages();
        if ready(&view) {
            return view;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待页面状态超时：{view:?}"
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

/// 打开一个页面并等它的状态就绪；栈里就有了这一页。
pub(super) async fn open_ready(rig: &Rig, page_tab: &TabRef, state: TabState) -> PagesViewModel {
    rig.app
        .dispatch(Action::OpenPage {
            tab: page_tab.clone(),
        })
        .unwrap();
    wait_pages(rig, |view| item(view, page_tab).state == state).await
}

async fn wait_my_app(rig: &Rig, page_tab: &TabRef) {
    let id = page_tab_plugin_id(page_tab);
    rig.wait_for(|view| {
        view.sidebar
            .apps
            .iter()
            .any(|app| app.plugin_id.as_deref() == Some(id))
    })
    .await;
}

fn page_tab_plugin_id(tab: &TabRef) -> &str {
    match tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugin_id,
        _ => unreachable!("tests only pin plugin pages"),
    }
}

#[tokio::test]
async fn pages_subscription_reports_current_changes_and_cancel() {
    let rig = Rig::new().await;
    let subscription = rig.app.subscribe_pages();
    let first = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("页面订阅没有返回首份状态")
        .expect("页面核心已释放");
    assert_eq!(first, rig.app.pages());
    assert!(first.stack.is_empty(), "启动时栈是空的");

    // 打开一个页面：订阅给出带它的新状态。
    let plugin_id = "app.sub";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some("home"));
    rig.app
        .dispatch(Action::OpenPage {
            tab: page_tab.clone(),
        })
        .unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("页面订阅没有返回变更")
        .expect("页面核心已释放");
    assert!(changed.stack.iter().any(|open| open.tab == page_tab));

    subscription.cancel();
    assert!(subscription.next().await.is_none());
}

#[tokio::test]
async fn defaults_root_stack_and_my_apps_invariants() {
    let rig = Rig::new().await;
    rig.connected().await;
    let initial = wait_pages(&rig, |view| view.root == TabRef::Chat).await;
    // 配对后的根页面是聊天；栈里还没有页面。
    assert_eq!(initial.root, TabRef::Chat);
    assert!(initial.stack.is_empty());
    assert!(initial.my_app_candidates.is_empty());
    assert!(initial.auto_add);

    let plugin_id = "app.web";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;

    let plugin_tab = tab(plugin_id, None);
    rig.app
        .dispatch(Action::OpenPage {
            tab: plugin_tab.clone(),
        })
        .unwrap();
    let opened = rig.app.pages();
    assert_eq!(opened.stack.len(), 1);
    assert_eq!(opened.stack[0].tab, plugin_tab);
    let page = &opened.stack[0];
    assert!(matches!(page.page.icon, TabIcon::Plugin { .. }));
    assert_eq!(page.page.layout, TabLayout::Inset);

    // 根页面一路弹回根；关掉最后一页也回到根。
    rig.app
        .dispatch(Action::OpenPage { tab: TabRef::Chat })
        .unwrap();
    assert!(rig.app.pages().stack.is_empty());

    // 打不开的页面拒绝：没装的插件。
    assert_invalid(rig.app.dispatch(Action::OpenPage {
        tab: tab("app.missing", None),
    }));
    // 我的应用不收内置页：聊天和应用是侧边栏的固定行。
    assert_invalid(rig.app.dispatch(Action::AddMyApp {
        app: MyAppRef::Page { tab: TabRef::Chat },
        title: None,
        index: None,
    }));
    assert_invalid(rig.app.dispatch(Action::MoveMyApp {
        app: MyAppRef::Plugin {
            plugin_id: plugin_id.to_owned(),
        },
        index: 4,
    }));
}

#[tokio::test]
async fn my_apps_add_remove_move_rules_and_sidebar_rows() {
    let rig = Rig::new().await;
    let plugin_id = "app.pages";
    let pages = vec![page("one"), page("two"), page("three"), page("four")];
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        pages,
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;

    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    wait_my_app(&rig, &one).await;
    assert_invalid(rig.app.dispatch(Action::AddMyApp {
        app: MyAppRef::Plugin {
            plugin_id: plugin_id.to_owned(),
        },
        title: None,
        index: None,
    }));

    // 加进来的行就是侧边栏的“我的应用”行，开着这个插件的第一页。
    let sidebar = rig.app.plugins().sidebar;
    assert_eq!(sidebar.apps.len(), 1);
    assert_eq!(sidebar.apps[0].plugin_id.as_deref(), Some(plugin_id));
    assert_eq!(
        sidebar.apps[0].open,
        wonton_client::SidebarOpen::Page { tab: one.clone() }
    );

    // 排序和越界。
    rig.app
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page { tab: two.clone() },
            title: Some("页面二".to_owned()),
            index: None,
        })
        .unwrap();
    let titles: Vec<String> = rig
        .app
        .plugins()
        .sidebar
        .apps
        .iter()
        .map(|app| app.title.clone())
        .collect();
    assert_eq!(
        titles,
        vec!["标题 app.pages".to_owned(), "页面二".to_owned()]
    );
    rig.app
        .dispatch(Action::MoveMyApp {
            app: MyAppRef::Page { tab: two.clone() },
            index: 0,
        })
        .unwrap();
    assert_eq!(rig.app.plugins().sidebar.apps[0].tab.as_ref(), Some(&two));
    assert_invalid(rig.app.dispatch(Action::MoveMyApp {
        app: MyAppRef::Page { tab: two.clone() },
        index: 5,
    }));
}

#[tokio::test]
async fn removing_my_app_rows_updates_the_sidebar_and_candidates() {
    let rig = Rig::new().await;
    let plugin_id = "app.pages";
    let pages = vec![page("one"), page("two"), page("three"), page("four")];
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        pages,
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;

    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    let three = tab(plugin_id, Some("three"));
    rig.app
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page { tab: two.clone() },
            title: Some("页面二".to_owned()),
            index: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::MoveMyApp {
            app: MyAppRef::Page { tab: two.clone() },
            index: 0,
        })
        .unwrap();

    // 移除再移除报错。
    rig.app
        .dispatch(Action::RemoveMyApp {
            app: MyAppRef::Page { tab: three.clone() },
        })
        .unwrap_err();

    // 移除插件行：页面行（页面二）还在，插件的页面回到候选里。
    rig.app
        .dispatch(Action::RemoveMyApp {
            app: MyAppRef::Plugin {
                plugin_id: plugin_id.to_owned(),
            },
        })
        .unwrap();
    let after = rig.app.plugins().sidebar;
    assert_eq!(after.apps.len(), 1);
    assert_eq!(after.apps[0].tab.as_ref(), Some(&two));
    assert!(after.more_apps);
    assert!(
        rig.app
            .pages()
            .my_app_candidates
            .iter()
            .any(|candidate| candidate.tab == one)
    );

    // 移除最后一个页面行：全空，全部页面回候选。
    rig.app
        .dispatch(Action::RemoveMyApp {
            app: MyAppRef::Page { tab: two },
        })
        .unwrap();
    assert!(rig.app.plugins().sidebar.apps.is_empty());
    assert!(rig.app.plugins().sidebar.more_apps);
    let candidates = rig.app.pages().my_app_candidates;
    assert!(candidates.iter().any(|candidate| candidate.tab == one));
    assert!(candidates.iter().any(|candidate| candidate.tab == three));
}

async fn connected_page_rig(plugin_id: &str, page_id: &str) -> (Rig, TabRef, InstalledPlugin) {
    let rig = Rig::new().await;
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id)],
    );
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some(page_id));
    wait_my_app(&rig, &page_tab).await;
    open_ready(&rig, &page_tab, TabState::Ready).await;
    // 开页面会补一次列表；等它落地，后面测试设的列表节奏才不被它打乱。
    let before = rig
        .core
        .seen(&axum::http::Method::GET, "/_wonton/api/plugins")
        .len();
    let deadline = Instant::now() + Duration::from_secs(5);
    while rig
        .core
        .seen(&axum::http::Method::GET, "/_wonton/api/plugins")
        .len()
        == before
    {
        assert!(Instant::now() < deadline, "开页面的补取没有发出");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
            vec![page(page_id)],
        )]]);
        rig.app.dispatch(Action::RefreshPlugins).unwrap();
        let view = wait_pages(&rig, |view| item(view, &page_tab).state == expected).await;
        let tab_page = item(&view, &page_tab);
        assert_eq!(tab_page.state, expected);
        // 侧边栏行的红点跟着插件状态走：不在运行就不亮“出问题”以外的升级点。
        let badge = rig
            .app
            .plugins()
            .sidebar
            .apps
            .first()
            .map(|app| app.badge.problem)
            .unwrap_or(false);
        assert_eq!(badge, matches!(expected, TabState::Failed { .. }));
    }

    // 页面被新版本拿掉：状态页如实说，红点亮。
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    )]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_pages(&rig, |view| {
        item(view, &page_tab).state == TabState::PageRemoved
    })
    .await;
    assert!(
        rig.app
            .plugins()
            .sidebar
            .apps
            .first()
            .is_some_and(|app| app.badge.problem)
    );

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_pages(&rig, |view| item(view, &page_tab).state == TabState::Ready).await;
}

#[tokio::test]
async fn corrupted_config_recovers_and_restart_starts_with_an_empty_stack() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.persist";
    let page_id = "home";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some(page_id));
    wait_my_app(&rig, &page_tab).await;
    open_ready(&rig, &page_tab, TabState::Ready).await;

    // 打开着的页面是运行时状态：重启后栈是空的，我的应用还在。
    rig.reopen();
    let restarted = rig.app.pages();
    assert!(restarted.stack.is_empty());
    assert_eq!(restarted.root, TabRef::Chat);
    assert_eq!(restarted.problem, None);

    std::fs::write(rig.data_dir().join("layout.json"), b"not json").unwrap();
    rig.reopen();
    let broken = rig.app.pages();
    assert!(broken.problem.is_some());
    assert!(!broken.my_app_candidates.is_empty());

    // 下一次成功的配置改动重写文件，原因消失。
    rig.app
        .dispatch(Action::SetAutoAddApps { enabled: false })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
    rig.reopen();
    assert!(rig.app.pages().problem.is_none());
    assert!(!rig.app.pages().auto_add);
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
        vec![page(page_id)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some(page_id));
    wait_my_app(&rig, &page_tab).await;
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    // 列表还没读到：我的应用行按缓存的标题和图标显示，红点不乱亮。
    let sidebar = rig.app.plugins().sidebar;
    assert_eq!(sidebar.apps.len(), 1);
    assert_eq!(sidebar.apps[0].title, "标题 app.cold");

    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_pages(&rig, |view| view.uninstalled == ["标题 app.cold"]).await;
    assert!(removed.my_app_candidates.is_empty());
    assert_eq!(removed.uninstalled, vec!["标题 app.cold"]);
    rig.app.dispatch(Action::DismissUninstalledTabs).unwrap();
    assert!(rig.app.pages().uninstalled.is_empty());
}

#[tokio::test]
async fn opening_a_page_is_not_a_selection_and_server_switch_clears_plugin_rows() {
    let rig = Rig::new().await;
    let plugin_id = "app.switch";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("full")],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some("full"));
    wait_my_app(&rig, &page_tab).await;
    open_ready(&rig, &page_tab, TabState::Ready).await;
    assert_eq!(rig.app.pages().stack.len(), 1);

    // 换到另一台服务器：绑在插件上的行跟着走，栈清空，回到根。
    let other = FakeHome::start(vec![rig.device()]).await;
    other.core.set_lists(vec![Vec::new()]);
    rig.connect_to(other.ticket());
    let switched = wait_pages(&rig, |view| {
        view.stack.is_empty()
            && view.root == TabRef::Chat
            && rig.app.plugins().sidebar.apps.is_empty()
    })
    .await;
    assert!(switched.uninstalled.is_empty());
    assert_eq!(switched.root, TabRef::Chat);
}

#[tokio::test]
async fn layout_follows_the_manifest_and_unknown_values_use_inset() {
    let rig = Rig::new().await;
    let plugin_id = "app.layout";
    let page_id = "edge";
    let mut declared = page(page_id);
    declared.layout = TabLayout::EdgeToEdge;
    let mut unknown = page("unknown");
    unknown.layout = TabLayout::Other;
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![declared, unknown],
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;

    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::OpenPage {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert_eq!(
        item(&rig.app.pages(), &page_tab).url.as_deref(),
        Some(&*format!("http://{plugin_id}.wonton.localhost/{page_id}"))
    );
    assert_eq!(
        item(&rig.app.pages(), &page_tab).layout,
        TabLayout::EdgeToEdge
    );

    let unknown_tab = tab(plugin_id, Some("unknown"));
    rig.app
        .dispatch(Action::OpenPage {
            tab: unknown_tab.clone(),
        })
        .unwrap();
    assert_eq!(
        item(&rig.app.pages(), &unknown_tab).layout,
        TabLayout::Inset
    );
}

#[tokio::test]
async fn closing_pages_pops_in_order_and_reopening_is_the_same_instance() {
    let (rig, page_tab, _) = connected_page_rig("app.stack", "home").await;
    let other = TabRef::PluginPath {
        plugin_id: "app.stack".to_owned(),
        path: "/other".to_owned(),
    };
    rig.app
        .dispatch(Action::OpenPage { tab: other.clone() })
        .unwrap();
    let stack: Vec<TabRef> = rig
        .app
        .pages()
        .stack
        .iter()
        .map(|open: &OpenPage| open.tab.clone())
        .collect();
    assert_eq!(stack, vec![page_tab.clone(), other.clone()]);

    rig.app.dispatch(Action::ClosePage).unwrap();
    assert_eq!(rig.app.pages().stack.len(), 1);
    assert_eq!(rig.app.pages().stack[0].tab, page_tab);

    // 再打开同一页：还是栈里那一份，不新建。
    rig.app.dispatch(Action::OpenPage { tab: other }).unwrap();
    assert_eq!(rig.app.pages().stack.len(), 2);
}
