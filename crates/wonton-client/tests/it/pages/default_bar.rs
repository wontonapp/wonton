//! 底栏（ADR-0126）：默认 Agent、应用两项，根页跟着选中的栏项走，旧配置落默认。

use super::*;
use wonton_client::Action;

fn select(entry: BarEntry) -> Action {
    Action::Pages {
        action: PageAction::SelectBarItem { entry },
    }
}

#[tokio::test]
async fn the_bar_defaults_to_agent_and_apps_and_keeps_the_selected_root() {
    let rig = Rig::new().await;
    let unpaired = rig.app.pages();
    // 没配对时根页照旧是 Agent；配对页盖在它上面。
    assert_eq!(unpaired.tab, BarEntry::Agent);
    assert_eq!(
        unpaired
            .bar
            .items
            .iter()
            .map(|item| item.entry.clone())
            .collect::<Vec<_>>(),
        [BarEntry::Agent, BarEntry::Apps]
    );
    assert_eq!(unpaired.bar.max_items, MAX_BAR_ITEMS);
    assert!(unpaired.stack.is_empty());

    rig.connected().await;
    let paired = wait_pages(&rig, |view| view.tab == BarEntry::Agent).await;
    assert_eq!(paired.tab, BarEntry::Agent);
    assert!(paired.stack.is_empty());
    assert!(paired.my_app_candidates.is_empty());
}

#[tokio::test]
async fn an_old_config_without_a_bar_gets_the_defaults() {
    let mut rig = Rig::new().await;
    rig.connected().await;
    wait_pages(&rig, |view| view.tab == BarEntry::Agent).await;
    // 老安装的 layout.json 没有 bar 字段：补默认两项，不整份作废。
    std::fs::write(
        rig.data_dir().join("layout.json"),
        serde_json::json!({
            "server_id": "home",
            "my_apps": [],
            "auto_add": true,
            "panel": [],
        })
        .to_string(),
    )
    .unwrap();
    rig.reopen();
    let reopened = rig.app.pages();
    assert_eq!(reopened.tab, BarEntry::Agent);
    assert_eq!(
        reopened
            .bar
            .items
            .iter()
            .map(|item| item.entry.clone())
            .collect::<Vec<_>>(),
        [BarEntry::Agent, BarEntry::Apps]
    );
    assert!(reopened.problem.is_none());

    // 显式存了空栏：读的时候按默认补回来，不能没有开口。
    std::fs::write(
        rig.data_dir().join("layout.json"),
        serde_json::json!({
            "server_id": "home",
            "my_apps": [],
            "auto_add": true,
            "panel": [],
            "bar": [],
        })
        .to_string(),
    )
    .unwrap();
    rig.reopen();
    assert_eq!(rig.app.pages().bar.items.len(), 2);
}

/// 老安装的整份形状（items/labels）读不出来：退回默认，原因照旧说清。
#[tokio::test]
async fn an_old_items_and_labels_layout_falls_back_to_the_defaults() {
    let mut rig = Rig::new().await;
    rig.connected().await;
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
    assert!(reopened.problem.is_some());
    assert!(reopened.auto_add);
    assert!(reopened.my_app_candidates.is_empty());
    assert_eq!(reopened.bar.items.len(), 2);
}

#[tokio::test]
async fn deep_links_pick_root_bar_items_and_old_links_still_work() {
    let rig = Rig::new().await;
    rig.connected().await;
    wait_pages(&rig, |view| view.tab == BarEntry::Agent).await;

    // 链接选的是栏项，不是页面：不压栈，只换根。
    let apps = wonton_client::parse_app_link("wonton://tab/apps".to_owned()).unwrap();
    let wonton_protocol::AppLink::Bar {
        entry: BarEntry::Apps,
    } = apps
    else {
        panic!("expected a bar link");
    };
    rig.app.dispatch(select(BarEntry::Apps)).unwrap();
    let switched = rig.app.pages();
    assert_eq!(switched.tab, BarEntry::Apps);
    assert!(switched.stack.is_empty());

    // 旧版发的 tab/chat 落回 Agent 根。
    let chat = wonton_client::parse_app_link("wonton://tab/chat".to_owned()).unwrap();
    assert!(matches!(
        chat,
        wonton_protocol::AppLink::Bar {
            entry: BarEntry::Agent
        }
    ));
    rig.app.dispatch(select(BarEntry::Agent)).unwrap();
    assert_eq!(rig.app.pages().tab, BarEntry::Agent);
}

