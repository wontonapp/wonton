//! 同插件网络命名空间共享：声明边界、依赖启动和目标重建。

use super::super::compose::DependCondition;
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::names::{container_name, service_host};
use super::super::render::{Deployment, dry_run};
use super::super::upstreams::Endpoint;
use super::super::{PluginPackage, PluginStatus};
use super::lan::{granted, office, on};
use super::manager::manager_in_container;
use super::package;

const SERVICES: &str =
    "  target:\n    image: nginx\n  sidecar:\n    image: busybox\n    network_mode: service:target";

fn plan(extra: &str) -> Deployment {
    dry_run(
        &PluginPackage::parse(&package(SERVICES, extra), &super::langs()).unwrap(),
        &super::langs(),
    )
    .unwrap()
}

#[test]
fn network_mode_service_joins_only_the_target_namespace_after_it_starts() {
    let plan = plan("  permissions: []");
    let names: Vec<_> = plan.containers.iter().map(|c| c.service.as_str()).collect();
    assert_eq!(names, ["target", "sidecar"]);
    let sidecar = &plan.containers[1];
    assert_eq!(
        sidecar
            .body
            .host_config
            .as_ref()
            .unwrap()
            .network_mode
            .as_deref(),
        Some("container:wonton-app-test--target")
    );
    assert!(sidecar.body.networking_config.is_none());
    assert!(
        sidecar
            .body
            .host_config
            .as_ref()
            .unwrap()
            .extra_hosts
            .is_none()
    );
    assert_eq!(
        sidecar.depends_on,
        [(
            container_name("app-test", "target"),
            DependCondition::ServiceStarted
        )]
    );
    let endpoints = plan.containers[0].body.networking_config.as_ref().unwrap();
    assert_eq!(
        endpoints.endpoints_config.as_ref().unwrap()[&plan.network]
            .aliases
            .as_deref(),
        Some(&["target".to_owned(), service_host("app-test", "target")][..])
    );
}

#[test]
fn network_mode_bot_services_share_the_default_bots_target_container() {
    let plan = plan(
        "  permissions: [matrix.bot]\n  bots: {services: [target, sidecar]}\n  webui: {service: target, port: 80}",
    );
    let target = &plan.containers[0];
    let sidecar = &plan.containers[1];
    assert_eq!(
        sidecar.body.host_config.as_ref().unwrap().network_mode,
        Some(format!("container:{}", target.name))
    );
    let endpoints = target.body.networking_config.as_ref().unwrap();
    assert_eq!(
        endpoints.endpoints_config.as_ref().unwrap()[&plan.network]
            .aliases
            .as_deref(),
        Some(
            &[
                "target".to_owned(),
                "target-default".to_owned(),
                service_host("app-test", "target-default"),
                service_host("app-test", "target")
            ][..]
        )
    );
}

#[tokio::test]
async fn network_mode_invalid_targets_and_entries_are_invalid_input() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine, &dir);
    let cases = vec![
        (
            SERVICES.replace("service:target", "service:missing"),
            "  permissions: []".to_owned(),
        ),
        (
            SERVICES.replace("service:target", "service:sidecar"),
            "  permissions: []".to_owned(),
        ),
        (
            SERVICES.replace("service:target", "service:"),
            "  permissions: []".to_owned(),
        ),
        (
            SERVICES.replace("image: nginx", "image: nginx\n    network_mode: none"),
            "  permissions: []".to_owned(),
        ),
        (
            SERVICES.replace("image: nginx", "image: nginx\n    network_mode: host"),
            "  permissions: [network.host]".to_owned(),
        ),
        (
            format!("{SERVICES}\n  other:\n    image: nginx\n    network_mode: service:sidecar"),
            "  permissions: []".to_owned(),
        ),
        (
            SERVICES.to_owned(),
            "  permissions: []\n  webui: {service: sidecar, port: 80}".to_owned(),
        ),
        (
            SERVICES.to_owned(),
            "  permissions: []\n  mcp: {service: sidecar, port: 80, transport: streamable-http}"
                .to_owned(),
        ),
        (
            SERVICES.to_owned(),
            "  permissions: [network.lan]\n  lan: {ports: [{service: sidecar, port: 8080}]}"
                .to_owned(),
        ),
    ];
    for (services, manifest) in cases {
        let error = manager
            .install(&package(&services, &manifest), &[], super::langs())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            "invalid_input",
            "{services}\n{manifest}: {error}"
        );
    }
    assert!(docker.writes().is_empty());
}

