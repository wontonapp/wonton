//! 已授权设备缺失只影响对应服务及依赖者，不阻止其余服务启动或恢复路由。

use std::sync::Arc;

use serde_json::json;

use super::super::fake_docker::FakeDocker;
use super::super::gpu_fake_host::FakeHost;
use super::super::health_gate::Gate;
use super::super::manifest::{Permission, PluginId};
use super::super::names::container_name;
use super::super::{PluginManager, PluginPackage, PluginStatus};
use super::langs;
use super::manager::manager_with;

fn source(optional: bool) -> String {
    super::package(
        "  radio: {image: example/radio}\n  worker:\n    image: example/worker\n    depends_on: [radio]\n  web: {image: example/web}",
        &format!(
            "  permissions: [devices.serial]\n  devices:\n    serial:\n      - {{service: radio, target: /dev/radio, optional: {optional}}}"
        ),
    )
}

async fn installed(
    optional: bool,
) -> (FakeHost, FakeDocker, Arc<PluginManager>, tempfile::TempDir) {
    let host = FakeHost::empty();
    host.serial("usb-radio", "ttyUSB0");
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir).with_host(host.paths()));
    manager
        .install_with_devices(
            &source(optional),
            &[Permission::DevicesSerial],
            &["serial:0:/dev/serial/by-id/usb-radio".to_owned()],
            langs(),
        )
        .await
        .unwrap();
    (host, docker, manager, dir)
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

fn unplug(host: &FakeHost) {
    std::fs::remove_file(host.dev().join("serial/by-id/usb-radio")).unwrap();
}

fn assert_only_web_serves(manager: &PluginManager) {
    let plugins = manager.plugins.read().unwrap();
    let entry = plugins.get(&id()).unwrap();
    assert!(matches!(entry.status, PluginStatus::Degraded { .. }));
    assert_eq!(entry.serving, ["web".to_owned()].into());
    drop(plugins);
    let detail = manager.installed_detail(&id(), &langs()).unwrap();
    assert_eq!(
        detail.problems[0].reason,
        wonton_protocol::PluginFailure::DeviceUnavailable
    );
}

#[tokio::test]
async fn missing_selected_serial_starts_other_services_and_keeps_its_authorization() {
    for optional in [false, true] {
        for removed in [false, true] {
            let (host, docker, manager, _dir) = installed(optional).await;
            unplug(&host);
            for service in ["radio", "worker", "web"] {
                let name = container_name("app-test", service);
                docker.set_state(
                    &name,
                    json!({"Status": "exited", "Running": false, "ExitCode": 0}),
                );
                docker.set_start_state(&name, json!({"Status": "running", "Running": true}));
            }
            if removed {
                docker.remove(&container_name("app-test", "radio"));
                docker.remove(&container_name("app-test", "worker"));
            }
            let before = docker.writes().len();
            let result = manager.start_detached(id()).await.unwrap();
            assert!(matches!(result.status, PluginStatus::Degraded { .. }));
            let writes = docker.writes();
            let writes = &writes[before..];
            assert!(
                writes
                    .iter()
                    .any(|write| write == "POST /containers/wonton-app-test--web/start")
            );
            assert!(
                !writes
                    .iter()
                    .any(|write| write.ends_with("--radio/start")
                        || write.ends_with("--worker/start"))
            );
            assert_only_web_serves(&manager);
            assert_eq!(manager.store.load(&id()).unwrap().unwrap().devices.len(), 1);
            manager.device_pass().await.unwrap();
            host.serial("usb-radio", "ttyUSB1");
            let before_replug = docker.writes().len();
            manager.device_pass().await.unwrap();
            tokio::time::sleep(
                super::super::device::watch::STABLE + std::time::Duration::from_millis(200),
            )
            .await;
            manager.device_pass().await.unwrap();
            manager.watch_containers().await;
            let detail = manager.installed_detail(&id(), &langs()).unwrap();
            assert!(matches!(
                detail.installed.unwrap().status,
                wonton_protocol::PluginState::Running { .. }
            ));
            assert!(detail.problems.is_empty());
            let plugins = manager.plugins.read().unwrap();
            assert_eq!(
                plugins[&id()].serving,
                ["radio".to_owned(), "worker".to_owned(), "web".to_owned()].into()
            );
            drop(plugins);
            let writes = docker.writes();
            let writes = &writes[before_replug..];
            assert!(writes.iter().any(|write| write.ends_with("--worker/start")));
            assert!(
                !writes.iter().any(|write| write.contains("--web")),
                "{writes:?}"
            );
        }
    }
}

#[tokio::test]
async fn restore_preserves_unrelated_service_routes_with_a_missing_selected_device() {
    let (host, docker, manager, dir) = installed(false).await;
    unplug(&host);
    let before = docker.writes().len();
    let restored = manager_with(docker.engine().await, &dir).with_host(host.paths());
    restored.restore().await.unwrap();
    assert_only_web_serves(&restored);
    restored.watch_containers().await;
    assert_only_web_serves(&restored);
    assert_eq!(docker.writes().len(), before, "已运行的其他服务无需重建");
    assert_eq!(manager.store.load(&id()).unwrap().unwrap().devices.len(), 1);
}

#[tokio::test]
async fn drift_rebuild_skips_unavailable_services_but_rebuilds_the_rest() {
    let (host, docker, manager, _dir) = installed(false).await;
    unplug(&host);
    let record = manager.store.load(&id()).unwrap().unwrap();
    let package =
        PluginPackage::with_files(&record.source, record.files.clone(), &langs()).unwrap();
    let mut ctx = manager
        .rebuild_context(&package, &record, None, None)
        .await
        .unwrap();
    ctx.tz = "Europe/London".to_owned();
    let plan = super::super::render::render(&package, &ctx, &langs()).unwrap();
    let before = docker.writes().len();
    manager
        .replace_containers(&ctx, plan, Gate::StartPeriod)
        .await
        .unwrap();
    let writes = docker.writes();
    assert!(
        writes[before..]
            .iter()
            .all(|write| !write.contains("--radio") && !write.contains("--worker"))
    );
    assert_eq!(docker.creations(&container_name("app-test", "web")), 2);
    assert_eq!(docker.creations(&container_name("app-test", "radio")), 1);
    assert_eq!(docker.creations(&container_name("app-test", "worker")), 1);
    let deployed = manager.reconcile_running(&package, &record).await.unwrap();
    assert_eq!(deployed.serving, ["web".to_owned()].into());
    assert!(deployed.problem.is_some());
}
