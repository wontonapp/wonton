//! 静态网页和外部网页的装、升、卸（ADR-0080）：文件进托管目录、提供方容器只读挂上、
//! 提供方被用着时不能卸、没有提供方时装不上，外部网页不起容器。

use std::sync::Arc;

use serde_json::{Value, json};
use wonton_protocol::PluginState;

use super::super::PluginPackage;
use super::super::fake_docker::FakeDocker;
use super::super::manifest::PluginId;
use super::super::names::container_name;
use super::super::package::static_files::StaticFiles;
use super::super::{PluginManager, PluginStatus};
use super::manager::manager_with;

const SNAKE: &str = "io.github.alice.snake";
const PROVIDER: &str = "app.wonton.static-host";

/// 提供方：真实的系统插件是同样的清单，只是 `kind: system` 由安装器预装；核心对它不做特殊处理。
fn provider() -> PluginPackage {
    let source = "services:\n  static:\n    image: caddy\nx-wonton:\n  spec: '0.12'\n  \
                  id: app.wonton.static-host\n  version: 1.0.0\n  kind: app\n  \
                  title: {en: Static host}\n  description: {en: Hosting}\n  permissions: []\n  \
                  provides:\n    static_web:\n      service: static\n      port: 8080\n      \
                  mount: /srv/static\n";
    PluginPackage::parse(source, &super::langs()).unwrap()
}

fn doc(runtime: &str, version: &str) -> Value {
    json!({"x-wonton": {
        "spec": "0.12", "id": SNAKE, "version": version, "kind": "app",
        "runtime": runtime, "title": {"en": "Snake"}, "description": {"en": "Snake"},
        "permissions": [], "webui": {"path": "/"}
    }})
}

fn snake(version: &str, files: &[(&str, &str)]) -> PluginPackage {
    let files: StaticFiles = files
        .iter()
        .map(|(path, data)| ((*path).to_owned(), data.as_bytes().to_vec()))
        .collect();
    PluginPackage::parse(&doc("static_web", version).to_string(), &super::langs())
        .unwrap()
        .with_static(files, &super::langs())
        .unwrap()
}

fn id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap()
}

async fn with_provider() -> (FakeDocker, tempfile::TempDir, Arc<PluginManager>) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install_package(provider(), &[], super::langs())
        .await
        .unwrap();
    (docker, dir, manager)
}

fn site(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("hosted/static_web").join(SNAKE)
}

// 提供方的容器只读挂上整个能力目录：提供方按请求头挑子目录，自己改不了任何插件的文件。
#[tokio::test]
async fn the_provider_gets_the_capability_directory_read_only() {
    let (docker, dir, _manager) = with_provider().await;
    let created = docker
        .created(&container_name("app-wonton-static-host", "static"))
        .unwrap();
    let mounts = created["HostConfig"]["Mounts"].as_array().unwrap();
    let hosted = mounts
        .iter()
        .find(|m| m["Target"] == "/srv/static")
        .unwrap_or_else(|| panic!("{mounts:?}"));
    assert_eq!(hosted["ReadOnly"], json!(true));
    assert_eq!(hosted["Type"], json!("bind"));
    let source = dir.path().join("hosted/static_web");
    assert_eq!(hosted["Source"], json!(source.display().to_string()));
    assert!(source.is_dir(), "目录要在容器创建前就在");
}

// 装：文件进托管目录、记录只留摘要、不起容器；网关的路由指向提供方，带着核心算的 CSP。
// 升：整个目录换成新版本，旧文件不留。卸：目录跟着删。
#[tokio::test]
async fn a_static_page_lives_in_the_hosted_directory_for_its_whole_life() {
    let (docker, dir, manager) = with_provider().await;
    let containers = docker.containers();
    let page = snake("1.0.0", &[("index.html", "v1"), ("old.js", "1")]);
    manager
        .install_package(page, &[], super::langs())
        .await
        .unwrap();
    assert_eq!(docker.containers(), containers, "静态网页不起容器");
    assert_eq!(std::fs::read(site(&dir).join("index.html")).unwrap(), b"v1");
    let record = manager.record_for_test(&id(SNAKE)).unwrap();
    assert_eq!(
        record.static_files.keys().collect::<Vec<_>>(),
        ["index.html", "old.js"]
    );
    assert_eq!(record.hosted, ["static_web"]);
    let view = manager.list();
    let listed = view.iter().find(|p| p.id == id(SNAKE)).unwrap();
    assert_eq!(listed.status, PluginStatus::Running);
    let route = manager.route("io-github-alice-snake", "/").unwrap();
    assert!(route.upstream.is_some());
    let hosted = route.hosted.unwrap();
    assert!(
        hosted.csp.starts_with("default-src 'self'"),
        "{}",
        hosted.csp
    );

    let next = snake("1.1.0", &[("index.html", "v2"), ("new.js", "2")]);
    manager
        .upgrade_upload(
            &id(SNAKE),
            next,
            Vec::new(),
            Vec::new(),
            false,
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(site(&dir).join("index.html")).unwrap(), b"v2");
    assert!(site(&dir).join("new.js").is_file());
    assert!(!site(&dir).join("old.js").exists());
    let leftovers = std::fs::read_dir(dir.path().join("hosted/.staging")).map(|d| d.count());
    assert!(leftovers.is_err() || leftovers.unwrap() == 0);

    manager
        .uninstall(&id(SNAKE), false, &super::langs())
        .await
        .unwrap();
    assert!(!site(&dir).exists());
}

