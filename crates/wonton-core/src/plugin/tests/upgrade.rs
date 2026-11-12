//! 升级（ADR-0017）：查到新版本，保留数据和 Bot 的设备，只换容器；新版本装不好就退回旧版本，错误如实报。

use std::fs;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::watch;
use wonton_protocol::{InstallRequest, PluginState, PluginTrust, UpgradeRequest};

use super::super::egress::settings::StoredEgress;
use super::super::fake_docker::FakeDocker;
use super::super::github::GitHub;
use super::super::manifest::Permission;
use super::super::names::container_name;
use super::super::preinstall::Backoff;
use super::super::storefront::StoreConfig;
use super::super::storefront::index::IndexEntry;
use super::super::{InstallError, PluginId, PluginManager, PluginPackage, PluginStatus, UpgradeTo};
use super::env_of_container;
use super::github::{FakeGitHub, zh};
use super::lan::{office, on};
use super::manager::{manager_in_container, manager_with};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::start;

const REPO: &str = "acme/hello";
const SOURCE: &str = "github:acme/hello";
const GIB: u64 = 1 << 30;

/// `web` 的镜像跟着版本走，`services` 追加在后面，`manifest` 追加在清单里。
fn hello(version: &str, services: &str, manifest: &str) -> String {
    format!(
        "services:\n  web:\n    image: web:{version}\n    volumes: ['data:/data']\n{services}volumes:\n  data: {{}}\n\
         x-wonton:\n  spec: \"0.2\"\n  id: app.hello\n  version: {version}\n  kind: app\n  title: {{zh: 你好, en: Hello}}\n  \
         description: {{zh: 打个招呼, en: Say hi}}\n  release_notes: {{zh: 版本 {version} 的说明, en: Notes}}\n{manifest}"
    )
}

fn release(github: &FakeGitHub, compose: &str) -> String {
    let package = PluginPackage::parse(compose, &super::langs()).unwrap();
    let tag = format!("v{}", package.manifest.version);
    github.publish(REPO, &tag, &[("docker-compose.yml", compose)])
}

fn id() -> PluginId {
    PluginId::parse("app.hello").unwrap()
}

fn web() -> String {
    container_name("app-hello", "web")
}

fn permissions(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions
}

async fn with_github(
    in_container: bool,
) -> (
    FakeGitHub,
    FakeDocker,
    Arc<PluginManager>,
    tempfile::TempDir,
) {
    let (github, client): (FakeGitHub, GitHub) = FakeGitHub::start().await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = if in_container {
        manager_in_container(engine, &dir)
    } else {
        manager_with(engine, &dir)
    };
    (github, docker, Arc::new(m.with_github(client)), dir)
}

async fn install(m: &Arc<PluginManager>, source: &str) {
    let preview = m.preview_source(source, &zh()).await.unwrap();
    let request = InstallRequest {
        source: preview.source.clone().unwrap(),
        digest: preview.digest.clone(),
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        devices: Vec::new(),
        lan_webui: false,
    };
    m.install_source(&request).await.unwrap();
}

/// 主人在确认页上点了确认。
async fn upgrade(m: &Arc<PluginManager>) -> Result<(), InstallError> {
    let preview = m.preview_upgrade(&id(), UpgradeTo::Offered, &zh()).await?;
    let request = UpgradeRequest {
        devices: Vec::new(),
        source: None,
        digest: preview.digest,
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        lan_webui: false,
    };
    m.upgrade_source(&id(), &request, &super::langs()).await
}

fn version(m: &PluginManager) -> String {
    m.installed_plugin(&id(), &zh()).unwrap().version
}

