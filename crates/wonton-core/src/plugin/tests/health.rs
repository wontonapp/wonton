//! 一次性服务和健康状态：安装和核心重启后核对时，一次性服务“已退出、退出码 0”是正常状态；没有健康检查的
//! 服务只能说在运行，不能说健康。

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::super::fake_docker::FakeDocker;
use super::super::health::{Target, wait_for};
use super::super::names::container_name;
use super::super::render::{RenderError, dry_run};
use super::super::{Health, PluginPackage, PluginStatus, PluginView};
use super::manager::manager_with;
use super::package;

/// `init` 是一次性服务，`app` 等它以 0 退出后启动。
fn with_init(app_extra: &str) -> String {
    package(
        &format!(
            "  init:\n    image: busybox\n    network_mode: none\n    restart: 'no'\n  \
             app:\n    image: nginx\n    depends_on:\n      init: {{condition: service_completed_successfully}}\n{app_extra}"
        ),
        "  permissions: []",
    )
}

fn exited(code: i64) -> Value {
    json!({"Status": "exited", "Running": false, "ExitCode": code})
}

fn only(views: Vec<PluginView>) -> (PluginStatus, Option<Health>) {
    let [view] = <[PluginView; 1]>::try_from(views).expect("只有一个插件");
    (view.status, view.health)
}

#[tokio::test]
async fn an_exited_one_shot_service_is_normal_after_install_and_restart() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let init = container_name("app-test", "init");
    docker.set_state(&init, exited(0));
    let m = manager_with(engine.clone(), &dir);
    let view = m.install(&with_init(""), &[]).await.unwrap();
    assert_eq!(
        (view.status, view.health),
        (PluginStatus::Running, Some(Health::Healthy))
    );

    let restarted = manager_with(engine.clone(), &dir);
    restarted.restore().await.unwrap();
    assert_eq!(
        only(restarted.list()),
        (PluginStatus::Running, Some(Health::Healthy))
    );

    // 一次性服务没有以 0 退出：如实标为异常。
    docker.set_state(&init, exited(1));
    let failed = manager_with(engine, &dir);
    failed.restore().await.unwrap();
    let (status, health) = only(failed.list());
    assert!(
        matches!(status, PluginStatus::Degraded { .. }),
        "{status:?}"
    );
    assert_eq!(health, None);
}

#[tokio::test]
async fn services_without_a_healthcheck_are_running_with_health_unknown() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), exited(0));
    let app = container_name("app-test", "app");
    docker.set_state(&app, json!({"Status": "running", "Running": true}));
    let m = manager_with(engine.clone(), &dir);
    let view = m.install(&with_init(""), &[]).await.unwrap();
    assert_eq!(
        (view.status, view.health),
        (PluginStatus::Running, Some(Health::Unknown))
    );
    let restarted = manager_with(engine.clone(), &dir);
    restarted.restore().await.unwrap();
    assert_eq!(
        only(restarted.list()),
        (PluginStatus::Running, Some(Health::Unknown))
    );

    // 健康检查还没通过（核心比插件先起来）也只能说在运行；检查失败则是异常。
    let starting = json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}});
    docker.set_state(&app, starting);
    let early = manager_with(engine.clone(), &dir);
    early.restore().await.unwrap();
    assert_eq!(
        only(early.list()),
        (PluginStatus::Running, Some(Health::Unknown))
    );
    let unhealthy =
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}});
    docker.set_state(&app, unhealthy);
    let sick = manager_with(engine, &dir);
    sick.restore().await.unwrap();
    assert!(matches!(only(sick.list()).0, PluginStatus::Degraded { .. }));
}

#[test]
fn one_shot_services_must_not_be_restarted() {
    for restart in ["", "    restart: always\n", "    restart: unless-stopped\n"] {
        let compose = with_init("").replace("    restart: 'no'\n", restart);
        let err = dry_run(&PluginPackage::parse(&compose).unwrap()).unwrap_err();
        assert!(
            matches!(&err, RenderError::Service { service, .. } if service == "init"),
            "{restart}: {err}"
        );
    }
    let plan = dry_run(&PluginPackage::parse(&with_init("")).unwrap()).unwrap();
    let one_shot: Vec<(&str, bool)> = plan
        .containers
        .iter()
        .map(|c| (c.service.as_str(), c.one_shot))
        .collect();
    assert_eq!(one_shot, [("init", true), ("app", false)]);
}