#[tokio::test]
async fn network_mode_shared_services_reject_ignored_network_fields_and_cross_bot_groups() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine, &dir);
    let mut cases = Vec::new();
    for field in [
        "networks: []",
        "ports: []",
        "hostname: sidecar",
        "domainname: example.com",
        "dns: []",
        "dns_search: []",
        "extra_hosts: []",
        "mac_address: '02:00:00:00:00:01'",
        "expose: []",
    ] {
        cases.push((
            format!("{SERVICES}\n    {field}"),
            "  permissions: []".to_owned(),
        ));
    }
    for bot in ["target", "sidecar"] {
        cases.push((
            SERVICES.to_owned(),
            format!("  permissions: [matrix.bot]\n  bots: {{services: [{bot}]}}"),
        ));
    }
    for (services, manifest) in cases {
        let error = manager
            .install(&package(&services, &manifest), &[], super::langs())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            "invalid_input",
            "{services}\n{manifest}: {error}"
        );
    }
    assert!(docker.writes().is_empty());
}

#[tokio::test]
async fn network_mode_webui_uses_the_default_bot_container() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine, &dir);
    let package = PluginPackage::parse(
            &package(
                SERVICES,
                "  permissions: [matrix.bot]\n  bots: {services: [target, sidecar]}\n  webui: {service: target, port: 80}",
            ),
            &super::langs(),
        )
        .unwrap();
    let target = container_name("app-test", "target");
    docker.add_container(&target, "app.test");
    assert_eq!(
        manager
            .webui_upstream(&package.manifest, "wonton-app-test")
            .await
            .unwrap(),
        Some(Endpoint::Tcp(format!(
            "{}:80",
            service_host("app-test", "target-default")
        )))
    );
}

#[tokio::test]
async fn network_mode_rebuilding_a_target_replaces_its_sidecars_but_not_other_services() {
    let (docker, manager, detected, _dir) = on(office()).await;
    let source = package(
        &format!("{SERVICES}\n  unrelated:\n    image: nginx"),
        "  permissions: [network.lan]\n  lan: {ports: [{service: target, port: 8080}]}",
    );
    manager
        .install(&source, &granted(&source), super::langs())
        .await
        .unwrap();
    let before = docker.writes().len();
    let mut changed = office();
    changed
        .interfaces
        .iter_mut()
        .find(|i| i.name == "eno1")
        .unwrap()
        .addresses[0]
        .ip = "10.0.0.10".parse().unwrap();
    *detected.lock().unwrap() = Ok(changed);
    manager.lan_pass(&Languages::default()).await.unwrap();
    let writes = docker.writes()[before..].to_vec();
    let target = container_name("app-test", "target");
    let sidecar = container_name("app-test", "sidecar");
    let position = |operation: String| {
        writes
            .iter()
            .position(|w| *w == operation)
            .unwrap_or_else(|| panic!("{operation}: {writes:?}"))
    };
    assert!(
        position(format!("DELETE /containers/{target}"))
            < position(format!("DELETE /containers/{sidecar}"))
    );
    assert!(
        position(format!("POST /containers/{target}/start"))
            < position(format!("POST /containers/{sidecar}/start"))
    );
    assert!(!writes.iter().any(|w| w.contains("--unrelated")));
    assert_eq!(manager.list()[0].status, PluginStatus::Running);
}

#[test]
fn network_mode_preserves_explicit_health_conditions_and_rejects_dependency_cycles() {
    let source = package(
        &format!("{SERVICES}\n    depends_on: {{target: {{condition: service_healthy}}}}"),
        "  permissions: []",
    );
    let plan = dry_run(
        &PluginPackage::parse(&source, &super::langs()).unwrap(),
        &super::langs(),
    )
    .unwrap();
    assert_eq!(
        plan.containers[1].depends_on,
        [(
            container_name("app-test", "target"),
            DependCondition::ServiceHealthy
        )]
    );
    let source = package(
        &SERVICES.replace("image: nginx", "image: nginx\n    depends_on: [sidecar]"),
        "  permissions: []",
    );
    assert!(matches!(
        dry_run(
            &PluginPackage::parse(&source, &super::langs()).unwrap(),
            &super::langs()
        ),
        Err(super::super::render::RenderError::Service { .. })
    ));
}

#[tokio::test]
async fn network_mode_existing_containers_start_target_before_sidecar() {
    use serde_json::json;
    use std::sync::Arc;

    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(super::manager::manager_with(engine, &dir));
    let installed = manager
        .install(&package(SERVICES, "  permissions: []"), &[], super::langs())
        .await
        .unwrap();
    let target = container_name("app-test", "target");
    let sidecar = container_name("app-test", "sidecar");
    for container in [&target, &sidecar] {
        docker.set_state(
            container,
            json!({"Status": "exited", "Running": false, "ExitCode": 0}),
        );
        docker.set_start_state(
            container,
            json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
        );
    }
    let before = docker.writes().len();
    let started = manager.start_detached(installed.id).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    assert_eq!(
        &docker.writes()[before..],
        [
            format!("POST /containers/{target}/start"),
            format!("POST /containers/{sidecar}/start")
        ]
    );
}

