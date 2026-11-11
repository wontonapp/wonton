//! 插件网页要摄像头/麦克风：按插件记决定，没问过的出询问，别的源一律拒。

use super::*;

fn origin_of(plugin_id: &str) -> String {
    format!("http://{plugin_id}.wonton.localhost")
}

fn foreign_origin() -> String {
    "http://evil.example".to_owned()
}

fn prompt_of(view: &TabsViewModel) -> &wonton_client::TabMediaPrompt {
    view.media_prompt
        .as_ref()
        .unwrap_or_else(|| panic!("没有待回答的询问: {view:?}"))
}

fn grant_of(view: &wonton_client::PluginsViewModel, plugin_id: &str) -> Option<bool> {
    view.media_grants
        .iter()
        .find(|grant| grant.plugin_id == plugin_id)
        .map(|grant| grant.allowed)
}

#[tokio::test]
async fn first_request_prompts_and_an_allow_persists() {
    let (rig, page_tab, _) = connected_page_rig("app.media", "call").await;
    let place = PlacedTab::Bar {
        tab: page_tab.clone(),
    };
    let origin = origin_of("app.media");

    // 首次请求：出询问，写清哪个插件、要什么。
    let decision = rig
        .app
        .tab_media_permission_request(place.clone(), origin.clone(), true, true)
        .unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Prompt);
    let view = rig.app.tabs();
    let prompt = prompt_of(&view);
    assert_eq!(prompt.plugin_id, "app.media");
    assert_eq!(prompt.plugin_title, "标题 app.media");
    assert!(prompt.camera && prompt.microphone);
    assert!(grant_of(&rig.app.plugins(), "app.media").is_none());

    // 允许：网页拿到放行，决定按插件记下。
    let decision = rig.app.tab_media_permission_decide(true).unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Allow);
    assert!(rig.app.tabs().media_prompt.is_none());
    assert_eq!(grant_of(&rig.app.plugins(), "app.media"), Some(true));

    // 同一插件再请求：直接放行，不再出询问。
    let decision = rig
        .app
        .tab_media_permission_request(place, origin, true, false)
        .unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Allow);
    assert!(rig.app.tabs().media_prompt.is_none());
}

#[tokio::test]
async fn a_denied_answer_rejects_later_requests() {
    let (rig, page_tab, _) = connected_page_rig("app.media", "call").await;
    let place = PlacedTab::Bar {
        tab: page_tab.clone(),
    };
    rig.app
        .tab_media_permission_request(place.clone(), origin_of("app.media"), false, true)
        .unwrap();
    assert_eq!(
        rig.app.tab_media_permission_decide(false).unwrap(),
        wonton_client::TabMediaPermission::Deny
    );
    assert_eq!(grant_of(&rig.app.plugins(), "app.media"), Some(false));

    let later = rig
        .app
        .tab_media_permission_request(place, origin_of("app.media"), true, true)
        .unwrap();
    assert_eq!(later, wonton_client::TabMediaPermission::Deny);
    assert!(rig.app.tabs().media_prompt.is_none());
}

#[tokio::test]
async fn decisions_are_kept_per_plugin() {
    let rig = Rig::new().await;
    let first = web_plugin(
        "app.first",
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("call", TabDisplay::Standard)],
    );
    let second = web_plugin(
        "app.second",
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("call", TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![first, second]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let first_tab = tab("app.first", Some("call"));
    let second_tab = tab("app.second", Some("call"));
    rig.app
        .dispatch(Action::PinTab {
            tab: first_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::PinTab {
            tab: second_tab.clone(),
            replacing: None,
        })
        .unwrap();

    rig.app
        .tab_media_permission_request(
            PlacedTab::Bar { tab: first_tab },
            origin_of("app.first"),
            true,
            false,
        )
        .unwrap();
    rig.app.tab_media_permission_decide(true).unwrap();

    // 另一个插件没问过：重新出询问，不和第一个共用决定。
    let decision = rig
        .app
        .tab_media_permission_request(
            PlacedTab::Bar { tab: second_tab },
            origin_of("app.second"),
            true,
            true,
        )
        .unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Prompt);
    assert_eq!(prompt_of(&rig.app.tabs()).plugin_id, "app.second");
    assert_eq!(grant_of(&rig.app.plugins(), "app.first"), Some(true));
    assert!(grant_of(&rig.app.plugins(), "app.second").is_none());
}

#[tokio::test]
async fn foreign_origins_are_always_rejected() {
    let (rig, page_tab, _) = connected_page_rig("app.media", "call").await;
    let place = PlacedTab::Bar {
        tab: page_tab.clone(),
    };

    // 没问过也不出询问：别的源（页面里嵌的第三方 iframe）直接拒。
    let decision = rig
        .app
        .tab_media_permission_request(place.clone(), foreign_origin(), true, true)
        .unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Deny);
    assert!(rig.app.tabs().media_prompt.is_none());

    // 记下的允许也只属于页面自己的源。
    rig.app
        .tab_media_permission_request(place.clone(), origin_of("app.media"), true, false)
        .unwrap();
    rig.app.tab_media_permission_decide(true).unwrap();
    let leaked = rig
        .app
        .tab_media_permission_request(place, foreign_origin(), true, true)
        .unwrap();
    assert_eq!(leaked, wonton_client::TabMediaPermission::Deny);
}

