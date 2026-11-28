//! 记录写入失败和记录损坏：回滚部署，坏记录只影响它自己。

use std::fs;

use serde_json::json;

use super::super::super::fake_docker::FakeDocker;
use super::super::super::names::container_name;
use super::super::super::store::{InstalledRecord, Store};
use super::super::super::{PluginId, PluginStatus};
use super::super::{langs, package};
use super::{app, manager_with};

#[tokio::test]
async fn a_failed_record_write_rolls_back_the_deployment() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    // 记录的位置被一个目录占着：写记录必然失败，以 root 运行也一样。
    fs::create_dir_all(dir.path().join("plugins/app.test.json")).unwrap();
    let err = m.install(&app("app.test"), &[], langs()).await.unwrap_err();
    assert_eq!(err.kind(), "storage", "{err}");
    let started = format!(
        "POST /containers/{}/start",
        container_name("app-test", "web")
    );
    assert!(
        docker.writes().contains(&started),
        "部署本身应已完成：{:?}",
        docker.writes()
    );
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert!(docker.networks().is_empty(), "{:?}", docker.networks());
    assert!(!dir.path().join("apps/app-test").exists());
    let failed = m
        .installed_plugin(&PluginId::parse("app.test").unwrap(), &langs())
        .unwrap();
    assert_eq!(
        serde_json::to_value(failed).unwrap()["install_failure"]["kind"],
        "storage"
    );
}

#[tokio::test]
async fn a_broken_record_degrades_only_its_own_plugin() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let good = PluginId::parse("app.good").unwrap();
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .save(&InstalledRecord {
            lifecycle: Default::default(),
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            bundled: false,
            id: good.clone(),
            version: "1.0.0".into(),
            source: app("app.good"),
            files: Default::default(),
            limits: Default::default(),
            granted_permissions: Vec::new(),
            enabled_permissions: Vec::new(),
            devices: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
            entry_label: None,
            clients_entries: Default::default(),
            setup_done_ever: false,
            origin: None,
            written_by: None,
            ask_every_update: false,
            settings: Default::default(),
            settings_changed_ms: None,
            static_files: Default::default(),
            hosted: Vec::new(),
            gpu_vendor: None,
        })
        .unwrap();
    docker.add_container(&container_name("app-good", "web"), "app.good");
    docker.add_network("wonton-app-good", "app.good", json!({"Internal": true}));
    let broken = dir.path().join("plugins/app.broken.json");
    fs::write(&broken, "{\"id\": ").unwrap();

    m.restore().await.expect("一条损坏的记录不能让核心起不来");
    let states: Vec<(String, PluginStatus)> = m
        .list()
        .into_iter()
        .map(|p| (p.id.to_string(), p.status))
        .collect();
    assert!(
        matches!(&states[..], [(b, PluginStatus::Degraded { .. }), (g, PluginStatus::Running)]
            if b == "app.broken" && g == "app.good"),
        "{states:?}"
    );
    // 坏记录不挡别的插件安装，也不参与标签防撞名。装没有网页入口的包：装好时的网页确认会真连端口，
    // CI 上 127.0.0.1:8000 没有服务，带 webui 的示例包会等满时限。
    m.install(
        &package("  web:\n    image: nginx", "  permissions: []"),
        &[],
        langs(),
    )
    .await
    .unwrap();
    assert_eq!(m.list().len(), 3);
    // 主人能卸载这个异常的插件，它的记录文件随之删除。
    let id = PluginId::parse("app.broken").unwrap();
    m.uninstall(&id, false, &langs()).await.unwrap();
    assert!(!broken.exists());
    assert_eq!(m.list().len(), 2);
}