fn image(docker: &FakeDocker, container: &str) -> String {
    docker.created(container).unwrap()["Image"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn owner_limits_survive_the_upgrade() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    m.set_limits(id(), memory_only(6 * GIB), Default::default())
        .await
        .unwrap();
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().revision, revision);

    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    let host = &docker.created(&web()).unwrap()["HostConfig"];
    assert_eq!(host["Memory"], json!(6 * GIB));
    assert_eq!(host["MemorySwap"], json!(6 * GIB));
    // 记录里没调过 CPU，compose 也没写 cpus：重建出来的容器不设 NanoCpus（默认不限）。
    assert_eq!(host["NanoCpus"], Value::Null);
    let detail = m.installed_detail(&id(), &zh()).unwrap();
    assert_eq!(detail.limits, Some(memory_only(6 * GIB)));
}

fn memory_only(memory: u64) -> wonton_protocol::PluginLimits {
    wonton_protocol::PluginLimits {
        memory: Some(memory),
        cpu_percent: None,
    }
}

#[tokio::test]
async fn a_new_tag_is_offered_and_the_upgrade_keeps_the_data() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    release(
        &github,
        &hello(
            "1.1.0",
            "  worker:\n    image: worker:1\n",
            "  permissions: []\n",
        ),
    );
    release(&github, &hello("2.0.0-rc.1", "", "  permissions: []\n"));
    assert_eq!(m.check_upgrades_now().await, None);
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert_eq!(offer.version, "1.1.0", "预发布版本不提示");
    assert_eq!(offer.release_notes.as_deref(), Some("版本 1.1.0 的说明"));
    assert!(!offer.permissions_changed);

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert_eq!(preview.upgrade_from.as_deref(), Some("1.0.0"));
    assert_eq!(preview.version, "1.1.0");

    let before = docker.writes().len();
    upgrade(&m).await.unwrap();
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.version, "1.1.0");
    assert_eq!(plugin.upgrade, None);
    assert!(matches!(plugin.status, PluginState::Running { .. }));
    assert_eq!(plugin.source.as_deref(), Some("github:acme/hello@v1.1.0"));
    assert_eq!(image(&docker, &web()), "web:1.1.0");
    assert!(
        docker
            .containers()
            .contains(&container_name("app-hello", "worker"))
    );
    let writes = docker.writes();
    assert!(
        !writes[before..]
            .iter()
            .any(|w| w.starts_with("DELETE /volumes")),
        "数据卷不动：{writes:?}"
    );
    let record = m.store.load(&id()).unwrap().unwrap();
    assert_eq!(record.version, "1.1.0");
    assert_eq!(record.origin.unwrap().tag.as_deref(), Some("v1.1.0"));
    assert_eq!(
        m.preview_upgrade(&id(), UpgradeTo::Offered, &zh())
            .await
            .unwrap_err()
            .kind(),
        "not_found",
        "已经是最新的"
    );
}

#[tokio::test]
async fn stopped_plugins_are_offered_new_versions() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    docker.set_state(
        &web(),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    m.watch_containers().await;
    assert_eq!(
        m.installed_plugin(&id(), &zh()).unwrap().status,
        PluginState::Stopped
    );

    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    assert_eq!(m.check_upgrades_now().await, None);
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.status, PluginState::Stopped);
    assert_eq!(plugin.upgrade.unwrap().version, "1.1.0");
    assert_eq!(plugin.revision, revision);
}

#[tokio::test]
async fn a_new_version_that_will_not_start_rolls_back_to_the_old_containers() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    let worker = container_name("app-hello", "worker");
    docker.set_state(
        &worker,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    release(
        &github,
        &hello(
            "1.1.0",
            "  worker:\n    image: worker:1\n",
            "  permissions: []\n",
        ),
    );

    let err = upgrade(&m).await.unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-upgrade-rolled-back",
            &[("new", "1.1.0"), ("version", "1.0.0")],
        )),
        "{err}"
    );
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(m.status_of(&id()), Some(PluginStatus::Running));
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().revision, revision);
    assert_eq!(m.installed(&zh()).plugins[0].revision, revision);
    assert_eq!(
        m.installed_detail(&id(), &zh())
            .unwrap()
            .installed
            .unwrap()
            .revision,
        revision
    );
    assert_eq!(image(&docker, &web()), "web:1.0.0");
    assert!(!docker.containers().contains(&worker), "新版本的容器都删了");
    assert_eq!(m.store.load(&id()).unwrap().unwrap().version, "1.0.0");
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_never_touches_the_old_containers() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    docker.fail_pulls(Some("toomanyrequests"));

    let err = upgrade(&m).await.unwrap_err();
    assert_eq!(err.kind(), "image_pull_failed", "{err}");
    let stopped = format!("POST /containers/{}/stop", web());
    assert!(!docker.writes().iter().any(|w| w.starts_with(&stopped)));
    assert_eq!(m.status_of(&id()), Some(PluginStatus::Running));
    assert_eq!(version(&m), "1.0.0");
}

