use super::*;
use crate::plugin::PluginStatus;

async fn install_optional_consumer(bed: &Bed) {
    install_existing_search(bed).await;
    let provider = PluginPackage::parse(
        &provider("app.ledger", "x-acme.ledger", bed.web_port)
            .replace("  permissions: [network.lan]", "  permissions: []")
            .replace("  optional_permissions:\n    docker.socket:\n      off_loss: {en: No container access, zh: 无法访问容器}\n", "")
            .replace(&format!("  lan:\n    ports: [{{service: api, port: {}}}]\n", bed.web_port), "")
            .replace("    volumes: ['/var/run/docker.sock:/var/run/docker.sock']\n", ""),
        &langs(),
    ).unwrap();
    IndexEntry::of(&provider, "github:acme/ledger", &"c".repeat(40))
        .unwrap()
        .write_to(bed._index.path(), &provider)
        .unwrap();
    let compose = consumer("app.optional", &["x-acme.search"])
        .replace("  requires: [x-acme.search]", "  requires: [x-acme.search]\n  wants: [x-acme.ledger]")
        .replace("      KEY: ${WONTON_X_ACME_SEARCH_KEY}", "      KEY: ${WONTON_X_ACME_SEARCH_KEY}\n      OPTIONAL_URL: ${WONTON_X_ACME_LEDGER_URL}\n      OPTIONAL_KEY: ${WONTON_X_ACME_LEDGER_KEY}");
    let preview = bed
        .manager
        .preview_upload(PluginPackage::parse(&compose, &langs()).unwrap(), &langs())
        .await
        .unwrap();
    bed.manager
        .install_source(&request(preview, "original-consumer-token"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_failed_optional_rebuild_restores_existing_config_and_reports_revoke_failure() {
    let bed = bed().await;
    install_optional_consumer(&bed).await;
    let web = container_name("app-optional", "web");
    let original = bed.docker.created(&web).unwrap();
    let before = connections(&bed);
    let keys = bed.keys.lock().unwrap().clone();
    let secrets = stored_secrets(&bed, "app.optional");
    let record = bed
        .manager
        .store
        .load(&id("app.optional"))
        .unwrap()
        .unwrap();
    let marker = bed.data.path().join("apps/app-optional/owner-data");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    fs::write(&marker, "keep").unwrap();
    let outer = transaction(&bed, "app.test", &["x-acme.search", "x-acme.ledger"]).await;
    let docker = bed.docker.clone();
    let issued = bed.keys.clone();
    bed.docker.on_exec(move |container, argv, stdin| {
        let [command, action, name] = argv else { return (2, String::new()) };
        if command != "credentials" { return (2, String::new()) }
        match action.as_str() {
            "issue" => {
                issued.lock().unwrap().insert(
                    (container.to_owned(), name.clone()),
                    String::from_utf8_lossy(stdin).trim().to_owned(),
                );
                if name == "app.optional" {
                    docker.set_start_state(&container_name("app-optional", "web"),
                        json!({"Status": "exited", "Running": false, "ExitCode": 43}));
                }
            }
            "revoke" => {
                issued.lock().unwrap().remove(&(container.to_owned(), name.clone()));
                if name == "app.optional" {
                    docker.set_start_state(&container_name("app-optional", "web"),
                        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}));
                    return (17, "rollback revoke rejected".to_owned());
                }
            }
            _ => return (2, String::new()),
        }
        (0, String::new())
    });
    let error = bed
        .manager
        .install_source(&outer)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("43"), "{error}");
    assert!(error.contains("rollback revoke rejected"), "{error}");
    assert_eq!(bed.docker.created(&web).unwrap(), original);
    assert_eq!(bed.docker.creations(&web), 3);
    assert_eq!(
        bed.manager.status_of(&id("app.optional")),
        Some(PluginStatus::Running)
    );
    assert_eq!(
        bed.manager
            .store
            .load(&id("app.optional"))
            .unwrap()
            .unwrap(),
        record
    );
    assert_eq!(stored_secrets(&bed, "app.optional"), secrets);
    assert_eq!(fs::read_to_string(marker).unwrap(), "keep");
    assert_eq!(connections(&bed), before);
    assert_eq!(*bed.keys.lock().unwrap(), keys);
    for (plugin, service) in [("app.ledger", "api"), ("app.test", "web")] {
        assert!(!bed.manager.is_installed(&id(plugin)));
        assert!(
            !bed.docker
                .containers()
                .contains(&container_name(&id(plugin).slug(), service))
        );
        assert_eq!(stored_secrets(&bed, plugin), None);
    }
    let route = bed
        .manager
        .capability_route(&id("app.optional"), "x-acme.search", &langs())
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.search"));
}

