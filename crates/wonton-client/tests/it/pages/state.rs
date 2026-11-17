use axum::http::Method;
use wonton_client::{BOT_PANEL_PINS_PATH, PagesFailure};

use crate::plugins::rig::{Canned, Headered};

use super::*;

#[tokio::test]
async fn a_storage_error_leaves_my_apps_unchanged_and_recovers() {
    let rig = Rig::new().await;
    let before = rig.app.pages();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    let result = rig.app.dispatch(Action::Pages {
        action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
    });
    assert!(
        matches!(result, Err(CoreError::Storage { .. })),
        "{result:?}"
    );
    let after = rig.app.pages();
    assert_eq!(after.my_app_candidates, before.my_app_candidates);
    assert_eq!(after.auto_add, before.auto_add);
    assert_eq!(
        after.problem.as_ref().and_then(|problem| problem.reason),
        Some(PagesFailure::StorageWrite)
    );
    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: true },
        })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
}

#[tokio::test]
async fn uninstall_persist_error_is_visible_and_recovers() {
    let rig = Rig::new().await;
    let plugin_id = "app.persist-error";
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
    wait_my_app(&rig, &page_tab).await;
    std::fs::remove_file(rig.data_dir().join("layout.json")).unwrap();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let removed = wait_pages(&rig, |view| {
        view.uninstalled == ["标题 app.persist-error"] && view.problem.is_some()
    })
    .await;
    assert!(!removed.problem.unwrap().message.is_empty());

    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: true },
        })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
}

#[tokio::test]
async fn page_actions_before_the_list_are_not_ready() {
    let rig = Rig::new().await;
    let page = tab("app.missing", Some("home"));
    let actions = [
        Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: page.clone() },
        },
        Action::Pages {
            action: wonton_protocol::PageAction::AddMyApp {
                app: MyAppRef::Plugin {
                    plugin_id: "app.missing".to_owned(),
                },
                title: None,
                index: None,
            },
        },
    ];
    for action in actions {
        assert!(
            matches!(rig.app.dispatch(action), Err(CoreError::NotReady { .. })),
            "列表未读到时应返回 NotReady"
        );
    }
}

#[tokio::test]
async fn uninstall_notice_uses_plugin_title_once_and_ignores_open_pages() {
    let rig = Rig::new().await;
    let plugin_id = "app.uninstalled";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("one"), page("two")],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    // 两个页面行加进我的应用，打开其中一个。
    for page in [&one, &two] {
        rig.app
            .dispatch(Action::Pages {
                action: wonton_protocol::PageAction::AddMyApp {
                    app: MyAppRef::Page { tab: page.clone() },
                    title: None,
                    index: None,
                },
            })
            .unwrap();
    }
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: one.clone() },
        })
        .unwrap();

    rig.core.set_lists(vec![Vec::new()]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let removed = wait_pages(&rig, |view| {
        view.uninstalled == ["标题 app.uninstalled"]
            && !view
                .stack
                .iter()
                .any(|open| open.tab == one || open.tab == two)
    })
    .await;
    assert_eq!(removed.uninstalled, vec!["标题 app.uninstalled"]);

    // 打开着的页面跟着插件走：栈清空，不额外提示。
    let other_rig = Rig::new().await;
    other_rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("one")],
    )]]);
    other_rig.connected().await;
    wait_pages(&other_rig, |_| other_rig.app.plugins().installed.is_some()).await;
    other_rig
        .app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: one.clone() },
        })
        .unwrap();
    other_rig.core.set_lists(vec![Vec::new()]);
    other_rig
        .app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let opened_removed = wait_pages(&other_rig, |view| {
        view.stack.is_empty() && view.root == TabRef::Chat
    })
    .await;
    // 打开着的页面不额外提示：还是卸载时的那一条，关掉就没了。
    assert_eq!(opened_removed.uninstalled, vec!["标题 app.uninstalled"]);
    other_rig
        .app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(other_rig.app.pages().uninstalled.is_empty());
}

