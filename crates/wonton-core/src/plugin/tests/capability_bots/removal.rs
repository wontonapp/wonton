use super::*;

const POWER: &str = "x-acme.power";

async fn bind_bot(manager: &PluginManager, provider: &str) {
    manager
        .set_capability_binding(
            &id("app.test"),
            Some("default"),
            ConsumerCapabilityChoice {
                capability: POWER.into(),
                provider: Some(provider.into()),
            },
            Some("default"),
            &langs(),
        )
        .await
        .unwrap();
}

async fn bot_service_replacement(confirmed_bot: bool) -> (Bed, PluginManager) {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let provider = bot_service_provider(POWER);
    install(&manager, &provider).await;
    install(
        &manager,
        &provider.replace("id: app.provider", "id: app.other"),
    )
    .await;
    let source = consumer(POWER).replace("  wants:", "  requires:");
    install(&manager, &source).await;
    select(&manager, POWER, None, Some("app.other")).await;
    select(&manager, POWER, None, Some("app.provider")).await;
    if confirmed_bot {
        bind_bot(&manager, "app.other").await;
    }
    bind_bot(&manager, "app.provider").await;
    (bed, manager)
}

#[tokio::test]
async fn last_bot_removal_refuses_a_replacement_with_only_plugin_approval() {
    let (_bed, manager) = bot_service_replacement(false).await;
    let error = manager
        .remove_bot(&id("app.provider"), "default", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "in_use");
    assert_eq!(
        route(&manager, POWER, Some(&bot("default"))).await,
        id("app.provider")
    );
    assert_eq!(
        manager.stored_bot_names(&id("app.provider")).unwrap(),
        ["default"]
    );
}

#[tokio::test]
async fn last_bot_removal_persists_an_effective_confirmed_bot_replacement() {
    let (bed, manager) = bot_service_replacement(true).await;
    manager
        .remove_bot(&id("app.provider"), "default", true)
        .await
        .unwrap();
    let route = manager
        .capability_route(&id("app.test"), POWER, &langs(), Some(&bot("default")))
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.other"));
    assert_eq!(
        route.bot,
        Some(BotRef {
            plugin_id: "app.other".into(),
            bot: "default".into(),
        })
    );
    let state = bed.connections();
    assert_eq!(state["selected"]["app.test"][POWER], "app.other");
    assert_eq!(
        state["selected"]["app.test/default"][POWER],
        json!({"kind":"bot","plugin_id":"app.other","bot":"default"})
    );
}

#[tokio::test]
async fn failed_resource_removal_still_clears_revoked_credentials_from_consumers() {
    let bed = Bed::start().await;
    let keys = super::super::capability_lifecycle::credentials(&bed.docker);
    let manager = bed.manager();
    let capability = "x-acme.power";
    install(&manager, &credential_bot_provider(capability)).await;
    manager
        .add_bot(&id("app.provider"), "worker", BotKeeper::Household)
        .await
        .unwrap();
    install(&manager, &consumer(capability)).await;
    manager
        .set_capability_binding(
            &id("app.test"),
            Some("default"),
            ConsumerCapabilityChoice {
                capability: capability.into(),
                provider: Some("app.provider".into()),
            },
            Some("worker"),
            &langs(),
        )
        .await
        .unwrap();
    let before = bed.env("default");
    assert!(!before["KEY"].is_empty());
    let worker =
        crate::plugin::names::Instance::default().bot_container("app-provider", "bot", "worker");
    bed.docker.fail_remove(&worker, Some("remove-worker-fault"));
    let error = manager
        .remove_bot(&id("app.provider"), "worker", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "docker_rejected");
    assert!(error.to_string().contains("remove-worker-fault"));
    assert!(!keys.lock().unwrap().contains_key("app.test/default/worker"));
    let after = bed.env("default");
    assert!(after["URL"].is_empty());
    assert!(after["KEY"].is_empty());
    assert_eq!(after["TOKEN"], before["TOKEN"]);
    bed.docker.fail_remove(&worker, None);
    manager
        .remove_bot(&id("app.provider"), "worker", true)
        .await
        .unwrap();
    assert_eq!(bed.env("default"), after);
}
