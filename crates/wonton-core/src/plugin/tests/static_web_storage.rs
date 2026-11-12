//! 静态网页的数据存储（ADR-0080 第 6 节）：装的时候建数据目录、提供方读写挂上，升级不动数据，
//! 卸载默认留下、彻底删除才删，留下的数据再装时醒目提示；`requires` 时没有提供方装不上、提供方卸不掉。

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use serde_json::{Value, json};

use super::super::PluginPackage;
use super::super::fake_docker::FakeDocker;
use super::super::manifest::PluginId;
use super::super::names::container_name;
use super::super::package::static_files::StaticFiles;
use super::super::{PluginManager, PluginStatus};
use super::manager::manager_with;

const TODO: &str = "io.github.alice.todo";
const PROVIDER: &str = "app.wonton.static-host";

/// 和官方静态托管插件一样提供两项能力（`kind: app` 才能由用户装；核心对系统插件不做特殊处理）。
fn provider(storage: bool) -> PluginPackage {
    let data = if storage {
        "    web_storage:\n      service: storage\n      port: 8081\n      mount: /srv/data\n"
    } else {
        ""
    };
    let source = format!(
        "services:\n  static:\n    image: caddy\n  storage:\n    image: dav\nx-wonton:\n  spec: '0.12'\n  \
         id: {PROVIDER}\n  version: 1.0.0\n  kind: app\n  title: {{en: Host}}\n  \
         description: {{en: Host}}\n  permissions: []\n  provides:\n    static_web:\n      \
         service: static\n      port: 8080\n      mount: /srv/static\n{data}"
    );
    PluginPackage::parse(&source, &super::langs()).unwrap()
}

fn todo(version: &str, storage: Value) -> PluginPackage {
    let mut doc = json!({"x-wonton": {
        "spec": "0.12", "id": TODO, "version": version, "kind": "app", "runtime": "static_web",
        "title": {"en": "To-do"}, "description": {"en": "To-do"}, "permissions": []
    }});
    for (key, value) in storage.as_object().unwrap() {
        doc["x-wonton"][key] = value.clone();
    }
    let files: StaticFiles = [("index.html".to_owned(), version.as_bytes().to_vec())].into();
    PluginPackage::parse(&doc.to_string(), &super::langs())
        .unwrap()
        .with_static(files, &super::langs())
        .unwrap()
}

fn requires() -> Value {
    json!({"requires": ["web_storage"]})
}

fn id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap()
}

async fn with_provider(storage: bool) -> (FakeDocker, tempfile::TempDir, Arc<PluginManager>) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install_package(provider(storage), &[], super::langs())
        .await
        .unwrap();
    (docker, dir, manager)
}

fn data(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("hosted/web_storage").join(TODO)
}

// 提供方的数据服务读写挂上数据存储的总目录。
#[tokio::test]
async fn the_storage_service_gets_the_data_directory_read_write() {
    let (docker, dir, _manager) = with_provider(true).await;
    let created = docker
        .created(&container_name("app-wonton-static-host", "storage"))
        .unwrap();
    let mounts = created["HostConfig"]["Mounts"].as_array().unwrap();
    let data = mounts.iter().find(|m| m["Target"] == "/srv/data").unwrap();
    assert_eq!(data["ReadOnly"], json!(false));
    let source = dir.path().join("hosted/web_storage");
    assert_eq!(data["Source"], json!(source.display().to_string()));
}

// 一生：装时建好属于主人用户的空目录；升级不动数据；卸载默认留下，再装时提示并接着用；彻底删除才删。
#[tokio::test]
async fn data_survives_upgrades_and_plain_uninstalls_but_not_a_purge() {
    let (_docker, dir, manager) = with_provider(true).await;
    manager
        .install_package(todo("1.0.0", requires()), &[], super::langs())
        .await
        .unwrap();
    let meta = fs::metadata(data(&dir)).unwrap();
    let owner = fs::metadata(dir.path()).unwrap();
    assert_eq!((meta.uid(), meta.gid()), (owner.uid(), owner.gid()));
    let record = manager.record_for_test(&id(TODO)).unwrap();
    assert_eq!(record.hosted, ["static_web", "web_storage"]);
    fs::write(data(&dir).join("todo.json"), "[1]").unwrap();

    // 新版本不再要数据存储：数据留着，账本也还记着。
    manager
        .upgrade_upload(
            &id(TODO),
            todo("1.1.0", json!({})),
            Vec::new(),
            Vec::new(),
            false,
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(fs::read(data(&dir).join("todo.json")).unwrap(), b"[1]");
    let record = manager.record_for_test(&id(TODO)).unwrap();
    assert!(record.hosted.iter().any(|c| c == "web_storage"));

    let report = manager
        .uninstall(&id(TODO), false, &super::langs())
        .await
        .unwrap();
    assert!(!report.data_removed);
    assert!(data(&dir).join("todo.json").is_file());
    assert!(!dir.path().join("hosted/static_web").join(TODO).exists());
    let notice = manager
        .retained_data(&id(TODO), &super::langs())
        .await
        .unwrap();
    assert!(notice.is_some_and(|line| line.emphasis));

    manager
        .install_package(todo("1.0.0", requires()), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(fs::read(data(&dir).join("todo.json")).unwrap(), b"[1]");
    let report = manager
        .uninstall(&id(TODO), true, &super::langs())
        .await
        .unwrap();
    assert!(report.data_removed);
    assert!(!data(&dir).exists());
}

// requires：没有数据存储的提供方装不上，什么都不留；wants：照装，也建好目录，提供方以后装上就能用。
#[tokio::test]
async fn requires_needs_a_provider_and_wants_does_not() {
    let (_docker, dir, manager) = with_provider(false).await;
    let error = manager
        .install_package(todo("1.0.0", requires()), &[], super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict", "{error}");
    assert!(error.to_string().contains("web_storage"), "{error}");
    assert!(!data(&dir).exists());
    assert!(manager.record_for_test(&id(TODO)).is_none());

    manager
        .install_package(
            todo("1.0.0", json!({"wants": ["web_storage"]})),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert!(data(&dir).is_dir());
    let listed = manager.list();
    let todo = listed.iter().find(|p| p.id == id(TODO)).unwrap();
    assert_eq!(todo.status, PluginStatus::Running);
}

// 卸载提供方：有网页 requires 它的数据存储时拦下；只是 wants 的不拦。
#[tokio::test]
async fn the_provider_stays_while_a_page_requires_its_storage() {
    let (_docker, _dir, manager) = with_provider(true).await;
    manager
        .install_package(todo("1.0.0", requires()), &[], super::langs())
        .await
        .unwrap();
    let error = manager
        .uninstall(&id(PROVIDER), false, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict", "{error}");
    assert!(error.to_string().contains("To-do"), "{error}");
}

// 数据存储只给静态网页：容器插件写了照旧报不支持。
#[tokio::test]
async fn container_plugins_cannot_ask_for_web_storage() {
    let (_docker, _dir, manager) = with_provider(true).await;
    let source = "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.12'\n  id: app.test\n  \
                  version: 1.0.0\n  kind: app\n  title: {en: T}\n  description: {en: T}\n  \
                  permissions: []\n  requires: [web_storage]\n";
    match PluginPackage::parse(source, &super::langs()) {
        Err(_) => {}
        Ok(package) => {
            let error = manager
                .install_package(package, &[], super::langs())
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "unsupported", "{error}");
        }
    }
}