#[tokio::test]
async fn raw_provider_failure_reports_failed_existing_consumer_recovery_without_hiding_cause() {
    let bed = bed().await;
    install_optional_consumer(&bed).await;
    let compose = crate::plugin::tests::capability_deps_install::provider(
        "app.ledger",
        "x-acme.ledger",
        CREDENTIALS,
    );
    let package = PluginPackage::parse(&compose, &langs()).unwrap();
    bed.manager
        .store
        .secrets()
        .save(
            &id("app.ledger"),
            &[("token".to_owned(), b"retained-provider-token".to_vec())].into(),
        )
        .unwrap();
    let docker = bed.docker.clone();
    let keys = bed.keys.clone();
    bed.docker.on_exec(move |container, argv, stdin| {
        let [command, action, name] = argv else {
            return (2, String::new());
        };
        if command != "credentials" {
            return (2, String::new());
        }
        if action == "issue" {
            keys.lock().unwrap().insert(
                (container.to_owned(), name.clone()),
                String::from_utf8_lossy(stdin).trim().to_owned(),
            );
            if name == "app.optional" {
                docker.fail_starts(Some("existing consumer rebuild rejected"));
            }
        } else if action == "revoke" {
            keys.lock()
                .unwrap()
                .remove(&(container.to_owned(), name.clone()));
        }
        (0, String::new())
    });
    let error = bed
        .manager
        .install_checked(package, &[], &[], false, Default::default(), &langs())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("existing consumer rebuild rejected"),
        "{error}"
    );
    assert!(
        error.contains("app.optional"),
        "cleanup context missing: {error}"
    );
    assert!(matches!(
        bed.manager.status_of(&id("app.optional")),
        Some(PluginStatus::Degraded { .. })
    ));
    assert!(!bed.manager.is_installed(&id("app.ledger")));
    assert_eq!(
        stored_secrets(&bed, "app.ledger"),
        Some([("token".to_owned(), b"retained-provider-token".to_vec()),].into())
    );
}

#[tokio::test]
async fn an_installed_slug_collision_rejects_nested_providers_before_side_effects() {
    let bed = bed().await;
    let existing = package(
        "  web:\n    image: consumer:1\n",
        &format!("  permissions: []\n{INPUT}"),
    )
    .replace("id: app.test", "id: app.foo.bar");
    let preview = bed
        .manager
        .preview_upload(PluginPackage::parse(&existing, &langs()).unwrap(), &langs())
        .await
        .unwrap();
    bed.manager
        .install_source(&request(preview, "existing-owner-token"))
        .await
        .unwrap();
    let outer = transaction(&bed, "app.foo-bar", &["x-acme.search"]).await;
    let writes = bed.docker.writes();
    let before = connections(&bed);
    let secrets = stored_secrets(&bed, "app.foo.bar");
    bed.manager.install_source(&outer).await.unwrap_err();
    assert_eq!(bed.docker.writes(), writes);
    assert_eq!(
        bed.docker.creations(&container_name("app-search", "api")),
        0
    );
    assert!(!bed.manager.is_installed(&id("app.search")));
    assert!(!bed.manager.is_installed(&id("app.foo-bar")));
    assert!(bed.manager.is_installed(&id("app.foo.bar")));
    assert_eq!(stored_secrets(&bed, "app.search"), None);
    assert_eq!(stored_secrets(&bed, "app.foo-bar"), None);
    assert_eq!(stored_secrets(&bed, "app.foo.bar"), secrets);
    assert_eq!(connections(&bed), before);
    assert!(bed.keys.lock().unwrap().is_empty());
}