#[tokio::test]
async fn network_mode_limits_update_both_containers_in_place() {
    use std::sync::Arc;

    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(super::manager::manager_with(engine, &dir));
    let installed = manager
        .install(&package(SERVICES, "  permissions: []"), &[], super::langs())
        .await
        .unwrap();
    let before = docker.writes().len();
    manager
        .set_limits(
            installed.id,
            wonton_protocol::PluginLimits {
                memory: Some(256 * 1024 * 1024),
                cpu_percent: None,
            },
            Default::default(),
        )
        .await
        .unwrap();
    let writes = docker.writes()[before..].to_vec();
    assert_eq!(
        writes,
        [
            format!(
                "POST /containers/{}/update",
                container_name("app-test", "target")
            ),
            format!(
                "POST /containers/{}/update",
                container_name("app-test", "sidecar")
            ),
        ]
    );
    for service in ["target", "sidecar"] {
        assert_eq!(
            docker
                .created(&container_name("app-test", service))
                .unwrap()["HostConfig"]["Memory"],
            serde_json::json!(256 * 1024 * 1024)
        );
    }
}

#[tokio::test]
async fn network_mode_upgrade_recreates_the_target_and_every_sidecar() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::manager::manager_with(engine, &dir);
    let source = package(SERVICES, "  permissions: []");
    manager.install(&source, &[], super::langs()).await.unwrap();
    let next = PluginPackage::parse(
        &source
            .replace("version: 1.0.0", "version: 1.1.0")
            .replace("image: nginx", "image: nginx:next"),
        &super::langs(),
    )
    .unwrap();
    let before = docker.writes().len();
    manager
        .upgrade_checked(next, &[], &[], false, &[], Default::default())
        .await
        .unwrap();
    let writes = docker.writes()[before..].to_vec();
    for service in ["target", "sidecar"] {
        let name = container_name("app-test", service);
        assert!(writes.contains(&format!("DELETE /containers/{name}")));
        assert!(writes.contains(&format!("POST /containers/{name}/start")));
    }
    let target = container_name("app-test", "target");
    let sidecar = container_name("app-test", "sidecar");
    assert_eq!(docker.created(&target).unwrap()["Image"], "nginx:next");
    assert_eq!(
        docker.created(&sidecar).unwrap()["HostConfig"]["NetworkMode"],
        format!("container:{target}")
    );
    let position = |operation: String| writes.iter().position(|w| *w == operation).unwrap();
    assert!(
        position(format!("POST /containers/{target}/start"))
            < position(format!("POST /containers/{sidecar}/start"))
    );
    assert_eq!(manager.list()[0].version, "1.1.0");
}

#[test]
fn network_mode_bot_webui_does_not_open_mcp_provides_or_host_network() {
    use super::super::package::PackageError;

    let bots = "  permissions: [matrix.bot]\n  bots: {services: [target, sidecar]}";
    let mcp = package(
        SERVICES,
        &format!("{bots}\n  mcp: {{service: target, port: 80, transport: streamable-http}}"),
    );
    assert!(matches!(
        PluginPackage::parse(&mcp, &super::langs()),
        Err(PackageError::Service { .. })
    ));
    let provider = package(SERVICES, &format!("{bots}\n  provides:\n    models:\n      service: target\n      port: 80\n      apis: {{openai: /v1}}\n      default_model: test\n      credentials:\n        issue: [issue, '{{name}}']\n        revoke: [revoke, '{{name}}']"))
        .replace("kind: app", "kind: system");
    assert!(matches!(
        PluginPackage::parse(&provider, &super::langs()),
        Err(PackageError::Service { .. })
    ));
    let host = package(
        &SERVICES.replace("image: nginx", "image: nginx\n    network_mode: host"),
        &format!("{bots}\n  webui: {{service: target, port: 80}}"),
    );
    assert!(matches!(
        dry_run(
            &PluginPackage::parse(&host, &super::langs()).unwrap(),
            &super::langs()
        ),
        Err(super::super::render::RenderError::Package(
            PackageError::Service { .. }
        ))
    ));
}

#[tokio::test]
async fn network_mode_waits_for_the_target_to_run_before_creating_a_sidecar() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::manager::manager_with(engine, &dir);
    docker.set_state(
        &container_name("app-test", "target"),
        serde_json::json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    let error = manager
        .install(&package(SERVICES, "  permissions: []"), &[], super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "unhealthy");
    assert!(
        docker
            .created(&container_name("app-test", "sidecar"))
            .is_none()
    );
    assert!(docker.containers().is_empty());
}
