//! 可选权限、默认关（ADR-0057）：清单可以声明可选项，关着装、开着才兑现字段，主人能随时关掉，Bot 打开必发卡片。

use serde_json::Value;

use std::sync::Arc;

use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::names::container_name;
use super::super::store::{EnabledPermission, OpenedBy};
use super::super::{PluginId, PluginManager, PluginPackage};
use super::manager::manager_with;
use super::{env_of_container, package};
use crate::plugin::manifest::Permission;

const OFF_LOSS: &str = "  optional_permissions:\n    docker.socket:\n      off_loss: {zh: 看不到每个容器, en: No per container usage}";

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

/// `web` 读状态变量，`agent` 挂 socket，`db` 什么都不拿。
fn app(permissions: &str, optional: &str) -> String {
    package(
        "  web:\n    image: nginx\n    environment: {STATE: '${WONTON_PERMISSION_DOCKER_SOCKET:-off}'}\n  \
         agent:\n    image: example/agent\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']\n  \
         db:\n    image: postgres",
        &format!("{optional}\n  permissions: [{permissions}]"),
    )
}

fn optional() -> String {
    app("", OFF_LOSS)
}

fn parsed(compose: &str) -> PluginPackage {
    PluginPackage::parse(compose, &super::langs()).unwrap()
}

fn granted(compose: &str) -> Vec<Permission> {
    parsed(compose).manifest.permissions
}

fn opened(permission: &str) -> Vec<EnabledPermission> {
    vec![EnabledPermission {
        permission: permission.parse().unwrap(),
        since: 10,
        opened_by: OpenedBy::Owner,
    }]
}

fn mounts(docker: &FakeDocker, service: &str) -> Value {
    let created = docker
        .created(&container_name("app-test", service))
        .expect("容器已创建");
    created["HostConfig"]["Mounts"]
        .clone()
        .as_array()
        .map(|m| Value::Array(m.clone()))
        .unwrap_or(Value::Null)
}

fn has_socket(docker: &FakeDocker, service: &str) -> bool {
    mounts(docker, service)
        .as_array()
        .is_some_and(|list| list.iter().any(|m| m["Source"] == "/var/run/docker.sock"))
}

async fn installed(
    package: &PluginPackage,
    enabled: &[EnabledPermission],
) -> (FakeDocker, Arc<PluginManager>, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    m.install_user_enabled(
        package.clone(),
        &package.manifest.permissions,
        enabled,
        false,
        &[],
        Default::default(),
        super::langs(),
    )
    .await
    .unwrap();
    (docker, m, dir)
}

#[test]
fn only_the_four_permissions_may_be_optional() {
    let full = "off_loss: {zh: 少, en: Less}";
    for good in [
        "docker.socket",
        "devices.gpu",
        "devices.serial",
        "devices.usb",
    ] {
        let extra = if good == "devices.gpu" {
            "\n  gpu: {services: [db]}"
        } else {
            ""
        };
        let compose = app(
            "",
            &format!("  optional_permissions:\n    {good}:\n      {full}{extra}"),
        );
        assert!(
            PluginPackage::parse(&compose, &super::langs()).is_ok(),
            "{good}"
        );
    }
    let bad = app(
        "",
        "  optional_permissions:\n    network.host:\n      off_loss: {en: Less}",
    );
    assert!(PluginPackage::parse(&bad, &super::langs()).is_err());
}

#[test]
fn off_loss_needs_both_languages() {
    let compose = app(
        "",
        "  optional_permissions:\n    docker.socket:\n      off_loss: {zh: 少了容器}",
    );
    assert!(PluginPackage::parse(&compose, &super::langs()).is_err());
}

#[test]
fn the_same_permission_cannot_be_required_and_optional() {
    let compose = app("docker.socket", OFF_LOSS);
    assert!(PluginPackage::parse(&compose, &super::langs()).is_err());
}

#[test]
fn an_optional_permission_still_needs_its_field() {
    let compose = package(
        "  web:\n    image: nginx",
        &format!("{OFF_LOSS}\n  permissions: []"),
    );
    let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
    assert!(
        super::super::render::dry_run(&package, &super::langs()).is_err(),
        "没有服务写那个卷"
    );
}

#[test]
fn agents_cannot_take_a_highest_permission_even_as_optional() {
    let compose = app("", OFF_LOSS).replace("kind: app", "kind: agent");
    assert!(PluginPackage::parse(&compose, &super::langs()).is_err());
}

