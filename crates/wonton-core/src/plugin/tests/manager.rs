//! 插件管理器对 Docker 的副作用：用假 Docker 验证安装、核对、卸载和回滚。

use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::lifecycle::LifecycleConfig;
use super::super::package_files::PackageFiles;
use super::super::render::container_name;
use super::super::store::{InstalledRecord, Store};
use super::super::{InstallError, PluginId, PluginManager, PluginPackage, PluginStatus};
use super::{WHOAMI, package};
use crate::config::PublicUrls;
use crate::http::HttpClient;

/// 用给定 Docker 的管理器。PUID:PGID 取测试目录的属主，准备应用数据目录时不需要 chown 权限。
fn manager_with(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    let owner = fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner.uid(),
        pgid: owner.gid(),
        install_timeout: Duration::from_secs(5),
    };
    let store = Store::open(dir.path().join("plugins")).unwrap();
    PluginManager::new(engine, store, HttpClient::new(), None, cfg)
}

/// Docker 不可达的管理器：只用来验证不依赖 Docker 的规则，以及 Docker 不可用时的报错。
fn manager(dir: &tempfile::TempDir) -> PluginManager {
    manager_with(Engine::unreachable_for_test(), dir)
}

/// 一个服务、把数据放在应用数据目录里的插件。
fn app(id: &str) -> String {
    package(
        "  web:\n    image: nginx\n    volumes: ['${WONTON_APP_DATA}/data:/data']",
        "  permissions: []",
    )
    .replace("id: app.test", &format!("id: {id}"))
}

#[tokio::test]
async fn a_failed_record_write_rolls_back_the_deployment() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    // 记录目录被换成普通文件：写记录必然失败，以 root 运行也一样。
    let records = dir.path().join("plugins");
    fs::remove_dir(&records).unwrap();
    fs::write(&records, "").unwrap();
    let err = m.install(&app("app.test"), &[]).await.unwrap_err();
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
    assert!(m.list().is_empty(), "失败的安装不能留在插件表里");
}

/// 归档里的文件：（路径，uid，gid，权限，内容）。
fn archived(archive: &[u8]) -> Vec<(String, u64, u64, u32, Vec<u8>)> {
    let mut archive = tar::Archive::new(archive);
    let entries = archive.entries().unwrap();
    entries
        .map(|entry| {
            let mut entry = entry.unwrap();
            let header = entry.header().clone();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            let path = entry.path().unwrap().display().to_string();
            let (uid, gid) = (header.uid().unwrap(), header.gid().unwrap());
            (path, uid, gid, header.mode().unwrap(), data)
        })
        .collect()
}

#[tokio::test]
async fn configs_are_written_after_create_and_before_start() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let compose = format!(
        "configs:\n  app:\n    file: app.yaml\n{}",
        package(
            "  web:\n    image: nginx\n    configs:\n      - source: app\n        \
             target: /etc/app.yaml\n        uid: 101\n        gid: '102'",
            "  permissions: []",
        )
    );
    let files: PackageFiles = [("app.yaml".to_owned(), b"level: info\n".to_vec())].into();
    let package = PluginPackage::with_files(&compose, files.clone()).unwrap();
    m.install_package(package, &[]).await.unwrap();

    let web = container_name("app-test", "web");
    let writes = docker.writes();
    let position = |call: String| {
        let found = writes.iter().position(|w| *w == call);
        found.unwrap_or_else(|| panic!("缺少 {call}：{writes:?}"))
    };
    assert!(
        position(format!("PUT /containers/{web}/archive"))
            < position(format!("POST /containers/{web}/start")),
        "配置要在启动前写进容器：{writes:?}"
    );
    assert_eq!(
        archived(&docker.archive(&web).unwrap()),
        [(
            "etc/app.yaml".to_owned(),
            101,
            102,
            0o444,
            b"level: info\n".to_vec()
        )]
    );

    // 插件包的文件随记录保存，核心重启后用它们恢复。
    let records = Store::open(dir.path().join("plugins")).unwrap();
    let record = records.load_all().unwrap().remove(0).record.unwrap();
    assert_eq!(record.files, files);
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
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
            id: good.clone(),
            version: "1.0.0".into(),
            source: app("app.good"),
            files: Default::default(),
            granted_permissions: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
        })
        .unwrap();
    docker.add_container(&container_name("app-good", "web"), "app.good");
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
    // 主人能卸载这个异常的插件，它的记录文件随之删除。
    let id = PluginId::parse("app.broken").unwrap();
    m.uninstall(&id, false).await.unwrap();
    assert!(!broken.exists());
    assert_eq!(m.list().len(), 1);
}

#[tokio::test]
async fn every_declared_permission_must_be_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir);
    let share = package(
        "  web:\n    image: nginx",
        "  permissions: ['storage.share.photos:read', 'notify']",
    );
    let photos = vec!["storage.share.photos:read".parse().unwrap()];
    let err = m.install(&share, &photos).await.unwrap_err();
    assert!(
        matches!(err, InstallError::PermissionNotGranted(ref p) if p == &["notify"]),
        "{err}"
    );
    let extra = vec!["models".parse().unwrap()];
    assert!(matches!(
        m.install(WHOAMI, &extra).await,
        Err(InstallError::Invalid(_))
    ));
    let all = vec![
        "storage.share.photos:read".parse().unwrap(),
        "notify".parse().unwrap(),
    ];
    let err = m.install(&share, &all).await.unwrap_err();
    assert_eq!(
        err.kind(),
        "unsupported",
        "M0 不能兑现 notify，必须如实拒绝：{err}"
    );
    assert!(m.list().is_empty(), "被拒的安装不能留下记录");
}

#[tokio::test]
async fn docker_outage_is_reported_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir);
    let err = m.install(WHOAMI, &[]).await.unwrap_err();
    assert_eq!(err.kind(), "docker_unavailable", "{err}");
    assert!(m.list().is_empty());
    let system = WHOAMI.replace("kind: app", "kind: system");
    assert_eq!(
        m.install(&system, &[]).await.unwrap_err().kind(),
        "unsupported"
    );
    let id = super::super::PluginId::parse("app.whoami").unwrap();
    assert_eq!(
        m.uninstall(&id, true).await.unwrap_err().kind(),
        "not_installed"
    );
}
