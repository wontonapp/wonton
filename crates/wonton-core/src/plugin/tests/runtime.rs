use serde_json::{Value, json};

use super::super::manifest::Runtime;
use super::super::package::{PackageError, PluginPackage};

fn document(runtime: Option<&str>, services: Option<Value>) -> Value {
    let mut doc = json!({"x-wonton": {
        "spec": "0.10", "id": "app.forms", "version": "1.0.0", "kind": "app",
        "title": {"en": "Forms"}, "description": {"en": "Forms"}, "permissions": []
    }});
    if let Some(runtime) = runtime {
        doc["x-wonton"]["runtime"] = runtime.into();
    }
    if let Some(services) = services {
        doc["services"] = services;
    }
    doc
}

fn parse(doc: &Value) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(&doc.to_string(), &super::langs())
}

#[test]
fn runtime_determines_whether_services_are_allowed() {
    let services = json!({"web": {"image": "nginx"}});
    for runtime in [None, Some("compose")] {
        let parsed = parse(&document(runtime, Some(services.clone()))).unwrap();
        assert_eq!(parsed.manifest.runtime, Runtime::Compose);
        assert!(parsed.has_service("web"));
        for absent_or_invalid in [None, Some(json!({})), Some(Value::Null), Some(json!([]))] {
            assert!(matches!(
                parse(&document(runtime, absent_or_invalid)),
                Err(PackageError::Service { .. })
            ));
        }
    }
    let skills = document(Some("skills"), None);
    assert_eq!(parse(&skills).unwrap().manifest.runtime, Runtime::Skills);
    let mut remote = document(Some("remote_mcp"), None);
    remote["x-wonton"]["remote_mcp"] = json!({"url": "https://mcp.example.com/mcp"});
    assert_eq!(
        parse(&remote).unwrap().manifest.runtime,
        Runtime::RemoteMcp
    );
    for parsed in [skills, remote] {
        for forbidden in [json!({}), services.clone(), Value::Null, json!([])] {
            let mut doc = parsed.clone();
            doc["services"] = forbidden;
            assert!(matches!(parse(&doc), Err(PackageError::Service { .. })));
        }
    }
    // 远端形态必须写 `remote_mcp`。
    assert!(matches!(
        parse(&document(Some("remote_mcp"), None)),
        Err(PackageError::Manifest(_))
    ));
    assert!(matches!(
        parse(&document(Some("future"), None)),
        Err(PackageError::Manifest(_))
    ));
}

fn agent_mount(mount: &str) -> Value {
    let mut doc = document(None, Some(json!({"agent": {"image": "agent"}})));
    doc["x-wonton"]["kind"] = "agent".into();
    doc["x-wonton"]["agent"] = json!({"protocol": "acp", "skills": {"mount": mount}});
    doc
}

#[test]
fn skills_mount_must_be_an_agent_owned_absolute_non_system_directory() {
    for mount in [
        "/etc/codex/skills",
        "/opt/wonton/skills",
        "/proc-safe/skills",
        "/opt/skills..backup",
    ] {
        let parsed = parse(&agent_mount(mount)).unwrap();
        assert_eq!(parsed.manifest.agent.unwrap().skills.unwrap().mount, mount);
    }
    for mount in [
        "",
        "/",
        "//",
        "/./",
        "skills",
        "~/.agents/skills",
        "/opt/../skills",
        "/../skills",
        "/proc",
        "/proc/skills",
        "/sys",
        "/sys/skills",
        "/dev",
        "/dev/skills",
        "/./proc/skills",
    ] {
        assert!(
            matches!(parse(&agent_mount(mount)), Err(PackageError::Manifest(_))),
            "{mount}"
        );
    }
    let mut app = agent_mount("/opt/wonton/skills");
    app["x-wonton"]["kind"] = "app".into();
    assert!(matches!(parse(&app), Err(PackageError::Manifest(_))));
}

#[tokio::test]
async fn staged_runtime_rejection_keeps_credentials_and_install_state_empty() {
    let (_docker, engine) = super::super::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = std::sync::Arc::new(super::manager::manager_with(engine, &dir));
    let mut doc = document(Some("skills"), None);
    doc["x-wonton"]["secrets"] = json!({"api_key": {"input": {"title": {"en": "API key"}}}});
    let package = parse(&doc).unwrap();
    let id = package.manifest.id.clone();
    let digest = package.digest();
    let source = manager.staged.stage(package);
    let request = serde_json::from_value(json!({
        "source": source, "digest": digest, "grant": [], "secrets": {"api_key": "not-stored"}
    }))
    .unwrap();
    let error = manager.install_source(&request).await.unwrap_err();
    assert!(matches!(
        error,
        super::super::InstallError::UnsupportedRuntime(_)
    ));
    assert_eq!(error.kind(), "unsupported");
    assert!(manager.record_for_test(&id).is_none());
    assert!(manager.store().secrets().load(&id).unwrap().is_none());
    assert!(manager.list().is_empty());
}

#[tokio::test]
async fn scope_choices_only_offer_a_selection_between_multiple_bots() {
    let (_docker, engine) = super::super::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::manager::manager_with(engine, &dir);
    let package = parse(&document(None, Some(json!({"web": {"image": "nginx"}})))).unwrap();
    let id = super::super::PluginId::parse("app.agent").unwrap();
    let keeper = wonton_protocol::BotKeeper::User {
        username: "owner".into(),
    };
    assert!(
        manager
            .preview_install(&package, &super::langs())
            .await
            .unwrap()
            .scope_choices
            .is_empty()
    );
    manager.bot_records_for_test(&id, &[("default", "@agent:test", keeper.clone(), "token")]);
    assert!(
        manager
            .preview_install(&package, &super::langs())
            .await
            .unwrap()
            .scope_choices
            .is_empty()
    );
    manager.bot_records_for_test(
        &id,
        &[
            ("default", "@agent:test", keeper.clone(), "token"),
            ("alice", "@agent-alice:test", keeper.clone(), "alice-token"),
        ],
    );
    assert_eq!(
        manager
            .preview_install(&package, &super::langs())
            .await
            .unwrap()
            .scope_choices,
        ["alice", "default"].map(|bot| wonton_protocol::BotRef {
            plugin_id: id.to_string(),
            bot: bot.into()
        })
    );
    manager.bot_records_for_test(&id, &[("default", "@agent:test", keeper, "token")]);
    assert!(
        manager
            .preview_install(&package, &super::langs())
            .await
            .unwrap()
            .scope_choices
            .is_empty()
    );
}