#[tokio::test]
async fn the_optional_gpu_cannot_be_required() {
    let compose = app(
        "",
        "  optional_permissions:\n    devices.gpu:\n      off_loss: {zh: 慢一些, en: Slower}\n  \
         gpu: {services: [db]}\n  requirements: {gpu: {need: required}}",
    );
    assert!(PluginPackage::parse(&compose, &super::langs()).is_err());
    let optional_gpu = compose.replace("{need: required}", "{need: optional}");
    assert!(
        PluginPackage::parse(&optional_gpu, &super::langs()).is_ok(),
        "{:?}",
        PluginPackage::parse(&optional_gpu, &super::langs()).err()
    );
}

#[tokio::test]
async fn installing_without_the_choice_leaves_the_socket_out() {
    let (docker, _m, _dir) = installed(&parsed(&optional()), &[]).await;
    assert!(
        !has_socket(&docker, "agent"),
        "{:?}",
        mounts(&docker, "agent")
    );
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(env.get("STATE").map(String::as_str), Some("off"));
}

#[tokio::test]
async fn enabling_mounts_the_socket_and_tells_every_service() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    assert!(
        has_socket(&docker, "agent"),
        "{:?}",
        mounts(&docker, "agent")
    );
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(env.get("STATE").map(String::as_str), Some("1"));
    let _ = m;
}

#[tokio::test]
async fn a_grant_that_still_carries_the_optional_one_does_not_switch_it_on() {
    let (docker, _m, _dir) = installed(&parsed(&optional()), &[]).await;
    assert!(!has_socket(&docker, "agent"));
}

#[tokio::test]
async fn the_switch_removes_and_brings_back_the_mount_without_touching_other_services() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    let before = docker.writes().len();
    let detail = m
        .set_plugin_permission(
            id(),
            Permission::DockerSocket,
            false,
            OpenedBy::Owner,
            super::langs(),
        )
        .await
        .unwrap();
    let line = detail
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(line.optional);
    assert!(!line.enabled);
    assert!(line.opened.is_none());
    assert!(!has_socket(&docker, "agent"));

    let writes = docker.writes()[before..].to_vec();
    let db = container_name("app-test", "db");
    assert!(
        !writes.contains(&format!("DELETE /containers/{db}")),
        "没变的服务重启了：{writes:?}"
    );
    let agent = container_name("app-test", "agent");
    assert!(writes.contains(&format!("DELETE /containers/{agent}")));

    let opened = m
        .set_plugin_permission(
            id(),
            Permission::DockerSocket,
            true,
            OpenedBy::Owner,
            super::langs(),
        )
        .await
        .unwrap();
    let line = opened
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(line.enabled);
    assert!(line.opened.as_ref().unwrap().bot.is_none(), "主人打开的");
    assert!(has_socket(&docker, "agent"));
}

#[tokio::test]
async fn the_switch_is_idempotent_and_rejects_permissions_that_are_not_optional() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    let before = docker.writes().len();
    m.set_plugin_permission(
        id(),
        Permission::DockerSocket,
        true,
        OpenedBy::Owner,
        super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(docker.writes().len(), before, "已经开着，不重建");

    let required = app("docker.socket", "");
    let (docker2, m2, _dir2) = installed(&parsed(&required), &[]).await;
    let _ = docker2;
    assert!(matches!(
        m2.set_plugin_permission(
            id(),
            Permission::DockerSocket,
            false,
            OpenedBy::Owner,
            super::langs()
        )
        .await,
        Err(super::super::manager::InstallError::Invalid(_))
    ));
}

#[tokio::test]
async fn a_failed_switch_keeps_the_old_state() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    docker.fail_starts(Some("启动失败"));
    let err = m
        .set_plugin_permission(
            id(),
            Permission::DockerSocket,
            false,
            OpenedBy::Owner,
            super::langs(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("启动失败"), "{err}");
    let record = m.store.load(&id()).unwrap().unwrap();
    assert_eq!(record.enabled_permissions.len(), 1, "记录没有改成关");
    assert!(has_socket(&docker, "agent"), "容器退回开着的样子");
}