#[tokio::test]
async fn the_last_bar_item_cannot_be_removed() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.app.dispatch(select(BarEntry::Agent)).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: PageAction::RemoveBarItem {
                entry: BarEntry::Apps,
            },
        })
        .unwrap();
    assert_eq!(rig.app.pages().bar.items.len(), 1);
    let last = rig.app.pages().bar.items[0].entry.clone();
    assert_invalid(rig.app.dispatch(Action::Pages {
        action: PageAction::RemoveBarItem { entry: last },
    }));
}

fn url_app(id: u32) -> MyAppRef {
    MyAppRef::Page {
        tab: TabRef::Url {
            url: format!("https://example.com/{id}"),
        },
    }
}

async fn add_my_app(rig: &Rig, app: MyAppRef) {
    rig.app
        .dispatch(Action::Pages {
            action: PageAction::AddMyApp {
                app,
                title: None,
                index: None,
            },
        })
        .unwrap();
}

#[tokio::test]
async fn adding_to_the_bar_checks_duplicates_the_limit_and_moves() {
    let rig = Rig::new().await;
    rig.connected().await;
    for id in 0..4 {
        add_my_app(&rig, url_app(id)).await;
        rig.app
            .dispatch(Action::Pages {
                action: PageAction::AddBarItem { app: url_app(id) },
            })
            .unwrap();
    }
    assert_eq!(rig.app.pages().bar.items.len(), 6);
    assert!(
        rig.app
            .dispatch(Action::Pages {
                action: PageAction::AddBarItem { app: url_app(0) },
            })
            .is_err()
    );
    add_my_app(&rig, url_app(4)).await;
    assert_invalid(rig.app.dispatch(Action::Pages {
        action: PageAction::AddBarItem { app: url_app(4) },
    }));
    assert!(rig.app.pages().bar.full);

    // 加成功了才清 full。
    rig.app
        .dispatch(Action::Pages {
            action: PageAction::RemoveBarItem {
                entry: BarEntry::Apps,
            },
        })
        .unwrap();
    assert!(!rig.app.pages().bar.full);
    rig.app
        .dispatch(Action::Pages {
            action: PageAction::AddBarItem { app: url_app(4) },
        })
        .unwrap();
    assert!(!rig.app.pages().bar.full);

    rig.app
        .dispatch(Action::Pages {
            action: PageAction::MoveBarItem { from: 0, to: 5 },
        })
        .unwrap();
    assert_eq!(rig.app.pages().bar.items[5].entry, BarEntry::Agent);
    assert_invalid(rig.app.dispatch(Action::Pages {
        action: PageAction::MoveBarItem { from: 0, to: 6 },
    }));
    assert_invalid(rig.app.dispatch(Action::Pages {
        action: PageAction::RemoveBarItem {
            entry: BarEntry::App { app: url_app(99) },
        },
    }));
}

#[tokio::test]
async fn removed_roots_are_still_selectable() {
    let rig = Rig::new().await;
    rig.connected().await;
    add_my_app(&rig, url_app(0)).await;
    rig.app
        .dispatch(Action::Pages {
            action: PageAction::AddBarItem { app: url_app(0) },
        })
        .unwrap();
    for entry in [BarEntry::Agent, BarEntry::Apps] {
        rig.app
            .dispatch(Action::Pages {
                action: PageAction::RemoveBarItem { entry },
            })
            .unwrap();
    }
    assert_eq!(
        rig.app
            .pages()
            .bar
            .items
            .iter()
            .map(|item| item.entry.clone())
            .collect::<Vec<_>>(),
        [BarEntry::App { app: url_app(0) }]
    );
    // 侧边栏、通知和新聊天都直接选根页，不管它在不在栏上。
    rig.app.dispatch(select(BarEntry::Apps)).unwrap();
    assert_eq!(rig.app.pages().tab, BarEntry::Apps);
    rig.app.dispatch(select(BarEntry::Agent)).unwrap();
    assert_eq!(rig.app.pages().tab, BarEntry::Agent);
    // 应用项不在栏上时不能选。
    assert_invalid(rig.app.dispatch(select(BarEntry::App { app: url_app(1) })));
}