#[tokio::test]
async fn more_permissions_need_a_new_confirmation_and_a_moved_tag_a_new_preview() {
    let (github, docker, m, _dir) = with_github(true).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let wanting = hello("1.1.0", "", "  permissions: [network.egress]\n");
    release(&github, &wanting);
    m.check_upgrades_now().await;
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert!(offer.permissions_changed);

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert!(preview.permissions.iter().all(|p| p.added));
    let not_confirmed = UpgradeRequest {
        devices: Vec::new(),
        source: None,
        digest: preview.digest.clone(),
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        lan_webui: false,
    };
    let err = m
        .upgrade_source(&id(), &not_confirmed, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "permission_denied", "{err}");

    // 确认页开着的时候，作者把 tag 挪到了别的内容上。
    let confirmed = UpgradeRequest {
        grant: vec!["network.egress".to_owned()],
        ..not_confirmed
    };
    release(
        &github,
        &hello("1.1.0", "", "  permissions: [network.egress, models]\n"),
    );
    let err = m
        .upgrade_source(&id(), &confirmed, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "changed", "{err}");
    assert_eq!(version(&m), "1.0.0");

    // 能不能出网是建网络时定的：新版本要出网，网络跟着重建，核心接回去。
    release(&github, &wanting);
    upgrade(&m).await.unwrap();
    let network = "wonton-app-hello";
    assert_eq!(
        m.engine.network_reach(network).await.unwrap(),
        Some(super::super::docker::NetworkReach::Egress)
    );
    assert!(docker.endpoint(network, "wonton-core").is_some());
    assert!(m.status_of(&id()) == Some(PluginStatus::Running));
}

#[tokio::test]
async fn a_rewritten_tag_is_reported_and_not_an_upgrade() {
    let (github, _docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(
        &github,
        &hello("1.0.0", "", "  permissions: []\n  license: MIT\n"),
    );
    m.check_upgrades_now().await;
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.upgrade, None);
    assert!(
        plugin.notices[0].text.contains("v1.0.0"),
        "{:?}",
        plugin.notices
    );
    let err = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found", "{err}");
}

#[tokio::test]
async fn plugins_without_a_tag_are_not_checked_and_uploads_are_not_offered_upgrades() {
    let (github, docker, m, _dir) = with_github(false).await;
    let commit = release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, &format!("{SOURCE}@{commit}")).await;
    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    let parse = |compose: &str| PluginPackage::parse(compose, &super::langs()).unwrap();
    let newer = hello("1.2.0", "", "  permissions: []\n");
    m.upgrade_upload(&id(), parse(&newer), Vec::new(), false, &super::langs())
        .await
        .unwrap();
    assert_eq!(image(&docker, &web()), "web:1.2.0");
    assert_eq!(
        m.installed_plugin(&id(), &zh()).unwrap().trust,
        PluginTrust::Local,
        "上传的是本地来源，以后不查新版本"
    );

    let other = hello("1.3.0", "", "  permissions: []\n").replace("id: app.hello", "id: app.other");
    let err = m
        .upgrade_upload(&id(), parse(&other), Vec::new(), false, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
}

