//! 插件管理器对 Docker 的副作用：用假 Docker 验证安装、核对、卸载和回滚。

use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use serde_json::json;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::image_worker::ImageWorker;
use super::super::lifecycle::LifecycleConfig;
use super::super::names::{Instance, container_name};
use super::super::package_files::PackageFiles;
use super::super::store::{InstalledRecord, Store};
use super::super::upstreams::Endpoint;
use super::super::{InstallError, PluginId, PluginManager, PluginPackage, PluginStatus};
use super::egress::serve;
use super::{env_of_container, example, package};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::owner::OwnerStore;

mod configs;
mod failures;
mod mcp_headers;
mod permissions;
mod records;
mod sso;
mod uninstall;

/// 核心在容器里（`WONTON_CORE_CONTAINER`）的管理器，其余同 [`manager_with`]。
pub(super) fn manager_in_container(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
    manager_for(
        engine,
        dir,
        &dir.path().join("apps"),
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
    manager_for(
        engine,
        dir,
        &dir.path().join("apps"),
        None,
        None,
        Instance::default(),
        images,
    )
}

/// 实例名是 `instance` 的核心（`WONTON_INSTANCE`）的管理器，其余同 [`manager_with`]。
pub(super) fn manager_in_instance(
    engine: Engine,
    dir: &tempfile::TempDir,
    instance: &str,
) -> PluginManager {
    let instance = Instance::parse(Some(instance)).unwrap();
    manager_for(
        engine,
        dir,
        &dir.path().join("apps"),
        None,
        None,
        instance,
        ImageWorker::in_process(),
    )
}

/// 核心直接跑在宿主上、配了身份提供方的管理器。
pub(super) fn manager_with_oidc(
    engine: Engine,
    dir: &tempfile::TempDir,
    oidc: Hydra,
) -> PluginManager {
    let images = ImageWorker::in_process();
    manager_for(
        engine,
        dir,
        &dir.path().join("apps"),
        None,
        Some(oidc),
        Instance::default(),
        images,
    )
}

/// 中继套接字目录在 `relay_dir` 的管理器，其余同 [`manager_with`]。
pub(super) fn manager_with_relay_dir(
    engine: Engine,
    dir: &tempfile::TempDir,
    relay_dir: &std::path::Path,
) -> PluginManager {
    manager_for_opts(
        engine,
        dir,
        &dir.path().join("apps"),
        None,
        None,
        Instance::default(),
        ImageWorker::in_process(),
        Some(relay_dir.to_owned()),
    )
}

fn manager_for(
    engine: Engine,
    dir: &tempfile::TempDir,
    apps_dir: &std::path::Path,
    core_container: Option<String>,
    oidc: Option<Hydra>,
    instance: Instance,
    images: ImageWorker,
) -> PluginManager {
    manager_for_opts(
        engine,
        dir,
        apps_dir,
        core_container,
        oidc,
        instance,
        images,
        None,
    )
}

/// [`manager_for`]，可以另指中继套接字目录。
#[allow(clippy::too_many_arguments)]
fn manager_for_opts(
    engine: Engine,
    dir: &tempfile::TempDir,
    apps_dir: &std::path::Path,
    core_container: Option<String>,
    oidc: Option<Hydra>,
    instance: Instance,
    images: ImageWorker,
    relay_dir: Option<PathBuf>,
) -> PluginManager {
    let owner = fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: apps_dir.to_owned(),
        shares_dir: dir.path().join("shares"),
        // 默认不是 `/run`：测试要能同时跑，套接字得落在各自的临时目录里。
        relay_dir: relay_dir.unwrap_or_else(|| dir.path().join("relay")),
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
pub(super) fn app(id: &str) -> String {
    package(
        "  web:\n    image: nginx\n    volumes: ['${WONTON_APP_DATA}/data:/data']",
        "  permissions: []",
    )
    .replace("id: app.test", &format!("id: {id}"))
}

/// 核心在容器里时，网页和 MCP 上游连服务的稳定域名 `服务名.<slug>.wonton.internal`（上游看到的 Host 就是它，
/// 作者经 `WONTON_SERVICE_DOMAIN` 把它写进白名单）。
#[tokio::test]
async fn in_a_container_the_webui_and_mcp_upstreams_use_the_service_host() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let compose = package(
        "  web:\n    image: nginx",
        "  permissions: []\n  webui: {service: web, port: 8123, sso: none}\n  \
         mcp: {service: web, port: 9000, transport: streamable-http}",
    );
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .save(&InstalledRecord {
            lifecycle: Default::default(),
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            bundled: false,
            id: PluginId::parse("app.test").unwrap(),
            version: "1.0.0".into(),
            source: compose,
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
    docker.add_container(&container_name("app-test", "web"), "app.test");
    docker.add_container("wonton-core", "app.test");
    docker.add_network(
        "wonton-app-test",
        "app.test",
        json!({"Internal": true, "IPAM": {"Config": [{
            "Subnet": "172.30.0.0/24", "IPRange": "172.30.0.0/25", "Gateway": "172.30.0.1",
        }]}, "Options": {
            "com.docker.network.bridge.gateway_mode_ipv4": "isolated",
            "com.docker.network.bridge.gateway_mode_ipv6": "isolated",
        }}),
    );
    let m = manager_in_container(engine, &dir);
    m.restore().await.unwrap();
    assert_eq!(
        m.route("app-test", "/").unwrap().upstream,
        Some(Endpoint::Tcp(
            "web.app-test.wonton.internal:8123".to_owned()
        ))
    );
    assert_eq!(
        m.mcp_servers()[0].url,
        "http://web.app-test.wonton.internal:9000/mcp"
    );
}
