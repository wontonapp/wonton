use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

async fn interrupted_default_change(reopen: bool) {
    let bed = Bed::start().await;
    let mut manager = bed.manager();
    let capability = "x-acme.power";
    for name in ["app.first", "app.second"] {
        install(&manager, &provider(name, capability, true)).await;
    }
    install(&manager, &consumer(capability)).await;
    manager
        .add_bot(&id("app.test"), "unconfirmed", BotKeeper::Household)
        .await
        .unwrap();
    select(&manager, capability, Some("default"), None).await;
    let token = bed.env("default")["TOKEN"].clone();
    let tmp = bed.path().with_extension("json.tmp");
    let blocked = tmp.clone();
    let injected = Arc::new(AtomicBool::new(false));
    let fired = injected.clone();
    bed.docker.on_exec_output(move |_, argv, _| {
        if matches!(argv, [_, action, name] if action == "issue" && name == "app.test")
            && !fired.swap(true, Ordering::SeqCst)
        {
            std::fs::create_dir(&blocked).unwrap();
        }
        (0, String::new(), String::new())
    });
    let error = manager
        .set_capability_default(capability, &id("app.second"), &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "storage");
    assert!(injected.load(Ordering::SeqCst));
    std::fs::remove_dir(&tmp).unwrap();
    if reopen {
        drop(manager);
        manager = bed.manager();
        manager.restore().await.unwrap();
    }
    manager
        .set_capability_default(capability, &id("app.second"), &langs())
        .await
        .unwrap();
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.second")
    );
    let env = bed.env("default");
    assert!(!env["URL"].is_empty());
    assert!(!env["KEY"].is_empty());
    assert_eq!(env["TOKEN"], token);
    assert!(matches!(
        manager
            .capability_route(
                &id("app.test"),
                capability,
                &langs(),
                Some(&bot("unconfirmed"))
            )
            .await,
        Err(CapabilityAccessError::Denied)
    ));
}

#[tokio::test]
async fn default_migration_write_failure_retries_the_original_bot_scope() {
    interrupted_default_change(false).await;
}

#[tokio::test]
async fn default_migration_write_failure_survives_a_manager_restart() {
    interrupted_default_change(true).await;
}

fn torrent_provider(name: &str, legacy_bot: bool) -> String {
    let identity = if legacy_bot {
        "  permissions: [matrix.bot]\n  agent: {protocol: matrix}\n  bot: {display_name: {en: Provider}}\n"
    } else {
        "  permissions: []\n"
    };
    let source = package(
        "  api:\n    image: provider:1\n",
        &format!("{identity}  provides:\n    torrent_client: {{service: api, port: 8080}}\n"),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {name}"));
    if legacy_bot {
        source.replace("kind: app", "kind: agent")
    } else {
        source
    }
}

#[tokio::test]
async fn last_legacy_bot_uses_an_ordinary_provider_without_a_prior_connection() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    install(&manager, &torrent_provider("app.provider", true)).await;
    install(&manager, &torrent_provider("app.other", false)).await;
    let source = package(
        "  app:\n    image: consumer:1\n    environment:\n      URL: ${WONTON_TORRENT_CLIENT_URL}\n",
        "  permissions: []\n  requires: [torrent_client]\n",
    ).replace("spec: '0.5'", "spec: '0.11'");
    install(&manager, &source).await;
    assert_eq!(
        route(&manager, "torrent_client", None).await,
        id("app.provider")
    );
    manager
        .remove_bot(&id("app.provider"), "default", true)
        .await
        .unwrap();
    assert_eq!(
        route(&manager, "torrent_client", None).await,
        id("app.other")
    );
    let env = env_of_container(
        &bed.docker,
        &super::super::super::names::container_name("app-test", "app"),
    );
    assert!(!env["URL"].is_empty());
}

async fn bind_power(manager: &PluginManager, caller: Option<&str>) {
    manager
        .set_capability_binding(
            &id("app.test"),
            caller,
            ConsumerCapabilityChoice {
                capability: "x-acme.power".into(),
                provider: Some("app.provider".into()),
            },
            Some("default"),
            &langs(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn bot_replacement_keeps_the_ordinary_plugin_connection_it_borrows() {
    let bed = Bed::start().await;
    let keys = super::super::capability_lifecycle::credentials(&bed.docker);
    let manager = bed.manager();
    let capability = "x-acme.power";
    install(&manager, &bot_provider(capability)).await;
    install(&manager, &provider("app.other", capability, false)).await;
    install(
        &manager,
        &consumer(capability).replace("  wants:", "  requires:"),
    )
    .await;
    bind_power(&manager, Some("default")).await;
    select(&manager, capability, None, Some("app.other")).await;
    let plugin_key = keys.lock().unwrap()["app.test"].clone();
    bind_power(&manager, None).await;
    let token = bed.env("default")["TOKEN"].clone();
    manager
        .remove_bot(&id("app.provider"), "default", true)
        .await
        .unwrap();
    assert_eq!(route(&manager, capability, None).await, id("app.provider"));
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.other")
    );
    assert_eq!(keys.lock().unwrap().get("app.test"), Some(&plugin_key));
    let env = bed.env("default");
    assert!(!env["URL"].is_empty());
    assert!(!env["KEY"].is_empty());
    assert_eq!(env["TOKEN"], token);
}

#[tokio::test]
async fn failed_revoke_and_explicit_retry_both_clear_the_retired_bots_credentials() {
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
    let failing = Arc::new(AtomicBool::new(true));
    let fail = failing.clone();
    let accounts = keys.clone();
    bed.docker.on_exec_output(move |_, argv, input| {
        match argv {
            [_, action, name]
                if action == "revoke"
                    && name == "app.test/default/worker"
                    && fail.load(Ordering::SeqCst) =>
            {
                return (17, String::new(), "revoke-worker-fault".into());
            }
            [_, action, name] if action == "issue" => {
                accounts
                    .lock()
                    .unwrap()
                    .insert(name.clone(), String::from_utf8_lossy(input).trim().into());
            }
            [_, action, name] if action == "revoke" => {
                accounts.lock().unwrap().remove(name);
            }
            _ => {}
        }
        (0, String::new(), String::new())
    });
    let error = manager
        .remove_bot(&id("app.provider"), "worker", true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("revoke-worker-fault"));
    let after_failure = bed.env("default");
    failing.store(false, Ordering::SeqCst);
    manager
        .remove_bot(&id("app.provider"), "worker", true)
        .await
        .unwrap();
    let after_retry = bed.env("default");
    assert!(after_failure["URL"].is_empty());
    assert!(after_failure["KEY"].is_empty());
    assert!(after_retry["URL"].is_empty());
    assert!(after_retry["KEY"].is_empty());
    assert_eq!(after_failure["TOKEN"], before["TOKEN"]);
    assert_eq!(after_retry["TOKEN"], before["TOKEN"]);
    assert!(!keys.lock().unwrap().contains_key("app.test/default/worker"));
}