/// Bot 插件，出网。
fn echo(version: &str, services: &str) -> String {
    format!(
        "services:\n  bot:\n    image: echo:{version}\n    environment:\n      \
         TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}\n      AUTH: Bearer ${{WONTON_MATRIX_ACCESS_TOKEN}}\n{services}\
         x-wonton:\n  spec: '0.4'\n  id: app.echo\n  version: {version}\n  kind: app\n  title: {{zh: 回声, en: Echo}}\n  \
         description: {{zh: 回声, en: Echo}}\n  bot: {{display_name: {{zh: 回声, en: Echo}}}}\n  \
         permissions: [matrix.bot, network.egress]\n"
    )
}

#[tokio::test]
async fn a_bot_keeps_its_device_and_proxy_when_upgraded_and_when_rolled_back() {
    let (docker, engine) = FakeDocker::start().await;
    let (tuwunel, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
        .await
        .unwrap();
    let m = Arc::new(manager_in_container(engine, &dir).with_matrix(Some(Arc::new(matrix))));
    let settings = StoredEgress {
        url: Some("http://proxy:1".to_owned()),
        direct_cidrs: Vec::new(),
        use_plugin: false,
    };
    m.store.save_egress_settings(&settings).unwrap();
    let id = PluginId::parse("app.echo").unwrap();
    let bot = container_name("app-echo", "bot");
    let v1 = echo("1.0.0", "");
    m.install(&v1, &permissions(&v1), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &bot);
    let token = env["TOKEN"].clone();
    let devices = tuwunel.state().tokens.len();
    let device = m.store.bots().load(&id).unwrap().unwrap().device_id;

    let parse = |compose: &str| PluginPackage::parse(compose, &super::langs()).unwrap();
    let helper = container_name("app-echo", "helper");
    docker.set_state(
        &helper,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    let broken = echo("1.1.0", "  helper:\n    image: helper:1\n");
    let err = m
        .upgrade_upload(
            &id,
            parse(&broken),
            permissions(&broken),
            false,
            &super::langs(),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-upgrade-rolled-back",
            &[("new", "1.1.0"), ("version", "1.0.0")],
        )),
        "{err}"
    );
    let env = env_of_container(&docker, &bot);
    assert_eq!(env["TOKEN"], token, "退回旧版本也沿用同一台设备");
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");

    let v2 = echo("1.2.0", "");
    m.upgrade_upload(&id, parse(&v2), permissions(&v2), false, &super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &bot);
    assert_eq!(env["TOKEN"], token);
    assert_eq!(env["AUTH"], format!("Bearer {token}"));
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
    assert_eq!(image(&docker, &bot), "echo:1.2.0");
    assert_eq!(tuwunel.state().tokens.len(), devices, "没有签新设备");
    assert_eq!(m.store.bots().load(&id).unwrap().unwrap().device_id, device);
    assert!(m.store.load(&id).unwrap().unwrap().proxied);
    assert_eq!(m.status_of(&id), Some(PluginStatus::Running));

    // 令牌读不回来时报错，不悄悄签新设备。
    m.engine.remove_container(&bot).await.unwrap();
    let v3 = echo("1.3.0", "");
    let err = m
        .upgrade_upload(&id, parse(&v3), permissions(&v3), false, &super::langs())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-egress-token-read-back",
            &[
                (
                    "reason",
                    &super::message(
                        "plugin-settings-egress-old-container-gone",
                        &[("container", "wonton-app-echo--bot")],
                    )
                ),
                (
                    "reinstall",
                    &super::message("plugin-settings-egress-token-reinstall", &[]),
                ),
            ],
        )),
        "{err}"
    );
    assert_eq!(tuwunel.state().tokens.len(), devices);
    assert_eq!(m.status_of(&id), Some(PluginStatus::Running));
}

