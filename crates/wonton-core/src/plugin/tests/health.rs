//! 一次性服务和健康状态：安装和核心重启后核对时，一次性服务“已退出、退出码 0”是正常状态；没有健康检查的
//! 服务只能说在运行，不能说健康。

use std::time::Duration;

use serde_json::{Value, json};

use crate::l10n::CATALOG;
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::super::compose::DependCondition;
use super::super::fake_docker::FakeDocker;
use super::super::health::{Readiness, Target, wait_for};
use super::super::health_gate::Wait;
use super::super::names::container_name;
use super::super::render::{RenderError, dry_run};
use super::super::{Health, PluginPackage, PluginStatus, PluginView};
use super::manager::manager_with;
use super::package;
use crate::plugin::languages::Languages;

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
    let view = m
        .install(&with_init(""), &[], super::langs())
        .await
        .unwrap();
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
    let view = m
        .install(&with_init(""), &[], super::langs())
        .await
        .unwrap();
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
        let err = dry_run(
            &PluginPackage::parse(&compose, &super::langs()).unwrap(),
            &super::langs(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, RenderError::Service { service, .. } if service == "init"),
            "{restart}: {err}"
        );
    }
    let plan = dry_run(
        &PluginPackage::parse(&with_init(""), &super::langs()).unwrap(),
        &super::langs(),
    )
    .unwrap();
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
    let plan = dry_run(
        &PluginPackage::parse(&compose, &super::langs()).unwrap(),
        &super::langs(),
    )
    .unwrap();
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
        m.install(&compose, &[], super::langs()).await.is_err(),
        "收尾失败就是没装好"
    );
    docker.set_state(&provision, exited(0));
    let view = m.install(&compose, &[], super::langs()).await.unwrap();
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
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let error = m
        .install(&with_init(""), &[], super::langs())
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
    let started = tokio::time::Instant::now();
    let budget = Duration::from_millis(100);
    let wait = Wait {
        at: started + budget,
        budget,
        started,
    };
    let error = wait_for(
        &engine,
        &crate::http::HttpClient::new(),
        &app,
        Target::Ready,
        None,
        wait,
        &Languages::default(),
    )
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
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let error = m
        .install(&with_init(""), &[], super::langs())
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
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let error = m
        .install(&with_init(""), &[], super::langs())
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
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let error = m
        .install(&with_init(""), &[], super::langs())
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
    let err = m.install(&compose, &[], super::langs()).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "一次性依赖失败要立即报错，不能等满超时：{err}"
    );
    let message = err.to_string();
    assert!(message.contains(&bootstrap), "{message}");
    assert!(
        message.contains(&CATALOG.format(
            CATALOG.negotiate(&["en"]),
            "plugin-settings-health-exited",
            &[
                ("container", bootstrap.as_str().into()),
                ("code", "1".into())
            ],
        )),
        "{message}"
    );
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
    let view = m.install(&compose, &[], super::langs()).await.unwrap();
    assert_eq!(view.status, PluginStatus::Running);
    flip.await.unwrap();
    assert!(
        !started_too_early.await.unwrap(),
        "init 在应用健康之前就被启动了"
    );
}

/// 一次性的初始化失败时，错误里带它的整段日志（几百行），不是最后几十行：真正的原因常在开头。
#[tokio::test]
async fn a_failed_install_carries_the_whole_log_not_just_the_last_lines() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let init = container_name("app-test", "init");
    docker.set_output(
        &init,
        1,
        "",
        &(1..=150)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    docker.set_state(&init, exited(1));
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let error = m
        .install(&with_init(""), &[], super::langs())
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("line 1\n"), "{message}");
    assert!(message.contains("line 150"), "{message}");
}

// 健康检查判成失败不等于没救：慢主机上的数据库冷启动会先探针全失败再恢复，安装要继续等，
// 不能一看到 unhealthy 就回滚（盲测 immich-db-start-timeout，核心 84f4092a3）。
#[tokio::test]
async fn an_unhealthy_container_that_recovers_is_not_failed_early() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&container_name("app-test", "init"), exited(0));
    docker.set_state(
        &app,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [{"ExitCode": 1, "Output": "no response"}]},
        }),
    );
    let m = manager_with(engine, &dir);
    // 钉住 inspect：第一眼必须真读到 unhealthy，不能靠调度碰运气。
    let first = docker.hold(&format!("GET /containers/{app}/json"));
    let install = tokio::spawn({
        let compose = with_init("");
        async move { m.install(&compose, &[], super::langs()).await }
    });
    first.reached().await;
    let second = docker.hold(&format!("GET /containers/{app}/json"));
    first.release();
    // 旧代码读到 unhealthy 当场失败；还能再来一眼才说明它接着等了。
    tokio::time::timeout(Duration::from_secs(10), second.reached())
        .await
        .expect("判成不健康后没有再看一眼");
    assert!(!install.is_finished(), "判成不健康也要继续等到期限");
    docker.set_state(
        &app,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    second.release();
    let view = install.await.unwrap().unwrap();
    assert_eq!(
        (view.status, view.health),
        (PluginStatus::Running, Some(Health::Healthy))
    );
}

#[tokio::test(start_paused = true)]
async fn a_pending_container_that_stays_unhealthy_is_named_when_the_deadline_expires() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = container_name("app-test", "db");
    docker.set_state(
        &db,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [{"ExitCode": 1, "Output": "no response"}]},
        }),
    );
    docker.set_state(
        &container_name("app-test", "app"),
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
    );
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::ZERO;
    let compose = package(
        "  db:\n    image: postgres\n    healthcheck: {test: ['CMD', 'pg_isready']}\n  \
         app:\n    image: nginx\n    depends_on:\n      db: {condition: service_healthy}\n",
        "  permissions: []",
    );
    let error = m
        .install(&compose, &[], super::langs())
        .await
        .unwrap_err()
        .to_string();
    // 报的是等着的那一方的依赖：只有 db 进了没就绪名单，还在启动的 app 不算。
    assert!(error.contains("no response"), "{error}");
    assert!(
        error.contains(&format!("and {db} never became ready")),
        "{error}"
    );
}

// `service_started` 只看“起来了”：健康检查失败也不妨碍这条依赖满足（商店里 socat 转发器、
// actual、uptime-kuma 都这样依赖后端）。
#[tokio::test]
async fn a_started_dependency_does_not_wait_for_its_health_check() {
    let (docker, engine) = FakeDocker::start().await;
    let db = container_name("app-test", "db");
    docker.add_container(&db, "app.db");
    docker.set_state(
        &db,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [{"ExitCode": 1, "Output": "no response"}]},
        }),
    );
    let started = tokio::time::Instant::now();
    let budget = Duration::from_secs(60);
    let wait = Wait {
        at: started + budget,
        budget,
        started,
    };
    let readiness = wait_for(
        &engine,
        &crate::http::HttpClient::new(),
        &db,
        Target::Depend(DependCondition::ServiceStarted),
        None,
        wait,
        &Languages::default(),
    )
    .await
    .unwrap();
    assert_eq!(readiness, Readiness::Running);
}
