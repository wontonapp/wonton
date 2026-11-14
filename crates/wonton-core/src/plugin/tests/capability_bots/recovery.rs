use super::*;

async fn recover_provider(manager: &PluginManager, name: &str) {
    let record = manager.store.load(&id(name)).unwrap().unwrap();
    let installed = crate::plugin::upgrade::installed_package(&record, &langs()).unwrap();
    manager
        .capabilities_installed(&installed.manifest, &[], true)
        .await
        .unwrap();
}

#[tokio::test]
async fn forced_recovery_restores_only_confirmed_bot_accounts_with_the_same_key() {
    let bed = Bed::start().await;
    let keys = super::super::capability_lifecycle::credentials(&bed.docker);
    let manager = bed.manager();
    let capability = "x-acme.power";
    let source = credential_bot_provider(capability);
    install(&manager, &source).await;
    manager
        .add_bot(&id("app.provider"), "worker", BotKeeper::Household)
        .await
        .unwrap();
    install(&manager, &provider("app.other", capability, true)).await;
    install(&manager, &consumer(capability)).await;
    for name in ["unconfirmed", "pinned"] {
        manager
            .add_bot(&id("app.test"), name, BotKeeper::Household)
            .await
            .unwrap();
    }
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
    select(&manager, capability, Some("pinned"), Some("app.other")).await;
    let before = bed.env("default");
    let pinned = bed.env("pinned");
    assert!(!before["KEY"].is_empty());
    assert_eq!(
        keys.lock().unwrap().get("app.test/default/worker"),
        Some(&before["KEY"])
    );
    assert_eq!(
        keys.lock().unwrap().get("app.test/pinned"),
        Some(&pinned["KEY"])
    );

    keys.lock().unwrap().clear();
    recover_provider(&manager, "app.provider").await;
    assert_eq!(
        keys.lock().unwrap().get("app.test/default/worker"),
        Some(&before["KEY"])
    );
    assert!(!keys.lock().unwrap().contains_key("app.test/pinned"));
    assert!(!keys.lock().unwrap().contains_key("app.test/unconfirmed"));
    assert_eq!(bed.env("default"), before);
    assert!(matches!(
        manager
            .capability_route(
                &id("app.test"),
                capability,
                &langs(),
                Some(&bot("unconfirmed")),
            )
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    recover_provider(&manager, "app.other").await;
    assert_eq!(
        keys.lock().unwrap().get("app.test/pinned"),
        Some(&pinned["KEY"])
    );
    assert_eq!(bed.env("pinned"), pinned);
}

#[tokio::test]
async fn changing_default_confirms_only_previously_approved_inheriting_bots() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.power";
    for name in ["app.first", "app.second"] {
        install(&manager, &provider(name, capability, true)).await;
    }
    install(&manager, &consumer(capability)).await;
    for name in ["following", "pinned", "unconfirmed"] {
        manager
            .add_bot(&id("app.test"), name, BotKeeper::Household)
            .await
            .unwrap();
    }
    select(&manager, capability, Some("default"), None).await;
    select(&manager, capability, Some("following"), None).await;
    select(&manager, capability, Some("pinned"), Some("app.first")).await;
    let pinned = bed.env("pinned");
    let unconfirmed = bed.env("unconfirmed");
    manager
        .set_capability_default(capability, &id("app.second"), &langs())
        .await
        .unwrap();
    for name in ["default", "following"] {
        assert_eq!(
            route(&manager, capability, Some(&bot(name))).await,
            id("app.second")
        );
        assert!(!bed.env(name)["KEY"].is_empty());
    }
    assert_eq!(
        route(&manager, capability, Some(&bot("pinned"))).await,
        id("app.first")
    );
    assert_eq!(bed.env("pinned"), pinned);
    assert_eq!(bed.env("unconfirmed"), unconfirmed);
    assert!(matches!(
        manager
            .capability_route(
                &id("app.test"),
                capability,
                &langs(),
                Some(&bot("unconfirmed")),
            )
            .await,
        Err(CapabilityAccessError::Denied)
    ));
}

async fn required_bot_override() -> (Bed, Arc<PluginManager>) {
    let bed = Bed::start().await;
    let manager = Arc::new(bed.manager());
    let capability = "x-acme.search";
    for name in ["app.first", "app.second"] {
        install(&manager, &provider(name, capability, false)).await;
    }
    install(
        &manager,
        &consumer(capability).replace("  wants:", "  requires:"),
    )
    .await;
    select(&manager, capability, Some("default"), Some("app.second")).await;
    (bed, manager)
}

#[tokio::test]
async fn required_dependency_hints_include_an_unavailable_bot_override() {
    let (bed, manager) = required_bot_override().await;
    let provider = crate::plugin::names::container_name("app-second", "api");
    bed.docker.set_state(
        &provider,
        json!({"Status":"exited","Running":false,"ExitCode":1}),
    );
    manager.watch_containers().await;
    let detail = manager
        .judged_detail(&id("app.test"), &langs())
        .await
        .unwrap();
    assert!(detail.hints.iter().any(|hint| {
        hint.category == wonton_protocol::PluginHintCategory::DependencyStopped
            && hint.capability.as_deref() == Some("x-acme.search")
            && hint.provider.as_ref().is_some_and(|p| p.id == "app.second")
    }));
    assert_eq!(
        route(&manager, "x-acme.search", None).await,
        id("app.first")
    );
    assert!(matches!(
        manager
            .capability_route(
                &id("app.test"),
                "x-acme.search",
                &langs(),
                Some(&bot("default"))
            )
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
}

#[tokio::test]
async fn starting_a_consumer_recovers_its_bot_override_but_respects_owner_stop() {
    let (bed, manager) = required_bot_override().await;
    manager.stop_detached(id("app.test")).await.unwrap();
    let provider = crate::plugin::names::container_name("app-second", "api");
    bed.docker.set_state(
        &provider,
        json!({"Status":"exited","Running":false,"ExitCode":1}),
    );
    bed.docker.set_start_state(
        &provider,
        json!({"Status":"running","Running":true,"Health":{"Status":"healthy"}}),
    );
    manager.watch_containers().await;
    manager.start_detached(id("app.test")).await.unwrap();
    assert!(matches!(
        manager
            .installed_plugin(&id("app.second"), &langs())
            .unwrap()
            .status,
        wonton_protocol::PluginState::Running { .. }
    ));
    assert_eq!(
        route(&manager, "x-acme.search", Some(&bot("default"))).await,
        id("app.second")
    );
    assert_eq!(
        route(&manager, "x-acme.search", None).await,
        id("app.first")
    );
    manager.stop_detached(id("app.test")).await.unwrap();
    manager.stop_detached(id("app.second")).await.unwrap();
    assert!(manager.start_detached(id("app.test")).await.is_err());
    assert!(matches!(
        manager
            .installed_plugin(&id("app.second"), &langs())
            .unwrap()
            .status,
        wonton_protocol::PluginState::Stopped
    ));
    assert!(matches!(
        manager
            .capability_route(
                &id("app.test"),
                "x-acme.search",
                &langs(),
                Some(&bot("default"))
            )
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
}
