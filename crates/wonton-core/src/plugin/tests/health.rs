//! 一次性服务和健康状态：安装和核心重启后核对时，一次性服务“已退出、退出码 0”是正常状态；没有健康检查的
//! 服务只能说在运行，不能说健康。

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
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
