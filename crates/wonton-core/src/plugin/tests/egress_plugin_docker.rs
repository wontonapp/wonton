//! 真实代理提供方、出口容器和出网插件：上游停掉后回 503，不改成直连。
//! 这些测试验证行为不验证速度，期限和生产一致；共用 runner 忙时 Docker 起容器会慢。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig};
use wonton_protocol::{EgressFailure, EgressProxyUpdate};

use super::super::core_address::Endpoint;
use super::super::docker::Engine;
use super::super::egress::gost;
use super::super::egress::probe::Prober;
use super::super::egress::settings::Egress;
use super::super::lan::{Lan, detect::Detector};
use super::super::languages::Languages;
use super::super::names::Instance;
use super::super::{PluginManager, PluginPackage};
use super::lan::{Host, office};
use super::manager::manager_in_instance;
use super::package;
use crate::config::DEFAULT_INSTALL_TIMEOUT;
use crate::test_docker::Names;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn busybox() -> String {
    wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
}

async fn install(m: &PluginManager, source: &str) {
    let granted = PluginPackage::parse(source, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    m.install(source, &granted, super::langs()).await.unwrap();
}

async fn fetch(engine: &Engine, client: &str) -> Result<String, String> {
    let argv = [
        "wget",
        "-q",
        "-T",
        "20",
        "-O",
        "-",
        "http://far.example.test/",
    ]
    .map(str::to_owned);
    let output = engine
        .exec_output(client, &argv, None, Duration::from_secs(30))
        .await
        .unwrap();
    if output.exit_code == 0 {
        Ok(output.stdout)
    } else {
        Err(output.stderr)
    }
}

async fn exercise(m: &Arc<PluginManager>, engine: &Engine, names: &Instance) {
    let proxy = package(
        &format!(
            "  proxy:\n    image: {}\n    command: ['-L', 'http://:7890']",
            gost::image()
        ),
        "  provides:\n    egress_proxy: {service: proxy, port: 7890}\n  permissions: [network.egress, egress.proxy]",
    ).replace("id: app.test", "id: app.proxy");
    install(m, &proxy).await;
    m.set_egress_proxy(
        EgressProxyUpdate {
            url: None,
            use_plugin: true,
            direct_cidrs: Vec::new(),
            restarts: Vec::new(),
        },
        Languages::default(),
    )
    .await
    .unwrap();
    let far_net = names.network("far");
    engine
        .create_relay_network(&far_net, &names.core_labels())
        .await
        .unwrap();
    let far = names.container("far", "web");
    engine.create_container(&far, ContainerCreateBody {
        image: Some(busybox()),
        cmd: Some(["sh", "-c", "mkdir -p /www; echo proxy-only > /www/index.html; exec httpd -f -p 80 -h /www"].map(str::to_owned).to_vec()),
        labels: Some(names.core_labels()),
        host_config: Some(HostConfig { network_mode: Some(far_net.clone()), ..Default::default() }),
        ..Default::default()
    }).await.unwrap();
    engine.start_container(&far).await.unwrap();
    engine.disconnect_network(&far_net, &far).await.unwrap();
    engine
        .connect_network_at(
            &far_net,
            &far,
            Endpoint {
                aliases: vec!["far.example.test".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let provider = names.container("app-proxy", "proxy");
    engine
        .connect_network_at(&far_net, &provider, Endpoint::default())
        .await
        .unwrap();
    let consumer = package(
        &format!(
            "  client:\n    image: {}\n    command: ['sleep', '3600']",
            busybox()
        ),
        "  permissions: [network.egress]",
    )
    .replace("id: app.test", "id: app.out");
    install(m, &consumer).await;
    let client = names.container("app-out", "client");
    let body = fetch(engine, &client).await.unwrap();
    assert_eq!(body, "proxy-only");
    let env = engine
        .inspect_container(&provider)
        .await
        .unwrap()
        .unwrap()
        .config
        .unwrap()
        .env
        .unwrap_or_default();
    assert!(!env.iter().any(|v| v.starts_with("HTTP_PROXY=")));
    engine.remove_container(&provider).await.unwrap();
    m.probe_egress(&Languages::default()).await.unwrap();
    assert_eq!(
        m.egress_problem().unwrap().reason,
        EgressFailure::ProxyUnreachable
    );
    let failed = fetch(engine, &client).await.unwrap_err();
    assert!(failed.contains("503"), "{failed}");
    let package =
        PluginPackage::parse(&consumer.replace("app.out", "app.blocked"), &super::langs()).unwrap();
    assert_eq!(
        m.install_package(
            package.clone(),
            &package.manifest.permissions,
            super::langs()
        )
        .await
        .unwrap_err()
        .kind(),
        "egress_proxy_unavailable"
    );
    println!(
        "{}: 插件经 gost → 代理服务容器名 → 独占目标返回 proxy-only；代理容器删除后 proxy_unreachable，客户端 503，安装被挡",
        names.egress()
    );
}

#[tokio::test]
#[ignore = "需要 Docker，真实代理插件经 gost 转发并在上游停止后回 503"]
async fn egress_plugin_relay_uses_provider_dns_and_fails_closed() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    for image in [gost::image(), busybox()] {
        crate::test_docker::ensure_image(&engine, &image).await;
    }
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let host: Host = Arc::new(Mutex::new(Ok(office())));
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name())
        .with_lan(Lan::new(Detector::Fixed(host), None))
        .with_egress(Egress::default().with_prober(Prober::Fine));
    manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    exercise(&Arc::new(manager), &engine, names.instance()).await;
}
