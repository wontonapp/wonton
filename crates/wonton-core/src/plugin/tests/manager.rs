//! 插件管理器对 Docker 的副作用：用假 Docker 验证安装、核对、卸载和回滚。

use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use serde_json::json;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::image_worker::ImageWorker;
use super::super::languages::Languages;
use super::super::lifecycle::LifecycleConfig;
use super::super::names::{Instance, container_name};
use super::super::package_files::PackageFiles;
use super::super::store::{InstalledRecord, Store};
use super::super::{InstallError, PluginId, PluginManager, PluginPackage, PluginStatus};
use super::egress::serve;
use super::{env_of_container, example, package};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::owner::OwnerStore;

/// 核心在容器里（`WONTON_CORE_CONTAINER`）的管理器，其余同 [`manager_with`]。
pub(super) fn manager_in_container(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    manager_for(
        engine,
        dir,
        Some("wonton-core".to_owned()),
        None,
        Instance::default(),
        ImageWorker::in_process(),
    )
}

/// 用给定 Docker 的管理器，核心直接跑在宿主上。PUID:PGID 取测试目录的属主，准备应用数据目录时不需要
/// chown 权限。
pub(super) fn manager_with(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    manager_with_images(engine, dir, ImageWorker::in_process())
}

/// 用给定的处理图片的进程，其余同 [`manager_with`]。
pub(super) fn manager_with_images(
    engine: Engine,
    dir: &tempfile::TempDir,
    images: ImageWorker,
) -> PluginManager {
    manager_for(engine, dir, None, None, Instance::default(), images)
}

/// 实例名是 `instance` 的核心（`WONTON_INSTANCE`）的管理器，其余同 [`manager_with`]。
pub(super) fn manager_in_instance(
    engine: Engine,
    dir: &tempfile::TempDir,
    instance: &str,
) -> PluginManager {
    let instance = Instance::parse(Some(instance)).unwrap();
    manager_for(engine, dir, None, None, instance, ImageWorker::in_process())
}

/// 核心直接跑在宿主上、配了身份提供方的管理器。
pub(super) fn manager_with_oidc(
    engine: Engine,
    dir: &tempfile::TempDir,
    oidc: Hydra,
) -> PluginManager {
    let images = ImageWorker::in_process();
    manager_for(engine, dir, None, Some(oidc), Instance::default(), images)
}

fn manager_for(
    engine: Engine,
    dir: &tempfile::TempDir,
    core_container: Option<String>,
    oidc: Option<Hydra>,
    instance: Instance,
    images: ImageWorker,
) -> PluginManager {
    let owner = fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container,
        tz: "UTC".into(),
        puid: owner.uid(),
        pgid: owner.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: OwnerStore::for_test(
            dir.path(),
            &crate::owner::Owner {
                username: "owner".into(),
                display_name: "云吞".into(),
                email: Some("owner@example.com".into()),
            },
        ),
        gateway_port: 8080,
        plugin_slice: None,
        instance,
    };
    let store = Store::open(dir.path().join("plugins")).unwrap();
    PluginManager::new(engine, store, HttpClient::new(), oidc, cfg, images)
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
    // 记录的位置被一个目录占着：写记录必然失败，以 root 运行也一样。
    fs::create_dir_all(dir.path().join("plugins/app.test.json")).unwrap();
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
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            id: good.clone(),
            version: "1.0.0".into(),
            source: app("app.good"),
            files: Default::default(),
            limits: Default::default(),
            granted_permissions: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
            origin: None,
            written_by: None,
            ask_every_update: false,
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
        m.install_package(example("whoami"), &extra).await,
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
    let err = m.install_package(example("whoami"), &[]).await.unwrap_err();
    assert_eq!(err.kind(), "docker_unavailable", "{err}");
    assert!(m.list().is_empty());
    let whoami = example("whoami");
    let system = PluginPackage::with_files(
        &whoami.source.replace("kind: app", "kind: system"),
        whoami.files().clone(),
    )
    .unwrap();
    assert_eq!(
        m.install_package(system, &[]).await.unwrap_err().kind(),
        "unsupported"
    );
    let id = super::super::PluginId::parse("app.whoami").unwrap();
    assert_eq!(
        m.uninstall(&id, true).await.unwrap_err().kind(),
        "not_installed"
    );
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_fails_the_install_with_dockers_reason() {
    let (docker, engine) = FakeDocker::start().await;
    docker.fail_pulls(Some("manifest unknown"));
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let err = m.install_package(example("whoami"), &[]).await.unwrap_err();
    assert_eq!(err.kind(), "image_pull_failed", "{err}");
    assert!(m.list().is_empty());
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
}

/// 网关代登录的插件：代加一个请求头和一个 Cookie，Cookie 的值是核心生成的密钥。
fn proxy_app(port: u16) -> String {
    package(
        "  web:\n    image: nginx",
        &format!(
            "  webui:\n    service: web\n    port: {port}\n    sso: proxy\n    proxy:\n      \
             headers: {{Remote-User: '{{user}}'}}\n      cookies: {{app_web: '{{secret:web_key}}'}}\n  \
             secrets: {{web_key: {{generate: {{bytes: 32}}}}}}\n  permissions: []"
        ),
    )
}

