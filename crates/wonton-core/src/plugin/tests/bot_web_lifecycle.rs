use super::*;

#[tokio::test]
async fn bot_web_removal_and_reinstall_preserve_only_retained_identities() {
    let bed = Bed::start().await;
    let web = super::super::egress::serve(axum::Router::new().fallback(|| async { "ok" })).await;
    let source = agent().replace(
        "  bots: {services: [bot]}",
        &format!(
            "  webui: {{service: bot, port: {}}}\n  bots: {{services: [bot]}}",
            web.port()
        ),
    );
    let mut manager = bed.manager();
    manager.cfg.core_container = None;
    manager
        .install(&source, &granted_with_tools(), super::super::langs())
        .await
        .unwrap();
    manager
        .add_bot(&id(), "family", BotKeeper::Household)
        .await
        .unwrap();
    let identity = wonton_protocol::BotRef {
        plugin_id: id().to_string(),
        bot: "family".to_owned(),
    };
    let label = wonton_protocol::bot_web_label(&identity);
    assert_eq!(
        manager.bot_web_identity(&label).unwrap(),
        Some(identity.clone())
    );
    let (_, page) = manager.bot_web_target(&identity).unwrap();
    assert!(page.url.contains(&label));
    manager
        .uninstall(&id(), false, &super::super::langs())
        .await
        .unwrap();
    assert_eq!(
        manager.bot_web_identity(&label).unwrap(),
        Some(identity.clone())
    );
    manager
        .install(&source, &granted_with_tools(), super::super::langs())
        .await
        .unwrap();
    assert_eq!(manager.bot_web_target(&identity).unwrap().1, page);
    manager.remove_bot(&id(), "family", true).await.unwrap();
    assert!(manager.bot_web_identity(&label).unwrap().is_none());
    assert!(matches!(
        manager.bot_web_target(&identity),
        Err(super::super::super::bot_web::BotWebError::Unavailable(
            wonton_protocol::BotWebFailure::NotFound
        ))
    ));
    let default = wonton_protocol::BotRef {
        plugin_id: id().to_string(),
        bot: "default".to_owned(),
    };
    assert!(manager.bot_web_target(&default).is_ok());
}
