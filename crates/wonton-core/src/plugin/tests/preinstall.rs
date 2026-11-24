//! 预装默认插件：只装一次，主人卸掉的不装回来；装不上时按原因重试或停下，等待期间如实显示异常。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::image_worker::ImageWorker;
use super::super::lan::Lan;
use super::super::lan::detect::Detector;
use super::super::names::container_name;
use super::super::preinstall::Backoff;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::lan::{Host, add_default_bridge, office};
use super::manager::{manager_with, manager_with_images};
use super::package;

const QUICK: Backoff = Backoff {
    first: Duration::from_millis(200),
    max: Duration::from_millis(200),
};
/// 测试期间不会到点重试。
const NEVER: Backoff = Backoff {
    first: Duration::from_secs(3600),
    max: Duration::from_secs(3600),
};

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

/// 只有一个默认插件（`app.test`）的默认插件目录。
fn defaults(dir: &tempfile::TempDir, extra_manifest: &str) -> PathBuf {
    let defaults = dir.path().join("defaults");
    fs::create_dir_all(defaults.join("test")).unwrap();
    let compose = package("  web:\n    image: nginx", extra_manifest);
    fs::write(defaults.join("test/docker-compose.yml"), compose).unwrap();
    defaults
}

fn status(m: &PluginManager) -> Option<PluginStatus> {
    m.list()
        .into_iter()
        .find(|p| p.id == id())
        .map(|p| p.status)
}