#[tokio::test]
async fn proxy_sso_plugins_get_their_credentials_from_their_own_secrets() {
    // 假 Docker 把插件容器的地址报成 127.0.0.1，安装最后一步检查的网页就是这个本机服务。
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);
    let compose = proxy_app(web.port());
    let preview = m
        .preview_install(
            &PluginPackage::parse(&compose).unwrap(),
            &Languages::default(),
        )
        .await
        .unwrap();
    assert!(
        preview.notices.iter().any(|n| n.text.contains("自动登录")),
        "安装确认页上显示会自动登录：{:?}",
        preview.notices
    );
    m.install(&compose, &[]).await.unwrap();
    let owner = crate::owner::Owner {
        username: "owner".into(),
        display_name: "主人".into(),
        email: Some("owner@wonton.localhost".into()),
    };
    let auth = m.route("app-test").unwrap().proxy.unwrap();
    let header = auth.headers(&owner).unwrap();
    assert_eq!(
        (header[0].0.as_str(), header[0].1.as_str()),
        ("remote-user", "owner")
    );
    let cookies = auth.cookies(&owner).unwrap();
    let (name, key) = (&cookies[0].0, cookies[0].1.clone());
    assert_eq!((*name, key.len()), ("app_web", 64), "32 字节的十六进制");
    assert_eq!(auth.secrets(), [key.as_str()]);
    // 核心重启后从密钥库读回同一个值。
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    let auth = restarted.route("app-test").unwrap().proxy.unwrap();
    assert_eq!(auth.cookies(&owner).unwrap()[0].1, key);
}

#[tokio::test]
async fn sso_plugins_that_ask_for_the_identity_get_the_owner_name_and_email() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      \
         ADMIN: ${WONTON_OWNER_NAME}\n      MAIL: ${WONTON_OWNER_EMAIL}",
        &format!(
            "  webui:\n    service: web\n    port: {}\n    sso: proxy\n    proxy:\n      \
             headers: {{Remote-User: '{{user}}'}}\n  permissions: [owner.identity]",
            web.port()
        ),
    );
    m.install(&compose, &["owner.identity".parse().unwrap()])
        .await
        .unwrap();
    let env = env_of_container(&docker, "wonton-app-test--web");
    assert_eq!(env["ADMIN"], "owner");
    assert_eq!(env["MAIL"], "owner@example.com");
}

#[test]
fn plugins_without_sso_cannot_reference_the_owner() {
    let compose = package(
        "  web:\n    image: nginx\n    environment: {MAIL: '${WONTON_OWNER_EMAIL}'}",
        "  webui: {service: web, port: 80}\n  permissions: []",
    );
    let pkg = PluginPackage::parse(&compose).unwrap();
    let err = super::super::render::dry_run(&pkg).unwrap_err().to_string();
    assert!(err.contains("WONTON_OWNER_EMAIL"), "{err}");
}

#[test]
fn proxy_sso_templates_use_known_placeholders_and_long_generated_secrets() {
    let parse = |headers: &str, cookies: &str, secrets: &str| {
        let manifest = format!(
            "  webui:\n    service: web\n    port: 80\n    sso: proxy\n    proxy:\n      \
             headers: {headers}\n      cookies: {cookies}\n  secrets: {secrets}\n  permissions: []"
        );
        PluginPackage::parse(&package("  web:\n    image: nginx", &manifest))
    };
    let key = "{web_key: {generate: {bytes: 32}}}";
    let ok = parse(
        "{Remote-User: '{user} <{email}>'}",
        "{app_web: 'v1.{secret:web_key}'}",
        key,
    );
    assert!(ok.is_ok(), "{ok:?}");
    for (headers, cookies, secrets) in [
        // 不认识的占位符、不配对的括号。
        ("{X-A: '{who}'}", "{}", key),
        ("{X-A: '{user'}", "{}", key),
        ("{X-A: 'user}'}", "{}", key),
        // 只能引用核心生成、至少 16 字节的密钥。
        ("{}", "{a: '{secret:nope}'}", key),
        (
            "{}",
            "{a: '{secret:web_key}'}",
            "{web_key: {generate: {bytes: 8}}}",
        ),
        (
            "{}",
            "{a: '{secret:web_key}'}",
            "{web_key: {collect: {service: web, path: /k}}}",
        ),
        // Cookie 的值里不能放用户名这类可能带分隔符的字段，也不能有空白；请求头的值里不能有控制字符。
        ("{}", "{a: '{name}'}", key),
        ("{}", "{a: 'v {secret:web_key}'}", key),
        ("{X-A: \"a\\u0001b\"}", "{}", key),
        // 网关自己管的头、逐跳头和保留的 Cookie 名。
        ("{Host: x}", "{}", key),
        ("{X-Forwarded-For: x}", "{}", key),
        ("{Cookie: x}", "{}", key),
        ("{}", "{wonton_app: x}", key),
        ("{}", "{'a;b': x}", key),
    ] {
        assert!(
            matches!(
                parse(headers, cookies, secrets),
                Err(super::super::package::PackageError::Manifest(_))
            ),
            "{headers} {cookies} {secrets}"
        );
    }
}

#[tokio::test]
async fn an_uninstall_finishes_after_its_request_goes_away() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    m.install(&app("app.test"), &[]).await.unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let hold = docker.hold(&format!(
        "DELETE /containers/{}",
        container_name("app-test", "web")
    ));
    let request = tokio::spawn({
        let (m, id) = (m.clone(), id.clone());
        async move { m.uninstall_detached(id, true).await }
    });
    hold.reached().await;
    // 请求断开时 hyper 丢弃处理它的 future。
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    // 卸载还在进行：再来一次如实回答，不另起一次。
    let err = m.uninstall_detached(id.clone(), true).await.unwrap_err();
    assert_eq!(err.kind(), "conflict");
    assert_eq!(m.list()[0].status, PluginStatus::Uninstalling);

    hold.release();
    let mut changes = m.changes();
    tokio::time::timeout(
        Duration::from_secs(5),
        changes.wait_for(|_| m.list().is_empty()),
    )
    .await
    .expect("请求断开后卸载照样做完")
    .unwrap();
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert!(docker.networks().is_empty(), "{:?}", docker.networks());
}
