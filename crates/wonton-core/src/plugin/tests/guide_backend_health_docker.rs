//! 真实 Docker：`assets/backend` 模板的健康检查轻、真能判断服务状态，并且不走代理。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::models::HealthStatusEnum;
use wonton_protocol::EgressProxyUpdate;

use super::super::docker::Engine;
use super::super::egress::gost;
use super::super::egress::probe::Prober;
use super::super::egress::settings::Egress;
use super::super::lan::Lan;
use super::super::lan::detect::Detector;
use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::{PluginId, PluginPackage};
use super::lan::{Host, office};
use super::manager::manager_in_instance;
use crate::test_docker::Names;

const PYTHON: &str = "public.ecr.aws/docker/library/python:3.13-alpine";

/// 模板包，外加出网权限：配了代理时健康检查才需要绕开代理。
fn template() -> PluginPackage {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin-guide/assets/backend");
    let mut package = PluginPackage::load_dir(&dir, &super::langs()).unwrap();
    package.manifest.permissions.push(Permission::NetworkEgress);
    package
}

fn lan() -> Lan {
    let host: Host = Arc::new(Mutex::new(Ok(office())));
    Lan::new(Detector::Fixed(host), None)
}

async fn health(engine: &Engine, container: &str) -> Option<HealthStatusEnum> {
    let info = engine.inspect_container(container).await.unwrap().unwrap();
    info.state
        .and_then(|state| state.health)
        .and_then(|h| h.status)
}

/// 给容器里跑 `pattern` 的进程发信号。模板带 `init: true`，PID 1 是 docker-init，
/// 冻结它不影响应用；信号又只能从容器外面发（PID 1 收不到容器里发的）。
async fn signal_process(engine: &Engine, container: &str, pattern: &str, signal: &str) {
    let exec = engine
        .docker
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(vec!["pkill", signal, "-f", pattern]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine
        .docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap();
}

async fn wait_for(engine: &Engine, container: &str, want: HealthStatusEnum, deadline: Duration) {
    let start = Instant::now();
    loop {
        let seen = health(engine, container).await;
        if seen == Some(want) {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "等 {want:?} 超时，现在是 {seen:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::test]
#[ignore = "需要 Docker，模板的健康检查不靠 Python、配了代理也探得了本机，并跟着服务状态走"]
async fn the_backend_example_probes_without_python_and_follows_the_service() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    for image in [gost::image(), wonton_test_images::image(PYTHON)] {
        crate::test_docker::ensure_image(&engine, &image).await;
    }
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name())
        .with_lan(lan())
        .with_egress(Egress::default().with_prober(Prober::Fine));
    manager.cfg.install_timeout = Duration::from_secs(60);
    let manager = Arc::new(manager);
    // 一个连不上的代理：探针要是不绕开代理，就永远进不了 healthy。
    manager
        .set_egress_proxy(
            EgressProxyUpdate {
                url: Some("http://192.0.2.1:9".to_owned()),
                use_plugin: false,
                direct_cidrs: Vec::new(),
                restarts: Vec::new(),
            },
            Languages::default(),
        )
        .await
        .unwrap();

    let package = template();
    let granted = package.manifest.permissions.clone();
    manager
        .install_package(package, &granted, super::langs())
        .await
        .unwrap();
    let container = names.container("app-example-notes", "app");

    let check = engine
        .inspect_container(&container)
        .await
        .unwrap()
        .unwrap()
        .config
        .unwrap()
        .healthcheck
        .expect("容器带上了健康检查");
    let test = check.test.expect("容器带上了 test");
    assert!(!test.iter().any(|arg| arg.contains("python")), "{test:?}");
    assert!(
        test.windows(3).any(|w| w == ["wget", "-Y", "off"]),
        "{test:?}"
    );
    wait_for(
        &engine,
        &container,
        HealthStatusEnum::HEALTHY,
        Duration::from_secs(60),
    )
    .await;

    // 服务卡住：进程和监听 socket 都还在，wget 连上后读超时（-T 3）才算失败。
    // 容器一退出 dockerd 就自己把健康状态改成 unhealthy、不跑探针，所以先冻结进程再等。
    signal_process(&engine, &container, "/app/server.py", "-STOP").await;
    let state = engine
        .inspect_container(&container)
        .await
        .unwrap()
        .unwrap()
        .state
        .and_then(|state| state.status);
    assert!(
        format!("{state:?}").to_lowercase().contains("running"),
        "{state:?}"
    );
    wait_for(
        &engine,
        &container,
        HealthStatusEnum::UNHEALTHY,
        Duration::from_secs(150),
    )
    .await;
    // 停住的进程收 SIGKILL 会直接没，回头再杀 docker-init，容器才退得了。exec 是 detached 的，等它落下。
    signal_process(&engine, &container, "/app/server.py", "-KILL").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    engine.kill_container(&container, "SIGKILL").await.unwrap();

    manager
        .uninstall(
            &PluginId::parse("app.example.notes").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
}