#[tokio::test]
async fn settings_reset_asks_again_and_rejects_clearing_an_unset_grant() {
    let (rig, page_tab, _) = connected_page_rig("app.media", "call").await;
    let place = PlacedTab::Bar {
        tab: page_tab.clone(),
    };
    rig.app
        .tab_media_permission_request(place.clone(), origin_of("app.media"), true, true)
        .unwrap();
    rig.app.tab_media_permission_decide(true).unwrap();
    assert_eq!(grant_of(&rig.app.plugins(), "app.media"), Some(true));

    // 没问过的插件清不掉。
    assert!(matches!(
        rig.app.dispatch(Action::SetPluginMediaPermission {
            plugin_id: "app.other".to_owned(),
            allow: false,
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(grant_of(&rig.app.plugins(), "app.media"), Some(true));

    // 详情页把开关改回没问过：记录清掉，下次请求重新出询问。
    rig.app
        .dispatch(Action::SetPluginMediaPermission {
            plugin_id: "app.media".to_owned(),
            allow: false,
        })
        .unwrap();
    assert_eq!(grant_of(&rig.app.plugins(), "app.media"), None);

    let again = rig
        .app
        .tab_media_permission_request(place, origin_of("app.media"), true, true)
        .unwrap();
    assert_eq!(again, wonton_client::TabMediaPermission::Prompt);
    assert_eq!(prompt_of(&rig.app.tabs()).plugin_id, "app.media");

    // 开关给回允许：再请求直接放行。
    rig.app
        .dispatch(Action::SetPluginMediaPermission {
            plugin_id: "app.media".to_owned(),
            allow: true,
        })
        .unwrap();
    let after = rig
        .app
        .tab_media_permission_request(
            PlacedTab::Bar {
                tab: page_tab.clone(),
            },
            origin_of("app.media"),
            true,
            true,
        )
        .unwrap();
    assert_eq!(after, wonton_client::TabMediaPermission::Allow);

    // 非法输入：既不要摄像头也不要麦克风。
    assert!(matches!(
        rig.app.tab_media_permission_request(
            PlacedTab::Bar { tab: page_tab },
            origin_of("app.media"),
            false,
            false,
        ),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn prompt_clears_when_its_page_is_removed() {
    let (rig, page_tab, _) = connected_page_rig("app.media", "call").await;
    let place = PlacedTab::Bar {
        tab: page_tab.clone(),
    };
    rig.app
        .tab_media_permission_request(place.clone(), origin_of("app.media"), true, true)
        .unwrap();
    assert!(rig.app.tabs().media_prompt.is_some());

    // 页面从底栏拿掉：询问跟着清掉，再答就是错的。
    rig.app
        .dispatch(Action::UnpinTab { tab: page_tab })
        .unwrap();
    assert!(rig.app.tabs().media_prompt.is_none());
    assert!(matches!(
        rig.app.tab_media_permission_decide(true),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn pages_without_a_plugin_decision_are_denied() {
    let (rig, _page_tab, _) = connected_page_rig("app.media", "call").await;
    // 不是插件页（聊天页没有网页）：直接拒，不出询问。
    let decision = rig
        .app
        .tab_media_permission_request(
            PlacedTab::Bar { tab: TabRef::Chat },
            origin_of("app.media"),
            true,
            true,
        )
        .unwrap();
    assert_eq!(decision, wonton_client::TabMediaPermission::Deny);
    assert!(rig.app.tabs().media_prompt.is_none());
}