/// 等到插件的状态满足 `wanted`，最多十秒。
async fn until(
    m: &PluginManager,
    wanted: impl Fn(&Option<PluginStatus>) -> bool,
) -> Option<PluginStatus> {
    for _ in 0..1000 {
        let now = status(m);
        if wanted(&now) {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件一直没到想要的状态，最后是 {:?}", status(m));
}

fn degraded(status: &Option<PluginStatus>) -> bool {
    matches!(status, Some(PluginStatus::Degraded { .. }))
}

/// 异常，而且已经重试过一次。
fn retried(status: &Option<PluginStatus>) -> bool {
    matches!(status, Some(PluginStatus::Degraded { reason }) if reason.contains("tried 2 time(s)"))
}

fn lan() -> Lan {
    let host: Host = Arc::new(Mutex::new(Ok(office())));
    Lan::new(Detector::Fixed(host), None)
}

fn exited() -> Value {
    json!({"Status": "exited", "Running": false, "ExitCode": 1})
}

fn creates(docker: &FakeDocker) -> usize {
    let create = "POST /containers/create";
    docker.writes().iter().filter(|w| *w == create).count()
}

/// 核心重启：新的管理器按记录恢复后再预装一遍。
async fn restart(engine: &Engine, dir: &tempfile::TempDir, defaults: &Path) -> PluginManager {
    let m = manager_with(engine.clone(), dir);
    m.restore().await.unwrap();
    m.ensure_defaults(defaults, QUICK).await.unwrap();
    m
}

#[tokio::test]
async fn a_default_plugin_is_installed_once_and_stays_removed_after_the_owner_uninstalls_it() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    let revision = PluginPackage::load_dir(&defaults.join("test"), &super::langs())
        .unwrap()
        .digest();
    let m = manager_with(engine.clone(), &dir);
    m.ensure_defaults(&defaults, QUICK).await.unwrap();
    assert_eq!(status(&m), Some(PluginStatus::Running));
    assert_eq!(
        m.installed_plugin(&id(), &Default::default())
            .unwrap()
            .revision,
        revision
    );
    // 重启时它还装着：照常恢复，不重装。
    let m = restart(&engine, &dir, &defaults).await;
    assert_eq!(status(&m), Some(PluginStatus::Running));
    assert_eq!(creates(&docker), 1);
    assert_eq!(
        m.installed_plugin(&id(), &Default::default())
            .unwrap()
            .revision,
        revision
    );
    m.uninstall(&id(), false, &super::langs()).await.unwrap();
    let m = restart(&engine, &dir, &defaults).await;
    assert!(m.list().is_empty(), "主人卸掉的默认插件不能装回来");
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
}

#[tokio::test]
async fn a_failed_preinstall_is_shown_with_its_cause_and_retried_until_it_works() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    let web = container_name("app-test", "web");
    docker.set_state(&web, exited());
    let m = Arc::new(manager_with(engine, &dir));
    let task = tokio::spawn({
        let m = m.clone();
        async move { m.ensure_defaults(&defaults, QUICK).await }
    });
    let Some(PluginStatus::Degraded { reason }) = until(&m, degraded).await else {
        unreachable!()
    };
    assert!(
        reason.contains(&web),
        "原因要说出是哪个容器出的问题：{reason}"
    );
    docker.set_state(
        &web,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    until(&m, |s| *s == Some(PluginStatus::Running)).await;
    task.await.unwrap().unwrap();
    assert!(creates(&docker) >= 2, "失败后应当重试过");
}

/// 出网插件自己还在装：预装说清是"还没就绪、等着"，不是连不上；它起来以后预装自己装上。
#[tokio::test]
async fn a_preinstall_waits_for_the_egress_provider_to_come_up_and_then_installs() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: [network.egress]");
    let m = Arc::new(manager_with(engine.clone(), &dir).with_lan(lan()));
    // 主人选了一个出网插件，它自己还在装。
    let proxy = super::egress_plugin::proxy_app("app.proxy");
    let grants = PluginPackage::parse(&proxy, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(&proxy, &grants, super::langs()).await.unwrap();
    let langs = super::langs();
    m.set_egress_proxy(
        wonton_protocol::EgressProxyUpdate {
            url: None,
            direct_cidrs: Vec::new(),
            restarts: Vec::new(),
            use_plugin: true,
        },
        langs,
    )
    .await
    .unwrap();
    let provider = PluginId::parse("app.proxy").unwrap();
    m.plugins
        .write()
        .unwrap()
        .get_mut(&provider)
        .unwrap()
        .status = PluginStatus::Installing;
    engine
        .remove_container("wonton-app-proxy--proxy")
        .await
        .unwrap();

    let task = tokio::spawn({
        let (m, defaults) = (m.clone(), defaults.clone());
        async move { m.ensure_defaults(&defaults, QUICK).await }
    });
    let Some(PluginStatus::Degraded { reason }) = until(&m, degraded).await else {
        unreachable!()
    };
    // 原因说出是哪个插件、还在装，而且不是"连不上"。
    assert!(reason.contains("Test"), "{reason}");
    assert!(reason.contains("still being installed"), "{reason}");
    assert!(!reason.contains("not running"), "{reason}");

    // 出网插件装好、跑起来了：预装接着装上。
    docker.add_container("wonton-app-proxy--proxy", "app.proxy");
    m.plugins
        .write()
        .unwrap()
        .get_mut(&provider)
        .unwrap()
        .status = PluginStatus::Running;
    until(&m, |s| *s == Some(PluginStatus::Running)).await;
    task.await.unwrap().unwrap();
}

/// 系统插件（如以前预装的模型网关）一直装不上、一直在等重试，默认插件照样装上（#643、#681）。
#[tokio::test]
async fn a_system_plugin_stuck_retrying_does_not_hold_back_the_default_plugins() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    let system = dir.path().join("system");
    fs::create_dir_all(system.join("gateway")).unwrap();
    let gateway = package("  web:\n    image: nginx", "  permissions: []")
        .replace("id: app.test", "id: app.gateway")
        .replace("kind: app", "kind: system");
    fs::write(system.join("gateway/docker-compose.yml"), gateway).unwrap();
    docker.set_state(&container_name("app-gateway", "web"), exited());
    let m = Arc::new(manager_with(engine, &dir));
    let task = tokio::spawn({
        let m = m.clone();
        async move {
            m.preinstall_bundled(Some(&system), Some(&defaults), NEVER)
                .await;
        }
    });
    until(&m, |s| *s == Some(PluginStatus::Running)).await;
    let gateway = PluginId::parse("app.gateway").unwrap();
    assert!(matches!(
        m.status_of(&gateway),
        Some(PluginStatus::Degraded { .. })
    ));
    task.abort();
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_is_named_in_the_reason_until_the_pull_works() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    let cause = "Get \"https://registry-1.docker.io/v2/\": dial tcp: lookup registry-1.docker.io: no such host";
    docker.fail_pulls(Some(cause));
    let m = Arc::new(manager_with(engine, &dir));
    let task = tokio::spawn({
        let m = m.clone();
        async move { m.ensure_defaults(&defaults, QUICK).await }
    });
    let Some(PluginStatus::Degraded { reason }) = until(&m, degraded).await else {
        unreachable!()
    };
    // 镜像名和 Docker 的原话都要在原因里（plugin-install-docker-pull-failed）。
    let expected = super::message(
        "plugin-install-docker-pull-failed",
        &[("image", "nginx"), ("error", cause)],
    );
    assert!(reason.contains(&expected), "{reason}");
    // 局域网状态页列出它，用插件的名字。
    let shown = m.bundled_not_running();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title.get("zh").map(String::as_str), Some("测试"));
    docker.fail_pulls(None);
    until(&m, |s| *s == Some(PluginStatus::Running)).await;
    task.await.unwrap().unwrap();
    assert!(m.bundled_not_running().is_empty());
}