#[tokio::test]
async fn a_rejected_grant_missing_a_required_permission_is_refused() {
    let compose = app("owner.identity", OFF_LOSS);
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let err = m
        .install_user_enabled(
            parsed(&compose),
            &[],
            &[],
            false,
            &[],
            Default::default(),
            super::langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "permission_denied", "{err}");
    assert!(docker.containers().is_empty());
}

#[tokio::test]
async fn enabling_a_permission_that_is_not_optional_is_refused() {
    let compose = app("owner.identity", OFF_LOSS);
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let err = m
        .install_user_enabled(
            parsed(&compose),
            &granted(&compose),
            &opened("owner.identity"),
            false,
            &[],
            Default::default(),
            super::langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
    let _ = docker;
}

#[tokio::test]
async fn the_preview_lists_the_option_off_with_its_loss_and_no_block() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let preview = m
        .preview_install(&parsed(&optional()), &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    assert_eq!(
        preview.permissions.last().unwrap().permission,
        "docker.socket"
    );
    let line = preview
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(line.optional);
    assert!(!line.enabled);
    assert!(line.off_loss.is_some());
    assert!(!preview.devices.as_ref().is_some_and(|d| d.is_empty()));
}

#[tokio::test]
async fn the_list_marks_a_running_highest_optional_permission() {
    let (_docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    let listed = m.installed(&Languages::default()).plugins;
    assert_eq!(listed[0].enabled_root_permissions, ["docker.socket"]);
    m.set_plugin_permission(
        id(),
        Permission::DockerSocket,
        false,
        OpenedBy::Owner,
        super::langs(),
    )
    .await
    .unwrap();
    assert!(
        m.installed(&Languages::default()).plugins[0]
            .enabled_root_permissions
            .is_empty()
    );
}

#[tokio::test]
async fn a_bot_is_recorded_as_the_opener() {
    let (_docker, m, _dir) = installed(&parsed(&optional()), &[]).await;
    let detail = m
        .set_plugin_permission(
            id(),
            Permission::DockerSocket,
            true,
            OpenedBy::Bot {
                plugin_id: "app.hermes".into(),
                title: "Hermes".into(),
            },
            super::langs(),
        )
        .await
        .unwrap();
    let opened = detail
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap()
        .opened
        .clone()
        .expect("记下了打开的人");
    assert_eq!(opened.bot.unwrap().title, "Hermes");
}

#[tokio::test]
async fn upgrading_keeps_a_kept_option_and_turns_a_new_required_one_into_added() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    let next = parsed(
        &package(
            "  web:\n    image: nginx\n  agent:\n    image: example/agent\n    \
             volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
            &format!("{OFF_LOSS}\n  permissions: [owner.identity]"),
        )
        .replace("version: 1.0.0", "version: 1.1.0"),
    );
    let preview = m
        .preview_upgrade(
            &id(),
            super::super::upgrade::UpgradeTo::Upload(Box::new(next.clone())),
            &Languages::default(),
        )
        .await
        .unwrap();
    let socket = preview
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(socket.enabled, "开着的保持开着");
    let identity = preview
        .permissions
        .iter()
        .find(|line| line.permission == "owner.identity")
        .unwrap();
    assert!(identity.added);
    let _ = docker;
}

#[tokio::test]
async fn upgrading_away_the_optional_one_drops_the_mount() {
    let (docker, m, _dir) = installed(&parsed(&optional()), &opened("docker.socket")).await;
    let next = parsed(
        &package(
            "  web:\n    image: nginx\n  agent:\n    image: example/agent",
            "  permissions: []",
        )
        .replace("version: 1.0.0", "version: 1.1.0"),
    );
    m.upgrade_checked(next, &[], &[], false, &[], Default::default())
        .await
        .unwrap();
    assert!(!has_socket(&docker, "agent"));
    let record = m.store.load(&id()).unwrap().unwrap();
    assert!(record.enabled_permissions.is_empty());
}

#[tokio::test]
async fn the_detail_shows_the_switch_state_and_who_opened_it() {
    let (_docker, m, _dir) = installed(&parsed(&optional()), &[]).await;
    let m = &m;
    let detail = m.installed_detail(&id(), &Languages::default()).unwrap();
    let line = detail
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(line.optional && !line.enabled);
    assert!(matches!(
        m.installed(&Languages::default()).plugins[0].status,
        wonton_protocol::PluginState::Running { .. }
    ));
}