#[tokio::test]
async fn a_one_shot_service_nobody_depends_on_still_has_to_finish() {
    let compose = package(
        "  app:\n    image: nginx\n  provision:\n    image: busybox\n    restart: 'no'\n    \
         depends_on:\n      app: {condition: service_started}",
        "  permissions: []",
    );
    let plan = dry_run(&PluginPackage::parse(&compose).unwrap()).unwrap();
    let one_shot: Vec<(&str, bool)> = plan
        .containers
        .iter()
        .map(|c| (c.service.as_str(), c.one_shot))
        .collect();
    assert_eq!(one_shot, [("app", false), ("provision", true)]);

    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let provision = container_name("app-test", "provision");
    docker.set_state(&provision, exited(1));
    let m = manager_with(engine, &dir);
    assert!(
        m.install(&compose, &[]).await.is_err(),
        "收尾失败就是没装好"
    );
    docker.set_state(&provision, exited(0));
    let view = m.install(&compose, &[]).await.unwrap();
    assert_eq!(view.status, PluginStatus::Running);
}

// 健康检查失败时，错误里要有探针自己的输出（存储探针、curl -f 的原因在 Docker 的 `State.Health.Log`
// 里，作者不该再去 `docker inspect`），和日志尾巴并列。
#[tokio::test]
async fn install_failure_names_the_failing_probes() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), exited(0));
    let app = container_name("app-test", "app");
    docker.set_state(
        &app,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "FailingStreak": 4, "Log": [
                {"ExitCode": 2, "Output": "最早的一次，不该出现在错误里"},
                {"ExitCode": 1, "Output": "curl: (7) Failed to connect to 127.0.0.1 port 1"},
                {"ExitCode": 7, "Output": "curl: (7) Couldn't connect to server"},
                {"ExitCode": 1, "Output": "存储卷不可写"},
            ]},
        }),
    );
    docker.set_output(&app, 0, "插件日志：数据库迁移失败", "");
    let m = manager_with(engine, &dir);
    let error = m
        .install(&with_init(""), &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("curl: (7) Failed to connect to 127.0.0.1 port 1"),
        "{error}"
    );
    assert!(error.contains("存储卷不可写"), "{error}");
    assert!(error.contains("插件日志：数据库迁移失败"), "{error}");
    assert!(!error.contains("最早的一次"), "{error}");
}

// 超时同样带探针输出，和日志尾巴并列。
#[tokio::test]
async fn timeout_also_names_the_failing_probes() {
    let (docker, engine) = FakeDocker::start().await;
    let app = container_name("app-test", "app");
    docker.add_container(&app, "app.probe-fail");
    docker.set_state(
        &app,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "starting", "Log": [{"ExitCode": 1, "Output": "探针一直失败"}]},
        }),
    );
    docker.set_output(&app, 0, "插件日志：还在初始化", "");
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    let error = wait_for(&engine, &app, Target::Ready, deadline)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("探针一直失败"), "{error}");
    assert!(error.contains("插件日志：还在初始化"), "{error}");
}

// 探针输出去掉控制字符（终端转义、回车），超过 1 KiB 的按字符边界截掉。
#[tokio::test]
async fn probe_output_is_trimmed_and_truncated() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), exited(0));
    let app = container_name("app-test", "app");
    let output = format!("HEAD\u{1b}[31m红\u{7}\r\n{}END-MARKER", "前".repeat(600));
    docker.set_state(
        &app,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [{"ExitCode": 1, "Output": output}]},
        }),
    );
    let m = manager_with(engine, &dir);
    let error = m
        .install(&with_init(""), &[])
        .await
        .unwrap_err()
        .to_string();
    // 颜色序列整个去掉，不留 [31m 这样的残渣；控制字符去掉（回车），换行和制表保留。
    assert!(error.contains("HEAD红\n"), "{error}");
    assert!(!error.contains("[31m"), "{error}");
    assert!(!error.contains('\u{1b}'), "{error}");
    assert!(!error.contains('\r'), "{error}");
    assert!(error.contains(&"前".repeat(300)), "{error}");
    assert!(!error.contains("END-MARKER"), "{error}");
}

// ANSI 转义序列整个去掉：`\x1b[31mred\x1b[0m` 只剩 red，结尾的复位序列也不留残渣。
#[tokio::test]
async fn ansi_sequences_are_stripped_entirely() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), exited(0));
    let app = container_name("app-test", "app");
    docker.set_state(
        &app,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy",
                       "Log": [{"ExitCode": 1, "Output": "\u{1b}[31mred\u{1b}[0m"}]},
        }),
    );
    let m = manager_with(engine, &dir);
    let error = m
        .install(&with_init(""), &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("] red\n"), "{error}");
    assert!(!error.contains("[31m") && !error.contains("[0m"), "{error}");
    assert!(!error.contains('\u{1b}'), "{error}");
}