/// 核心起不了处理图片的子进程，和插件包里的图标无关：照样等着重试，不像插件包本身的问题那样停下。
#[tokio::test]
async fn a_preinstall_is_retried_while_the_core_cannot_start_its_image_worker() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []\n  icon: icon.svg");
    fs::write(defaults.join("test/icon.svg"), "<svg/>").unwrap();
    let missing = ImageWorker::new(dir.path().join("wonton-core"));
    let m = Arc::new(manager_with_images(engine, &dir, missing));
    let task = tokio::spawn({
        let m = m.clone();
        async move { m.ensure_defaults(&defaults, QUICK).await }
    });
    let Some(PluginStatus::Degraded { reason }) = until(&m, retried).await else {
        unreachable!()
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-img-worker-unavailable",
            &[("error", "")],
        )),
        "{reason}"
    );
    assert!(reason.contains("tried 2 time(s)"), "{reason}");
    assert!(reason.contains("will retry automatically"), "{reason}");
    task.abort();
}

#[tokio::test]
async fn uninstalling_a_failed_preinstall_stops_the_retries_for_good() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    docker.set_state(&container_name("app-test", "web"), exited());
    let m = Arc::new(manager_with(engine.clone(), &dir));
    let task = tokio::spawn({
        let (m, defaults) = (m.clone(), defaults.clone());
        async move { m.ensure_defaults(&defaults, NEVER).await }
    });
    until(&m, degraded).await;
    m.uninstall(&id(), false, &super::langs()).await.unwrap();
    // 不等下次重试的时间到：主人一卸掉就停下。
    let stopped = tokio::time::timeout(Duration::from_secs(5), task).await;
    stopped.expect("卸掉后应当马上停止重试").unwrap().unwrap();
    assert!(m.list().is_empty());
    let m = restart(&engine, &dir, &defaults).await;
    assert!(m.list().is_empty(), "主人卸掉的默认插件不能装回来");
    assert_eq!(creates(&docker), 1);
}

#[tokio::test]
async fn a_retry_due_while_the_owner_is_uninstalling_does_not_bring_the_plugin_back() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir, "  permissions: []");
    let web = container_name("app-test", "web");
    docker.set_state(&web, exited());
    let m = Arc::new(manager_with(engine.clone(), &dir));
    // 留够时间让卸载在到点之前开始。
    let second = Backoff {
        first: Duration::from_secs(1),
        max: Duration::from_secs(1),
    };
    let task = tokio::spawn({
        let (m, defaults) = (m.clone(), defaults.clone());
        async move { m.ensure_defaults(&defaults, second).await }
    });
    until(&m, degraded).await;
    // Docker 暂停回应：卸载停在删容器之前，重试的时间在这期间到了。
    let paused = docker.pause().await;
    let uninstall = tokio::spawn({
        let m = m.clone();
        async move { m.uninstall(&id(), false, &super::langs()).await }
    });
    until(&m, |s| *s == Some(PluginStatus::Uninstalling)).await;
    let stopped = tokio::time::timeout(Duration::from_secs(5), task).await;
    stopped.expect("卸载开始后不再重试").unwrap().unwrap();
    drop(paused);
    uninstall.await.unwrap().unwrap();
    // 再装的话这次能装上。
    docker.set_state(&web, json!({"Status": "running", "Running": true}));
    let m = restart(&engine, &dir, &defaults).await;
    assert!(m.list().is_empty(), "主人卸掉的默认插件不能装回来");
    assert_eq!(creates(&docker), 1);
}

#[tokio::test]
async fn a_package_that_cannot_run_here_is_not_retried_but_shown_at_every_start() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    // 假 Docker 是 x86_64。
    let defaults = defaults(
        &dir,
        "  permissions: []\n  requirements:\n    arch: [arm64]",
    );
    let m = manager_with(engine.clone(), &dir);
    // 重试的话这里要等一小时。
    m.ensure_defaults(&defaults, NEVER).await.unwrap();
    let Some(PluginStatus::Degraded { reason }) = status(&m) else {
        panic!("应当标为异常：{:?}", status(&m));
    };
    assert!(reason.contains("x86_64"), "{reason}");
    let m = restart(&engine, &dir, &defaults).await;
    assert!(degraded(&status(&m)), "下次启动再试一次，仍如实显示");
    assert!(docker.writes().is_empty(), "{:?}", docker.writes());
}

/// 没有可用的显卡要主人装驱动、插显卡，到点重试也一样。
#[tokio::test]
async fn a_default_plugin_needing_a_gpu_the_host_lacks_is_not_retried() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let gpu = "  gpu: {services: [web]}\n  permissions: [devices.gpu]";
    let defaults = defaults(&dir, gpu);
    let host = FakeHost::empty();
    let m = manager_with(engine, &dir).with_host(host.paths());
    // 重试的话这里要等一小时。
    m.ensure_defaults(&defaults, NEVER).await.unwrap();
    let Some(PluginStatus::Degraded { reason }) = status(&m) else {
        panic!("应当标为异常：{:?}", status(&m));
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-host-gpu-missing",
            &[(
                "card",
                &super::message("plugin-install-req-card-generic", &[])
            )]
        )),
        "{reason}"
    );
    assert!(!reason.contains("自动重试"), "{reason}");
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
}
