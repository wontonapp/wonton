//! 真实 Docker：没有 Compose 或镜像健康检查的网页服务由核心直连探测，失败安装撤销资源。

use std::time::Duration;

use super::super::docker::Engine;
use super::super::names::LABEL_INSTANCE;
use super::super::store::Store;
use super::super::{Health, PluginId, PluginManager, PluginStatus};
use super::manager::manager_in_instance;
use super::package;
use crate::test_docker::Names;

async fn busybox(engine: &Engine) -> String {
    let reference = wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    );
    let image = crate::test_docker::ensure_image(engine, &reference).await;
    assert!(engine.image_healthcheck(&image).await.unwrap().is_none());
    image
}

fn compose(image: &str, path: Option<&str>, failing: bool) -> String {
    // 根路径永远回 200；CGI 在 404/503 之间切换，防止探测误用根路径也通过。
    let script = format!(
        "mkdir -p /www/cgi-bin\necho ok > /www/index.html\n\
         cat > /www/cgi-bin/probe <<'CGI'\n\
         #!/bin/sh\n\
         if [ -f /www/failing ]; then\n\
         printf 'Status: 503 Service Unavailable\\r\\n'\n\
         else\n\
         printf 'Status: 404 Not Found\\r\\n'\n\
         fi\n\
         printf 'Content-Type: text/plain\\r\\n\\r\\nprobe\\n'\n\
         CGI\n\
         chmod 755 /www/cgi-bin/probe\n\
         {}\n\
         exec httpd -f -p 8000 -h /www",
        if failing {
            "touch /www/failing"
        } else {
            "rm -f /www/failing"
        }
    );
    let command = serde_json::to_string(&["sh", "-c", &script]).unwrap();
    let path = path.map_or_else(String::new, |path| format!(", path: {path}"));
    format!(
        "volumes:\n  data: {{}}\n{}",
        package(
            &format!(
                "  app:\n    image: {image}\n    init: true\n    command: {command}\n    volumes: ['data:/www']\n"
            ),
            &format!(
                "  permissions: []\n  webui: {{service: app, port: 8000, sso: none{path}}}"
            ),
        )
        .replace("spec: '0.5'", "spec: '0.15'")
    )
}

fn manager(engine: Engine, dir: &tempfile::TempDir, names: &Names) -> PluginManager {
    let mut manager = manager_in_instance(engine, dir, names.name());
    // 宿主负载不能耗尽真实 Docker 的启动预算；只改本组测试，不放宽其他测试。
    manager.cfg.install_timeout = Duration::from_secs(30);
    manager
}

fn seen(manager: &PluginManager) -> (PluginStatus, Option<Health>) {
    let [view] = <[_; 1]>::try_from(manager.list()).expect("只有一个插件");
    (view.status, view.health)
}

async fn assert_no_docker_healthcheck(engine: &Engine, container: &str) {
    let info = engine.inspect_container(container).await.unwrap().unwrap();
    assert!(info.config.unwrap().healthcheck.is_none());
    let state = info.state.unwrap();
    assert_eq!(state.running, Some(true));
    assert!(state.health.is_none());
}

async fn assert_resources_removed(engine: &Engine, names: &Names) {
    let resources = engine
        .labeled(LABEL_INSTANCE, names.name(), |_| true)
        .await
        .unwrap();
    assert!(
        resources.containers.is_empty(),
        "{:?}",
        resources.containers
    );
    assert!(resources.networks.is_empty(), "{:?}", resources.networks);
    assert!(resources.volumes.is_empty(), "{:?}", resources.volumes);
    assert!(
        engine
            .inspect_container(&names.container("app-test", "app"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        engine
            .network_reach(&names.network("app-test"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        engine
            .volume_labels(&names.instance().volume("app-test", "data"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "需要 Docker，无 healthcheck 的网页服务安装后由核心探测为健康"]
async fn a_webui_without_healthcheck_installs_running_and_healthy() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = busybox(&engine).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine.clone(), &dir, &names);
    let installed = manager
        .install(&compose(&image, None, false), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(installed.status, PluginStatus::Running);
    assert_eq!(installed.health, Some(Health::Healthy));
    assert_no_docker_healthcheck(&engine, &names.container("app-test", "app")).await;
    let probe = manager
        .health_report(&installed.id)
        .await
        .unwrap()
        .probe
        .unwrap();
    assert!(probe.reachable);
    assert_eq!(probe.http_status, Some(200));
    manager
        .uninstall(&installed.id, true, &super::langs())
        .await
        .unwrap();
    assert_resources_removed(&engine, &names).await;
}

#[tokio::test]
#[ignore = "需要 Docker，webui.path 的 404 健康、503 巡检异常且恢复后重新健康"]
async fn a_webui_path_accepts_404_and_watch_degrades_on_503_then_recovers() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = busybox(&engine).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine.clone(), &dir, &names);
    let installed = manager
        .install(
            &compose(&image, Some("/cgi-bin/probe"), false),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        seen(&manager),
        (PluginStatus::Running, Some(Health::Healthy))
    );
    let container = names.container("app-test", "app");
    assert_no_docker_healthcheck(&engine, &container).await;
    let probe = manager
        .health_report(&installed.id)
        .await
        .unwrap()
        .probe
        .unwrap();
    assert!(probe.reachable);
    assert_eq!(probe.http_status, Some(404));

    set_failing(&engine, &container, true).await;
    manager.watch_containers().await;
    assert!(matches!(seen(&manager).0, PluginStatus::Degraded { .. }));
    let probe = manager
        .health_report(&installed.id)
        .await
        .unwrap()
        .probe
        .unwrap();
    assert!(!probe.reachable);
    assert_eq!(probe.http_status, Some(503));
    assert_eq!(probe.error_kind, Some("upstream_error"));
    // HTTP 异常没有停止容器，也没有 Docker health 事件；巡检必须实际重探 HTTP 才能恢复。
    assert_no_docker_healthcheck(&engine, &container).await;
    set_failing(&engine, &container, false).await;
    manager.watch_containers().await;
    assert_eq!(
        seen(&manager),
        (PluginStatus::Running, Some(Health::Healthy))
    );
    manager
        .uninstall(&installed.id, true, &super::langs())
        .await
        .unwrap();
    assert_resources_removed(&engine, &names).await;
}

async fn set_failing(engine: &Engine, container: &str, failing: bool) {
    let argv = if failing {
        vec!["touch".into(), "/www/failing".into()]
    } else {
        vec!["rm".into(), "-f".into(), "/www/failing".into()]
    };
    let result = engine
        .exec(container, &argv, None, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0, "{result:?}");
}

#[tokio::test]
#[ignore = "需要 Docker，网页 503 导致安装失败并回滚容器、网络、卷和安装记录"]
async fn a_webui_returning_503_rolls_back_every_install_resource() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = busybox(&engine).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine.clone(), &dir, &names);
    let error = manager
        .install(
            &compose(&image, Some("/cgi-bin/probe"), true),
            &[],
            super::langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "unhealthy", "{error}");
    assert!(error.to_string().contains("503"), "{error}");
    assert_resources_removed(&engine, &names).await;
    let store = Store::open(dir.path().join("plugins")).unwrap();
    let id = PluginId::parse("app.test").unwrap();
    assert!(store.load(&id).unwrap().is_none());
    assert!(store.load_all().unwrap().is_empty());
    // 失败报告可留给主人看，但不能再作为已安装插件恢复任何资源。
    let restored = manager_in_instance(engine.clone(), &dir, names.name());
    restored.restore().await.unwrap();
    assert_resources_removed(&engine, &names).await;
}