// 没有 Health.Log 的旧 Docker：错误和以前一样，只有日志尾巴。
#[tokio::test]
async fn failures_without_probe_log_read_as_before() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), exited(0));
    let app = container_name("app-test", "app");
    docker.set_state(
        &app,
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
    );
    docker.set_output(&app, 0, "插件日志：只写了一半", "");
    let m = manager_with(engine, &dir);
    let error = m
        .install(&with_init(""), &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("插件日志：只写了一半"), "{error}");
    assert!(!error.contains("最近探针"), "{error}");
}

/// 依赖里既有迟迟不健康的长期服务、又有已经失败的一次性服务时，不等长期服务到超时：
/// 安装立即失败，报的是失败的那个（名字、退出码、日志）。
#[tokio::test]
async fn a_failed_one_shot_dependency_fails_the_install_immediately() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    let bootstrap = container_name("app-test", "bootstrap");
    docker.set_state(
        &app,
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
    );
    docker.set_state(&bootstrap, exited(1));
    docker.set_output(&bootstrap, 1, "", "初始化失败");
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::from_secs(300);
    let compose = package(
        "  app:\n    image: nginx\n    healthcheck: {test: ['CMD', 'true']}\n  \
         bootstrap:\n    image: busybox\n    network_mode: none\n    restart: 'no'\n  \
         web:\n    image: nginx\n    depends_on:\n      app: {condition: service_healthy}\n      \
         bootstrap: {condition: service_completed_successfully}",
        "  permissions: []",
    );
    let started = std::time::Instant::now();
    let err = m.install(&compose, &[]).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "一次性依赖失败要立即报错，不能等满超时：{err}"
    );
    let message = err.to_string();
    assert!(message.contains(&bootstrap), "{message}");
    assert!(message.contains("退出码 1"), "{message}");
    assert!(message.contains("初始化失败"), "{message}");
    assert!(
        !message.contains(&app),
        "报的应该是失败的一次性服务，不是还没就绪的长期服务：{message}"
    );
}

/// 一次性服务可以依赖长期服务的 `service_healthy`：应用健康之后才轮到它启动。
#[tokio::test]
async fn a_one_shot_service_starts_only_after_the_long_running_service_is_healthy() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    let init = container_name("app-test", "init");
    docker.set_state(
        &app,
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
    );
    docker.set_state(&init, exited(0));
    let compose = package(
        "  app:\n    image: nginx\n    healthcheck: {test: ['CMD', 'true']}\n  \
         init:\n    image: busybox\n    network_mode: none\n    restart: 'no'\n    \
         depends_on: {app: {condition: service_healthy}}",
        "  permissions: []",
    );
    let m = manager_with(engine, &dir);
    let hold = docker.hold(&format!("POST /containers/{init}/start"));
    // 应用 1 秒后才健康：init 的启动请求必须在那之后才到。
    let (flipped_tx, mut flipped_rx) = oneshot::channel();
    let flip = tokio::spawn({
        let docker = docker.clone();
        async move {
            sleep(Duration::from_secs(1)).await;
            docker.set_state(
                &app,
                json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
            );
            flipped_tx.send(()).unwrap();
        }
    });
    let started_too_early = tokio::spawn(async move {
        hold.reached().await;
        let early = flipped_rx.try_recv().is_err();
        hold.release();
        early
    });
    let view = m.install(&compose, &[]).await.unwrap();
    assert_eq!(view.status, PluginStatus::Running);
    flip.await.unwrap();
    assert!(
        !started_too_early.await.unwrap(),
        "init 在应用健康之前就被启动了"
    );
}

/// 规范：长期运行的服务都要有健康检查，官方目录的插件缺了不收录。`plugins/examples/` 不进官方目录，
/// 其中 whoami 的镜像里没有任何可以用来检查的命令。插件草稿用到核心还没实现的字段，这里直接读 YAML。
#[test]
fn official_plugins_check_the_health_of_every_long_running_service() {
    let plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut checked = Vec::new();
    for entry in fs::read_dir(&plugins).unwrap() {
        let dir = entry.unwrap().path();
        let compose = dir.join("docker-compose.yml");
        if dir.ends_with("examples") || !compose.is_file() {
            continue;
        }
        let doc: Value = serde_saphyr::from_str(&fs::read_to_string(&compose).unwrap()).unwrap();
        let services = doc["services"].as_object().expect("compose 里有服务");
        for (name, service) in services.iter().filter(|(_, s)| s["restart"] != "no") {
            let check = &service["healthcheck"];
            assert!(
                !check["test"].is_null() && check["disable"] != true,
                "{}：长期运行的服务 {name} 没有健康检查",
                compose.display()
            );
            checked.push(name.clone());
        }
    }
    assert!(!checked.is_empty(), "{} 下没有找到插件", plugins.display());
}