/// 没有来源的默认插件（Hermes 这样随核心预装的）：看官方索引。
#[tokio::test]
async fn a_bundled_default_plugin_is_offered_from_the_official_index() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    let m = Arc::new(manager_with(engine, &dir).with_store(StoreConfig {
        index_dir: Some(index.clone()),
        community: false,
    }));
    let defaults = dir.path().join("defaults");
    fs::create_dir_all(defaults.join("hello")).unwrap();
    let v1 = hello("1.0.0", "", "  permissions: []\n");
    fs::write(defaults.join("hello/docker-compose.yml"), &v1).unwrap();
    m.ensure_defaults(&defaults, Backoff::default())
        .await
        .unwrap();
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    let v2 =
        PluginPackage::parse(&hello("1.1.0", "", "  permissions: []\n"), &super::langs()).unwrap();
    let commit = "a".repeat(40);
    let entry = IndexEntry::of(&v2, SOURCE, &commit).unwrap();
    entry.write_to(&index, &v2).unwrap();
    assert_eq!(m.check_upgrades_now().await, None);
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.upgrade.unwrap().version, "1.1.0");

    upgrade(&m).await.unwrap();
    assert_eq!(image(&docker, &web()), "web:1.1.0");
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.trust, PluginTrust::Official);
    assert_eq!(plugin.source.as_deref(), Some("github:acme/hello@v1.1.0"));
}

#[tokio::test]
async fn system_plugins_upgrade_from_the_image_and_say_so_when_they_cannot() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let system = dir.path().join("system");
    fs::create_dir_all(system.join("hello")).unwrap();
    let write = |version: &str, services: &str| {
        let compose =
            hello(version, services, "  permissions: []\n").replace("kind: app", "kind: system");
        fs::write(system.join("hello/docker-compose.yml"), compose).unwrap();
    };
    write("1.0.0", "");
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    assert_eq!(version(&m), "1.0.0");
    let err = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "unsupported", "接口升不了系统插件：{err}");

    let worker = container_name("app-hello", "worker");
    docker.set_state(
        &worker,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    write("1.1.0", "  worker:\n    image: worker:1\n");
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.version, "1.0.0");
    assert!(matches!(plugin.status, PluginState::Running { .. }));
    assert!(
        {
            // 原因里有动态的出错详情，钉住前缀就够：版本、还在跑的版本和“下次启动再试”。
            let prefix = super::message(
                "plugin-settings-upgrade-bundled-failed",
                &[("version", "1.1.0"), ("from", "1.0.0"), ("error", "")],
            );
            let prefix = prefix
                .strip_suffix(": ")
                .or_else(|| prefix.strip_suffix("："))
                .unwrap_or(&prefix);
            plugin.notices[0].text.contains(prefix)
        },
        "{:?}",
        plugin.notices
    );

    write("1.2.0", "");
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.version, "1.2.0");
    assert!(plugin.notices.is_empty(), "{:?}", plugin.notices);
    assert_eq!(image(&docker, &web()), "web:1.2.0");
}

/// 核心升级后读不懂旧记录（清单新增了必填字段，插件版本号却没变）：接口重装不了系统插件，升级要先读旧包也走不通。
/// 启动时保留数据，用随核心带来的包重装。
#[tokio::test]
async fn a_system_plugin_whose_record_the_core_cannot_read_is_reinstalled_from_the_image() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let system = dir.path().join("system");
    fs::create_dir_all(system.join("hello")).unwrap();
    let compose = hello("1.0.0", "", "  permissions: []\n").replace("kind: app", "kind: system");
    fs::write(system.join("hello/docker-compose.yml"), &compose).unwrap();
    let m = manager_with(engine.clone(), &dir);
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    let kept = dir.path().join("apps").join(id().slug()).join("kept");
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::write(&kept, "主人的数据").unwrap();

    let old = compose.replace("  title: {zh: 你好, en: Hello}\n", "");
    assert!(
        PluginPackage::parse(&old, &super::langs()).is_err(),
        "旧记录这份核心读不懂"
    );
    let mut record = m.store.load(&id()).unwrap().unwrap();
    record.source = old;
    m.store.save(&record).unwrap();

    let m = manager_with(engine, &dir);
    m.restore().await.unwrap();
    assert!(
        matches!(m.list()[0].status, PluginStatus::Degraded { .. }),
        "重启后记录读不懂"
    );
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    assert!(matches!(m.list()[0].status, PluginStatus::Running));
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(m.store.load(&id()).unwrap().unwrap().source, compose);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "主人的数据");
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}

