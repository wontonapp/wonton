//! 真实 Docker：删掉插件的容器和网络（相当于开发栈 down）后核心重启，restore 自己重建，不要求重装。

use super::super::PluginStatus;
use super::super::docker::Engine;
use super::manager::manager_in_instance;
use super::package;
use crate::test_docker::Names;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
async fn busybox(engine: &Engine) -> String {
    let reference = wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    );
    crate::test_docker::ensure_image(engine, &reference).await
}

/// busybox 起个 httpd 并配 healthcheck：安装的“就绪”等的是健康信号，裸 sleep 的容器永远不算就绪。
fn compose(image: &str) -> String {
    package(
        &format!(
            "  app:\n    image: {image}\n    init: true\n    command: ['sh', '-c', 'mkdir -p /www && echo ok > /www/index.html && exec httpd -f -p 8000 -h /www']\n    healthcheck:\n      test: ['CMD', 'wget', '-q', '-O', '/dev/null', 'http://127.0.0.1:8000/']\n      interval: 1s\n      timeout: 1s\n      retries: 3\n"
        ),
        "  permissions: []\n  webui: {service: app, port: 8000, sso: none}",
    )
}

#[tokio::test]
#[ignore = "需要 Docker，删掉插件的容器和网络后 restore 能自愈"]
async fn a_stack_restart_with_lost_network_and_containers_recovers_on_restore() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = busybox(&engine).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_instance(engine.clone(), &dir, names.name());
    let installed = manager
        .install(&compose(&image), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(installed.status, PluginStatus::Running);
    let slug = installed.id.slug();
    let network = names.network(&slug);
    let container = names.container(&slug, "app");

    // 相当于 compose down：先断开并删容器（网络还挂着端点时 Docker 不让删网络），再删网络。Docker 的
    // 端点清理是异步的，删网络要等它收完端点。
    engine
        .disconnect_network(&network, &container)
        .await
        .unwrap();
    engine.remove_container(&container).await.unwrap();
    for _ in 0..20 {
        match engine.remove_network(&network).await {
            Ok(()) => break,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
        }
    }
    assert!(engine.network_reach(&network).await.unwrap().is_none());

    let restarted = manager_in_instance(engine.clone(), &dir, names.name());
    restarted.restore().await.unwrap();
    let [view] = <[_; 1]>::try_from(restarted.list()).expect("只有一个插件");
    assert_eq!(view.status, PluginStatus::Running, "不用重装就该自己回来");
    assert!(engine.network_reach(&network).await.unwrap().is_some());
    let running = engine
        .inspect_container(&container)
        .await
        .unwrap()
        .and_then(|info| info.state)
        .and_then(|state| state.running);
    assert_eq!(running, Some(true));

    restarted
        .uninstall(&installed.id, false, &super::langs())
        .await
        .unwrap();
}
