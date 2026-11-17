//! Bot 钉页在视图里的顺序、去重和最初的红点。
use super::*;

fn bot_pin(id: &str, tab: TabRef, attention: bool) -> wonton_protocol::BotPanelPin {
    wonton_protocol::BotPanelPin {
        id: id.to_owned(),
        scope: main_scope(),
        tab,
        title: format!("Bot 的 {id}"),
        attention,
        plugin_id: "app.shared".to_owned(),
        bot: "hermes".to_owned(),
        name: "Hermes".to_owned(),
    }
}

#[tokio::test]
async fn bot_pinned_pages_come_after_user_pages_and_count_toward_the_limit() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    {
        let mut state = lock(&rig.tabs.state);
        state.bot_pins = vec![
            bot_pin("a", page_tab(), false),
            bot_pin("b", url_tab("https://example.com/bot-1"), true),
            bot_pin("c", url_tab("https://example.com/bot-2"), false),
            bot_pin("d", url_tab("https://example.com/bot-3"), true),
        ];
    }
    rig.tabs.recompute();
    let view = rig.tabs.view();
    // 和用户页同一个页面的那一项只画一次。
    assert_eq!(view.panel.len(), 4);
    assert_eq!(view.panel[0].source, wonton_protocol::PanelSource::User);
    assert!(matches!(
        view.panel[1].source,
        wonton_protocol::PanelSource::Bot { .. }
    ));
    assert!(!view.panel[1].movable);
    assert!(view.panel[1].attention);
    rig.tabs
        .dispatch(PageAction::MovePanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/bot-3"),
            index: 0,
        })
        .unwrap();

    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/fifth"),
        })
        .unwrap();
    let sixth = rig
        .tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/sixth"),
        })
        .unwrap_err();
    assert!(matches!(sixth, CoreError::InvalidInput { .. }));

    rig.tabs
        .dispatch(PageAction::SelectPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/bot-3"),
        })
        .unwrap();
    assert!(!rig.tabs.view().panel[1].attention);
}

/// 已知连不上家里的服务器时，拉钉页失败不记成问题：连接提示已经说了，列表留着上次的。
#[tokio::test]
async fn a_known_outage_keeps_the_pins_without_a_settings_problem() {
    let rig = Rig::new();
    lock(&rig.tabs.state).bot_pins = vec![bot_pin("a", page_tab(), false)];
    rig.tabs
        .home_link
        .set_status_for_test(HomeStatus::Disconnected {
            reason: wonton_protocol::HomeFailure::TransportUnavailable,
            message: "测试".into(),
        });
    let tabs = Arc::new(rig.tabs);
    tabs.refresh_bot_pins().await;
    assert_eq!(tabs.view().problem, None);
    assert_eq!(lock(&tabs.state).bot_pins.len(), 1, "列表要留着上次的");

    // 连接不是已知断开时，拉不到照旧如实记下。
    tabs.home_link.set_status_for_test(HomeStatus::Connected);
    tabs.refresh_bot_pins().await;
    assert!(tabs.view().problem.is_some());
}
