use super::*;

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