#[tokio::test]
async fn a_static_page_tracks_its_providers_availability() {
    let (docker, _dir, manager) = with_provider().await;
    manager
        .install_package(snake("1.0.0", &[("index.html", "x")]), &[], super::langs())
        .await
        .unwrap();
    let status = || {
        manager
            .installed_plugin(&id(SNAKE), &super::langs())
            .unwrap()
            .status
    };
    assert!(matches!(status(), PluginState::Running { .. }));

    let service = container_name("app-wonton-static-host", "static");
    docker.set_state(
        &service,
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    manager.watch_containers().await;
    assert!(matches!(status(), PluginState::Degraded { .. }));

    docker.set_state(
        &service,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.watch_containers().await;
    assert!(matches!(status(), PluginState::Running { .. }));
}

// 卸提供方会让它托管的网页打不开：先卸那些网页。
#[tokio::test]
async fn the_provider_cannot_be_uninstalled_while_it_serves_pages() {
    let (_docker, _dir, manager) = with_provider().await;
    manager
        .install_package(snake("1.0.0", &[("index.html", "x")]), &[], super::langs())
        .await
        .unwrap();
    let error = manager
        .uninstall(&id(PROVIDER), false, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict", "{error}");
    assert!(error.to_string().contains("Snake"), "{error}");
    manager
        .uninstall(&id(SNAKE), false, &super::langs())
        .await
        .unwrap();
    manager
        .uninstall(&id(PROVIDER), false, &super::langs())
        .await
        .unwrap();
}

// 没装提供方：如实说缺哪个能力，什么都不留。
#[tokio::test]
async fn without_a_provider_a_static_page_is_refused_and_leaves_nothing() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let error = manager
        .install_package(snake("1.0.0", &[("index.html", "x")]), &[], super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict", "{error}");
    assert!(error.to_string().contains("static_web"), "{error}");
    assert!(manager.list().is_empty());
    assert!(!site(&dir).exists());
}

// 核心重启时托管目录不见了：如实显示为降级，不当成正常。
#[tokio::test]
async fn a_missing_hosted_directory_is_degraded_after_a_restart() {
    let (docker, dir, manager) = with_provider().await;
    manager
        .install_package(snake("1.0.0", &[("index.html", "x")]), &[], super::langs())
        .await
        .unwrap();
    std::fs::remove_dir_all(site(&dir)).unwrap();
    let restarted = manager_with(docker.engine().await, &dir);
    restarted.restore().await.unwrap();
    let listed = restarted.list();
    let page = listed.iter().find(|p| p.id == id(SNAKE)).unwrap();
    assert!(
        matches!(page.status, PluginStatus::Degraded { .. }),
        "{:?}",
        page.status
    );
}

// 外部网页只存地址：不起容器、不要提供方。
#[tokio::test]
async fn a_link_installs_without_containers_or_a_provider() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let mut link = doc("link", "1.0.0");
    link["x-wonton"].as_object_mut().unwrap().remove("webui");
    link["x-wonton"]["link"] = json!({"url": "https://example.com/app"});
    let package = PluginPackage::parse(&link.to_string(), &super::langs()).unwrap();
    manager
        .install_package(package, &[], super::langs())
        .await
        .unwrap();
    assert!(docker.containers().is_empty());
    assert_eq!(manager.list()[0].status, PluginStatus::Running);
    let detail = manager
        .installed_detail(&id(SNAKE), &super::langs())
        .unwrap();
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(
        detail["link_url"],
        json!("https://example.com/app"),
        "{detail}"
    );
}