#[tokio::test]
async fn content_change_reload_clears_a_previous_load_failure() {
    let (rig, page_tab, mut running_plugin) =
        connected_page_rig("app.upgrade-reload", "home").await;
    // 后台化停掉 10 秒一轮的页面轮询：列表节奏由测试控制，重载只数这一次。
    rig.app
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::TabLoadFailed {
                page: PlacedTab::Page {
                    tab: page_tab.clone(),
                },
                status: Some(502),
            },
        })
        .unwrap();
    let before = item(&rig.app.pages(), &page_tab).reload;
    running_plugin.revision = "changed-content".to_owned();
    rig.core.set_lists(vec![vec![running_plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let recovered = wait_pages(&rig, |view| {
        let page = item(view, &page_tab);
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(item(&recovered, &page_tab).state, TabState::Ready);
}

#[tokio::test]
async fn reconnect_reload_clears_a_previous_load_failure() {
    let rig = Rig::new().await;
    let plugin_id = "app.reconnect-reload";
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
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::TabLoadFailed {
                page: PlacedTab::Page {
                    tab: page_tab.clone(),
                },
                status: Some(502),
            },
        })
        .unwrap();
    let before = item(&rig.app.pages(), &page_tab).reload;
    rig.connect_to(rig.unreachable_ticket());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(rig.app.home().status, HomeStatus::Disconnected { .. }) {
        assert!(
            Instant::now() < deadline,
            "未进入断开状态：{:?}",
            rig.app.home()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    rig.connect_to(rig.home_ticket.clone());
    let recovered = wait_pages(&rig, |view| {
        let page = item(view, &page_tab);
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(item(&recovered, &page_tab).state, TabState::Ready);
}

#[tokio::test]
async fn open_pages_report_loading_and_removal_honestly() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.fixed-state";
    let page_id = "home";
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

    // 列表还没读到：页面如实说在加载。
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage {
                tab: page_tab.clone(),
            },
        })
        .unwrap();
    assert_eq!(item(&rig.app.pages(), &page_tab).state, TabState::Loading);

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_pages(&rig, |view| item(view, &page_tab).state == TabState::Ready).await;
    // 新版本不再提供这个页面：状态页照实说，入口还在。
    let mut removed = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    );
    removed.web_url = Some("https://x/app".to_owned());
    rig.core.set_lists(vec![vec![removed]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_pages(&rig, |view| {
        item(view, &page_tab).state == TabState::PageRemoved
    })
    .await;
}

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

fn pin_response(id: &str) -> String {
    serde_json::json!({
        "pins": [{
            "id": id,
            "scope": {"type": "main"},
            "tab": {"type": "url", "url": format!("https://example.com/{id}")},
            "title": id,
            "attention": false,
            "plugin_id": "app.example.agent",
            "bot": "agent",
            "name": "Agent"
        }],
        "consents": []
    })
    .to_string()
}

#[tokio::test]
async fn page_problem_pin_read_preserves_pages_and_recovers() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.core.answer_always(
        Method::GET,
        BOT_PANEL_PINS_PATH,
        Canned(200, pin_response("first")),
    );
    rig.connected().await;
    let first_tab = TabRef::Url {
        url: "https://example.com/first".into(),
    };
    let first = wait_pages(&rig, |view| {
        view.panel.iter().any(|page| page.tab == first_tab)
    })
    .await;
    for (status, body, expected) in [
        (200, "{\"pins\":[]}", PagesFailure::InvalidResponse),
        (502, "", PagesFailure::Upstream),
        (
            403,
            "{\"error\":{\"kind\":\"permission_denied\",\"message\":\"denied\"}}",
            PagesFailure::Forbidden,
        ),
        (503, "", PagesFailure::Upstream),
    ] {
        rig.core.answer_always(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            Canned(status, body.into()),
        );
        let failed = wait_pages(&rig, |view| {
            view.problem.as_ref().and_then(|problem| problem.reason) == Some(expected)
        })
        .await;
        assert_eq!(
            failed
                .panel
                .iter()
                .map(|page| &page.tab)
                .collect::<Vec<_>>(),
            first.panel.iter().map(|page| &page.tab).collect::<Vec<_>>()
        );
    }
    rig.core.answer_always(
        Method::GET,
        BOT_PANEL_PINS_PATH,
        Canned(200, pin_response("recovered")),
    );
    let recovered_tab = TabRef::Url {
        url: "https://example.com/recovered".into(),
    };
    let recovered = wait_pages(&rig, |view| {
        view.problem.is_none() && view.panel.iter().any(|page| page.tab == recovered_tab)
    })
    .await;
    assert!(!recovered.panel.iter().any(|page| page.tab == first_tab));
}

#[tokio::test]
async fn page_problem_reloads_an_externally_repaired_local_file() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::fs::write(&layout, b"{\"server_id\":null}").unwrap();
    rig.reopen();
    let failed = rig.app.pages();
    assert_eq!(
        failed.problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    let result = rig.app.dispatch(Action::Pages {
        action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
    });
    assert!(matches!(result, Err(CoreError::Storage { .. })));
    assert_eq!(std::fs::read(&layout).unwrap(), b"{\"server_id\":null}");
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    let fixed = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &fixed,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(fixed, layout).unwrap();
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

/// 配置坏着的时候对账跳过了账号数据写回（sync.rs 的 reconcile）；文件修好后，第一个普通页面操作
/// （走 runtime_change）就要把这份配置补上：dirty 置起来，会话来了先推本机的，别的设备的
/// 默认值盖不掉修好的文件。
#[tokio::test]
async fn page_problem_recovery_marks_the_layout_for_push_back() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::fs::write(&layout, b"{\"server_id\":null}").unwrap();
    rig.reopen();
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    assert!(!rig.app.test_pages_layout_dirty());

    let fixed = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &fixed,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(fixed, layout).unwrap();

    // 第一个普通页面操作（runtime_change）就要接住恢复：补 dirty，别等 5 秒的 recompute 轮询。
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(rig.app.test_pages_layout_dirty());
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

/// 没有恢复时，普通操作和刷新不得置 dirty：会话就绪前置了它，reconcile 会推本机的
/// 默认值，盖掉另一台设备已有的配置。
#[tokio::test]
async fn page_problem_normal_actions_do_not_mark_the_layout() {
    let rig = Rig::new().await;
    assert!(!rig.app.test_pages_layout_dirty());
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(!rig.app.test_pages_layout_dirty());
}

/// 自动加入落盘后账号数据写失败（Tuwunel 拒掉 layout 账号数据请求）：就绪分支推送前置了
/// dirty（push_layout 失败不清它），重连对账推的是本机合并后的配置，服务器旧配置盖不掉新
/// 入口。要真实 Matrix 会话，所以标了 ignore；本机经 cargo-slot 只编不跑，Docker 场景留给 CI。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_failed_layout_push_keeps_the_local_entry_on_reconciliation() {
    use crate::chat::support::{Home, Phone};

    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;

    let plugin_id = "app.push-failure";
    let mut plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    );
    plugin.web_url = Some(format!("http://{plugin_id}.wonton.localhost/"));

    let other = home.matrix.login("owner", "Other device").await;
    other
        .update_layout(|raw| {
            raw.insert(
                "my_apps".to_owned(),
                serde_json::json!([{
                    "app": {"type": "plugin", "plugin_id": plugin_id},
                    "title": ""
                }]),
            );
        })
        .await
        .unwrap();
    // 旧入口引用的插件也在已装列表里（另一台设备装的），先喂给它建立 previous_plugins
    // 快照：换成不在列表里的入口，对账后的首次刷新会把它当已卸载删掉，场景就走了样。
    home.set_installed_plugins(vec![plugin.clone()]);

    phone.connect(&home);
    phone.signed_in().await;
    // 首次对账收下服务器旧配置并落盘；等落盘出现旧入口，reconcile（layout_ready）才算完，
    // 后面自动加入的推送才一定走就绪分支。
    let layout_path = dir.path().join("servers/1/layout.json");
    tokio::time::timeout(crate::plugins::rig::WAIT, async {
        loop {
            let Ok(content) = std::fs::read_to_string(&layout_path) else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if content.contains(plugin_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("对账没有收下服务器的旧配置");

    // 让 Tuwunel 整体断开：update_layout 要先读服务器再写，两步都断，推送必失败。
    // set_layout_down 只拦 app.wonton.chat_layout，拦不到这里的 app.wonton.layout。
    home.set_matrix_down(true);
    phone
        .wait_chat("同步因断线停下", |view| {
            matches!(view.sync, wonton_client::ChatSync::Failed { .. }).then_some(())
        })
        .await;

    // 新装的插件到达，自动加入 append 并落盘；就绪分支置 dirty 后推送，写失败不清它。
    let mut new_plugin = web_plugin(
        "app.push-failure-new",
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    );
    new_plugin.web_url = Some("http://app.push-failure-new.wonton.localhost/".to_owned());
    home.set_installed_plugins(vec![plugin, new_plugin]);
    tokio::time::timeout(crate::plugins::rig::WAIT, async {
        while !phone.test_pages_layout_dirty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("自动加入后没有置 dirty");

    // Matrix 回来触发重连对账：dirty=true 时推本机合并后的配置，不收服务器的旧配置。
    home.set_matrix_down(false);

    let deadline = tokio::time::Instant::now() + crate::plugins::rig::WAIT;
    loop {
        let server = other.server_layout().await.unwrap();
        let kept = server.as_ref().is_some_and(|content| {
            let apps = content.0.get("my_apps").and_then(|apps| apps.as_array());
            apps.is_some_and(|apps| {
                apps.iter()
                    .any(|entry| entry["app"]["plugin_id"] == plugin_id)
            }) && apps.is_some_and(|apps| {
                apps.iter()
                    .any(|entry| entry["app"]["plugin_id"] == "app.push-failure-new")
            })
        });
        if kept {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "对账没有把本机新入口推上去：{server:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn page_problem_rechecks_a_temporarily_unreadable_local_file() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::os::unix::fs::symlink("layout.json", &layout).unwrap();
    rig.reopen();
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::StorageRead)
    );
    let repaired = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &repaired,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(repaired, layout).unwrap();
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

#[tokio::test]
async fn page_problem_pin_consent_failure_clears_when_the_result_is_observed() {
    let rig = Rig::new().await.without_periodic_checks();
    let plugin = "app.example.agent";
    let bot = "agent";
    let path = wonton_client::bot_panel_consent_path(plugin, bot);
    let list = serde_json::json!({"pins": [], "consents": [{
        "plugin_id": plugin, "bot": bot, "name": "Agent", "main_consent": true
    }]})
    .to_string();
    rig.core
        .answer_always(Method::GET, BOT_PANEL_PINS_PATH, Canned(200, list));
    rig.core
        .answer_always(Method::PUT, &path, Canned(503, "".into()));
    rig.connected().await;
    wait_pages(&rig, |view| {
        view.bot_pin_consents.iter().any(|entry| entry.main_consent)
    })
    .await;
    let action = || Action::Pages {
        action: wonton_protocol::PageAction::SetBotPinConsent {
            plugin_id: plugin.into(),
            bot: bot.into(),
            allowed: false,
        },
    };
    rig.app.dispatch(action()).unwrap();
    let failed = wait_pages(&rig, |view| {
        view.problem.as_ref().and_then(|problem| problem.reason) == Some(PagesFailure::Upstream)
    })
    .await;
    assert!(failed.bot_pin_consents[0].main_consent);
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
        })
        .unwrap();
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::Upstream)
    );
    let recovered_list = serde_json::json!({"pins": [], "consents": [{
        "plugin_id": plugin, "bot": bot, "name": "Agent", "main_consent": false
    }]})
    .to_string();
    rig.core.answer_always(
        Method::GET,
        BOT_PANEL_PINS_PATH,
        Canned(200, recovered_list),
    );
    let recovered = wait_pages(&rig, |view| {
        view.problem.is_none()
            && view
                .bot_pin_consents
                .iter()
                .any(|entry| !entry.main_consent)
    })
    .await;
    assert!(!recovered.bot_pin_consents[0].main_consent);

    let active_list = serde_json::json!({"pins": [], "consents": [{
        "plugin_id": plugin, "bot": bot, "name": "Agent", "main_consent": true
    }]})
    .to_string();
    rig.core
        .answer_always(Method::GET, BOT_PANEL_PINS_PATH, Canned(200, active_list));
    wait_pages(&rig, |view| {
        view.bot_pin_consents.iter().any(|entry| entry.main_consent)
    })
    .await;
    rig.app.dispatch(action()).unwrap();
    wait_pages(&rig, |view| {
        view.problem.as_ref().and_then(|problem| problem.reason) == Some(PagesFailure::Upstream)
    })
    .await;
    rig.core.answer_always(
        Method::GET,
        BOT_PANEL_PINS_PATH,
        Canned(200, r#"{"pins":[],"consents":[]}"#.into()),
    );
    let recovered = wait_pages(&rig, |view| {
        view.problem.is_none() && view.bot_pin_consents.is_empty()
    })
    .await;
    assert!(recovered.bot_pin_consents.is_empty());
}

#[tokio::test]
async fn page_problem_uses_the_tunnel_error_category_and_recovers() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    for (code, expected) in [
        ("upstream-unavailable", PagesFailure::Upstream),
        ("unauthorized", PagesFailure::Unauthorized),
        ("transport-unavailable", PagesFailure::TransportUnavailable),
        ("protocol", PagesFailure::Protocol),
    ] {
        rig.core.answer_always_headered(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            Headered(
                502,
                vec![("x-wonton-tunnel-error".into(), code.into())],
                String::new(),
            ),
        );
        wait_pages(&rig, |view| {
            view.problem.as_ref().and_then(|problem| problem.reason) == Some(expected)
        })
        .await;
    }
    for (status, kind, expected) in [
        (403, "cross_site", PagesFailure::Forbidden),
        (401, "unauthenticated", PagesFailure::Unauthorized),
        (400, "invalid_input", PagesFailure::InvalidInput),
        (404, "not_found", PagesFailure::NotFound),
        (500, "storage", PagesFailure::Upstream),
    ] {
        rig.core.answer_always_headered(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            Headered(
                status,
                vec![("x-wonton-error".into(), kind.into())],
                serde_json::json!({"error": {"kind": kind, "message": "test"}}).to_string(),
            ),
        );
        wait_pages(&rig, |view| {
            view.problem.as_ref().and_then(|problem| problem.reason) == Some(expected)
        })
        .await;
    }
    rig.core
        .stop_answering_headered(Method::GET, BOT_PANEL_PINS_PATH);
    wait_pages(&rig, |view| view.problem.is_none()).await;
}
