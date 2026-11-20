//! Agent 生成应用用的 `assets/backend` 模板：健康检查用核心会改写的轻量探针，不走代理。

use std::collections::BTreeMap;
use std::path::Path;

use super::super::compose::Healthcheck;
use super::super::egress::{EgressVars, healthcheck};
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::render::{dry_run_context, render};

fn example(name: &str) -> PluginPackage {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugin-guide/assets")
        .join(name);
    PluginPackage::load_dir(&dir, &super::langs()).unwrap()
}

fn template() -> PluginPackage {
    example("backend")
}

fn healthcheck(package: &PluginPackage, service: &str) -> Healthcheck {
    let vars = super::super::vars::injected_vars(&package.manifest, &dry_run_context(package));
    let mut used = BTreeMap::new();
    let services = package
        .services(|_| &vars, &mut used, &super::langs())
        .unwrap();
    services[service]
        .healthcheck
        .clone()
        .expect("模板写了健康检查")
}

/// 模板里 `app` 服务的健康检查，原样。
fn app_healthcheck() -> Healthcheck {
    healthcheck(&template(), "app")
}

fn test_of(check: &Healthcheck) -> Vec<String> {
    match check.test.clone().expect("模板写了 test") {
        super::super::compose::StringOrList::One(one) => vec![one],
        super::super::compose::StringOrList::Many(many) => many,
    }
}

#[test]
fn the_backend_example_probes_with_a_command_the_rewrite_makes_proxy_free() {
    let check = app_healthcheck();
    let original = test_of(&check);
    assert!(
        !original.iter().any(|arg| arg.contains("python")),
        "{original:?}"
    );

    let rewritten = healthcheck_rewrite(&original);
    assert_eq!(
        &rewritten[..3],
        ["CMD", "wget", "-Y"],
        "wget 后面要能被核心加上 -Y off：{rewritten:?}"
    );
    assert_eq!(rewritten[3], "off");
    assert!(
        rewritten.windows(2).any(|pair| pair == ["-T", "3"]),
        "探针自己也要有超时：{rewritten:?}"
    );
    assert!(rewritten.contains(&"http://127.0.0.1:8080/healthz".to_owned()));
}

#[test]
fn the_backend_example_keeps_the_start_and_interval_settings() {
    let check = app_healthcheck();
    assert_eq!(check.interval.as_deref(), Some("30s"));
    assert_eq!(check.timeout.as_deref(), Some("5s"));
    assert_eq!(check.start_period.as_deref(), Some("10s"));
    assert_eq!(check.start_interval.as_deref(), Some("1s"));
}

/// 两个例子的探针都自带超时：`SIGSTOP` 这类卡住只靠 compose 的 `timeout` 要等满 5 秒，
/// `-T 3` 让 wget 自己先退。
#[test]
fn both_examples_bound_the_probe_themselves() {
    for (dir, service) in [("backend", "app"), ("static", "web")] {
        let test = test_of(&healthcheck(&example(dir), service));
        assert!(
            test.windows(2).any(|pair| pair == ["-T", "3"]),
            "{dir}: {test:?}"
        );
    }
}

/// 配了代理时渲染出的容器用改写后的探针，模板回归成 `python3 -c` 时这里跟着红。
#[test]
fn the_rendered_template_probes_without_python() {
    let mut package = template();
    package.manifest.permissions.push(Permission::NetworkEgress);
    let mut ctx = dry_run_context(&package);
    ctx.egress = Some(EgressVars { host_proxy: None });
    let plan = render(&package, &ctx, &super::langs()).unwrap();
    let check = plan.containers[0]
        .body
        .healthcheck
        .clone()
        .expect("渲染后的容器还有健康检查");
    let test = check.test.expect("渲染后的容器还有 test");
    assert!(!test.iter().any(|arg| arg.contains("python")), "{test:?}");
    assert!(
        test.windows(3).any(|w| w == ["wget", "-Y", "off"]),
        "{test:?}"
    );
}

fn healthcheck_rewrite(test: &[String]) -> Vec<String> {
    healthcheck::rewrite(test).expect("模板的写法必须由核心改写")
}
