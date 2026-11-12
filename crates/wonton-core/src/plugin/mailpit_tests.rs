use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;

use bollard::models::ContainerCreateBody;
use serde_json::json;

use super::*;
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::mailbox::Component;
use crate::members::Members;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::Store;

async fn compose(engine: &Engine, name: &str, project: &str, service: &str) {
    engine
        .create_container(
            name,
            ContainerCreateBody {
                image: Some("mailpit-test".into()),
                labels: Some(HashMap::from([
                    ("com.docker.compose.project".into(), project.into()),
                    ("com.docker.compose.service".into(), service.into()),
                ])),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

fn manager(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    let meta = std::fs::metadata(dir.path()).unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let mailpit = Component::open(
        engine.clone(),
        "core".into(),
        dir.path(),
        "owner",
        members,
        meta.uid(),
        meta.gid(),
    )
    .unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        relay_dir: dir.path().join("relay"),
        core_container: Some("core".into()),
        tz: "UTC".into(),
        puid: meta.uid(),
        pgid: meta.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: crate::owner::OwnerStore::for_test(
            dir.path(),
            &crate::owner::Owner {
                username: "owner".into(),
                display_name: "Owner".into(),
                email: None,
            },
        ),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    PluginManager::new(
        engine,
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        cfg,
        ImageWorker::in_process(),
    )
    .with_mailpit(Some(mailpit))
}

fn source() -> &'static str {
    "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.5'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  title: {en: Test}\n  description: {en: Test}\n  permissions: [notify]\n"
}

#[tokio::test]
async fn notify_install_restore_rebuild_and_uninstall_manage_smtp_endpoint() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    compose(&engine, "core", "home", "core").await;
    compose(&engine, "mail", "home", "mailpit").await;
    compose(&engine, "foreign", "other", "mailpit").await;
    let manager = manager(engine.clone(), &dir);
    let langs = crate::plugin::languages::Languages::from_header(Some("en"));
    manager
        .install(source(), &[Permission::Notify], langs.clone())
        .await
        .unwrap();
    let network = "wonton-app-test";
    let endpoint = docker.endpoint(network, "mail").unwrap();
    assert_eq!(endpoint["Aliases"], json!(["smtp.wonton.internal"]));
    assert_eq!(endpoint["GwPriority"], -1);
    assert!(docker.endpoint(network, "foreign").is_none());
    engine.disconnect_network(network, "mail").await.unwrap();
    manager.restore().await.unwrap();
    assert!(docker.endpoint(network, "mail").is_some());
    // Compose 重建 Mailpit，不重建插件；显式巡检恢复新容器的授权端点。
    engine.remove_container("mail").await.unwrap();
    compose(&engine, "replacement", "home", "mailpit").await;
    manager.refresh_mailpit_networks().await.unwrap();
    assert!(docker.endpoint(network, "replacement").is_some());
    manager
        .uninstall(&PluginId::parse("app.test").unwrap(), false, &langs)
        .await
        .unwrap();
    let writes = docker.writes();
    let disconnect = writes
        .iter()
        .rposition(|call| call == "POST /networks/wonton-app-test/disconnect")
        .unwrap();
    let remove = writes
        .iter()
        .rposition(|call| call == "DELETE /networks/wonton-app-test")
        .unwrap();
    assert!(disconnect < remove);
    assert!(docker.containers().contains(&"replacement".into()));
}

#[tokio::test]
async fn failed_install_detaches_mailpit_before_rolling_back_network() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    compose(&engine, "core", "home", "core").await;
    compose(&engine, "mail", "home", "mailpit").await;
    let manager = manager(engine, &dir);
    docker.fail_starts(Some("injected plugin startup failure"));
    assert!(
        manager
            .install(
                source(),
                &[Permission::Notify],
                crate::plugin::languages::Languages::from_header(Some("en"))
            )
            .await
            .is_err()
    );
    assert!(docker.networks().is_empty());
    assert!(docker.containers().contains(&"mail".into()));
}

#[tokio::test]
async fn removing_notify_from_the_manifest_disconnects_mailpit_only() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    compose(&engine, "core", "home", "core").await;
    compose(&engine, "mail", "home", "mailpit").await;
    docker.add_unlabeled_network("plugin");
    let manager = manager(engine, &dir);
    let mut manifest = crate::plugin::PluginPackage::parse(
        source(),
        &crate::plugin::languages::Languages::default(),
    )
    .unwrap()
    .manifest;
    manager.attach_mailpit("plugin", &manifest).await.unwrap();
    assert!(docker.endpoint("plugin", "mail").is_some());
    manifest.permissions.clear();
    manager.attach_mailpit("plugin", &manifest).await.unwrap();
    assert!(docker.endpoint("plugin", "mail").is_none());
    assert!(docker.networks().contains(&"plugin".into()));
}