#[tokio::test]
async fn nothing_is_upgraded_while_the_core_itself_is_upgrading() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (frozen, watching) = watch::channel(false);
    let m = Arc::new(manager_with(engine, &dir).with_upgrade_freeze(watching));
    let v1 = hello("1.0.0", "", "  permissions: []\n");
    m.install(&v1, &[], super::langs()).await.unwrap();
    frozen.send(true).unwrap();
    let v2 =
        PluginPackage::parse(&hello("1.1.0", "", "  permissions: []\n"), &super::langs()).unwrap();
    let err = m
        .upgrade_upload(&id(), v2, Vec::new(), false, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "core_upgrading", "{err}");
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}

/// 新版本的网页端口要开关了：开关算关，确认页说清这些端口会关（ADR-0023）。
#[tokio::test]
async fn ports_that_become_switched_close_and_the_preview_says_so() {
    let (_docker, m, _host, _dir) = on(office()).await;
    let compose = |version: &str, webui: &str| {
        format!(
            "services:\n  web:\n    image: web:{version}\nx-wonton:\n  spec: '0.4'\n  id: app.hello\n  version: {version}\n  \
             kind: app\n  title: {{zh: 你好, en: Hello}}\n  description: {{zh: 打个招呼, en: Say hi}}\n  \
             webui: {{service: web, port: 1, {webui}}}\n  lan:\n    ports: [{{service: web, port: 1}}]\n  \
             permissions: [network.lan]\n"
        )
    };
    let port = serve_any().await;
    let v1 = compose("1.0.0", "sso: proxy, proxy: {headers: {X-Host: 'static'}}")
        .replace("port: 1", &format!("port: {port}"));
    let granted = permissions(&v1);
    m.install(&v1, &granted, super::langs()).await.unwrap();

    let v2 = compose("1.1.0", "sso: none").replace("port: 1", &format!("port: {port}"));
    let preview = m
        .preview_upgrade(
            &id(),
            UpgradeTo::Upload(Box::new(
                PluginPackage::parse(&v2, &super::langs()).unwrap(),
            )),
            &zh(),
        )
        .await
        .unwrap();
    assert!(!preview.lan_webui.unwrap().enabled);
    let closing = crate::l10n::text(
        &zh(),
        "plugin-settings-upgrade-lan-ports-closing",
        &[("ports", format!("{port}/tcp").into())],
    );
    let notice = preview.notices.iter().find(|n| n.text == closing);
    assert!(notice.is_some_and(|n| n.emphasis), "{:?}", preview.notices);
}

async fn serve_any() -> u16 {
    use axum::Router;
    use axum::routing::get;
    super::egress::serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port()
}

#[tokio::test]
async fn always_allow_stays_through_an_upgrade_and_the_preview_lists_it() {
    let (github, _docker, m, _dir) = with_github(true).await;
    let bot = "  permissions: [tools]\n";
    release(&github, &hello("1.0.0", "", bot));
    install(&m, SOURCE).await;
    let policy = m.tool_policy();
    policy.set_always("app.hello", "app.hello", true).unwrap();
    release(&github, &hello("1.1.0", "", bot));
    m.check_upgrades_now().await;

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    let listed: Vec<_> = preview.always_allowed.iter().map(|b| &b.caller).collect();
    assert_eq!(listed, ["app.hello"]);

    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    assert!(policy.is_always("app.hello", "app.hello"));

    policy.set_always("app.hello", "app.hello", false).unwrap();
    release(&github, &hello("1.2.0", "", bot));
    m.check_upgrades_now().await;
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert!(preview.always_allowed.is_empty());
}
