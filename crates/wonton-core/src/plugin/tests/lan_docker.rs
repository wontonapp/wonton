//! 真实 Docker：地址变化后重跑一次性服务、更新内联配置，并保留无关服务和数据。
//! 这些测试验证行为不验证速度，期限和生产一致；共用 runner 忙时 Docker 起容器会慢。

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;

use super::super::docker::Engine;
use super::super::lan::{Detector, Lan};
use super::super::names::Instance;
use super::super::{PluginId, PluginManager, PluginStatus};
use super::manager::manager_in_instance;
use super::package;
use crate::config::DEFAULT_INSTALL_TIMEOUT;

const IMAGE: &str = "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

fn lan(address: IpAddr) -> Lan {
    let detector = Detector::Fixed(Arc::new(Mutex::new(Err("测试直接指定地址".into()))));
    Lan::new(detector, Some(vec![address]))
}

fn compose(port: u16) -> String {
    let services = format!(
        r#"  untouched:
    image: {IMAGE}
    init: true
    command: ['sleep', '3600']
  init:
    image: {IMAGE}
    restart: 'no'
    depends_on: [untouched]
    volumes: ['${{WONTON_APP_DATA}}:/data']
    command: ['sh', '-c', 'echo ${{WONTON_LAN_ADDRESSES}} >> /data/runs']
  web:
    image: {IMAGE}
    init: true
    depends_on:
      init: {{condition: service_completed_successfully}}
    configs: [addresses]
    command: ['sh', '-c', 'mkdir /www; cp /addresses /www/index.html; exec httpd -f -p {port} -h /www']
    healthcheck:
      test: ['CMD', 'wget', '-q', '-O', '/dev/null', 'http://127.0.0.1:{port}/']
      interval: 1s
      timeout: 1s
      start_interval: 1s
  lan:
    image: {IMAGE}
    init: true
    depends_on: [web]
    command: ['httpd', '-f', '-p', '{port}']"#
    );
    format!(
        "configs:\n  addresses:\n    content: '${{WONTON_LAN_ADDRESSES}}'\n{}",
        package(
            &services,
            &format!(
                "  permissions: [network.lan]\n  lan: {{ports: [{{service: lan, port: {port}}}]}}\n  webui: {{service: web, port: {port}, sso: none}}"
            )
        )
    )
}

async fn page(manager: &PluginManager) -> String {
    let route = manager.route("app-test").unwrap();
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(route.upstream.unwrap().url("/"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap()
}

async fn exercise(
    manager: &mut PluginManager,
    names: &Instance,
    addresses: [IpAddr; 2],
    dir: &std::path::Path,
    port: u16,
) {
    let engine = manager.engine.clone();
    let source = compose(port);
    let granted = super::lan::granted(&source);
    manager.install(&source, &granted).await.unwrap();
    assert_eq!(page(manager).await, addresses[0].to_string());
    let mut before = Vec::new();
    for service in ["init", "web", "lan", "untouched"] {
        before.push(
            engine
                .inspect_container(&names.container("app-test", service))
                .await
                .unwrap()
                .unwrap()
                .id,
        );
    }
    manager.lan = lan(addresses[1]);
    manager.lan_pass().await.unwrap();
    assert_eq!(manager.list()[0].status, PluginStatus::Running);
    assert_eq!(page(manager).await, addresses[1].to_string());
    for (service, id) in ["init", "web", "lan", "untouched"].into_iter().zip(before) {
        let after = engine
            .inspect_container(&names.container("app-test", service))
            .await
            .unwrap()
            .unwrap();
        if service == "untouched" {
            assert_eq!(id, after.id);
        } else {
            assert_ne!(id, after.id, "{service}");
        }
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("apps/app-test/runs")).unwrap(),
        format!("{}\n{}\n", addresses[0], addresses[1])
    );
    println!(
        "LAN: 页面 {} → {}；init/web/lan 已重建；untouched ID 未变；一次性服务依赖顺序与数据保留通过",
        addresses[0], addresses[1]
    );
}

#[tokio::test]
#[ignore = "需要 Docker，地址变化重建变量消费者并按依赖顺序重跑一次性服务"]
async fn lan_address_change_updates_real_containers_and_config_page() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let instance = format!("p173{:x}", getrandom::u32().unwrap());
    let names = Instance::parse(Some(&instance)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, &instance);
    manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let result = std::panic::AssertUnwindSafe(async {
        let mut addresses = Vec::new();
        for fixture in ["address-one", "address-two"] {
            let network = names.network(fixture);
            engine
                .create_relay_network(&network, &names.core_labels())
                .await
                .unwrap();
            addresses.push(
                engine
                    .network_gateway(&network)
                    .await
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
        }
        let addresses = [addresses[0], addresses[1]];
        manager.lan = lan(addresses[0]);
        exercise(&mut manager, &names, addresses, dir.path(), port).await;
    })
    .catch_unwind()
    .await;
    let _ = manager
        .uninstall(&PluginId::parse("app.test").unwrap(), false)
        .await;
    for service in ["init", "web", "lan", "untouched"] {
        engine
            .remove_container(&names.container("app-test", service))
            .await
            .unwrap();
    }
    engine
        .remove_network(&names.lan_network("app-test"))
        .await
        .unwrap();
    engine
        .remove_network(&names.network("app-test"))
        .await
        .unwrap();
    for fixture in ["address-one", "address-two"] {
        engine
            .remove_network(&names.network(fixture))
            .await
            .unwrap();
    }
    if dir.path().join("apps").exists() {
        std::fs::remove_dir_all(dir.path().join("apps")).unwrap();
    }
    println!("{instance}: 已删除本测试全部容器、网络和数据");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
