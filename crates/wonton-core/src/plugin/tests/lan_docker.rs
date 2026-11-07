//! 真实 Docker：地址变化后重跑一次性服务、更新内联配置，并保留无关服务和数据。
//! 这些测试验证行为不验证速度，期限和生产一致；共用 runner 忙时 Docker 起容器会慢。

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};

use super::super::compose::PullPolicy;
use super::super::docker::Engine;
use super::super::lan::{Detector, Lan};
use super::super::{PluginId, PluginManager, PluginStatus};
use super::manager::manager_in_instance;
use super::package;
use crate::config::DEFAULT_INSTALL_TIMEOUT;
use crate::test_docker::Names;

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
    names: &Names,
    addresses: [IpAddr; 2],
    dir: &std::path::Path,
    port: u16,
) {
    let engine = manager.engine.clone();
    let source = compose(port);
    let granted = super::lan::granted(&source);
    manager
        .install(&source, &granted, super::langs())
        .await
        .unwrap();
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
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut addresses = Vec::new();
    for fixture in ["address-one", "address-two"] {
        let network = names.instance().network(fixture);
        engine
            .create_relay_network(&network, &names.instance().core_labels())
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
    let _ = manager
        .uninstall(
            &PluginId::parse("app.test").unwrap(),
            false,
            &super::langs(),
        )
        .await;
}

/// 宿主上已经被别的容器发布着的端口，装的时候立刻就报、类别是端口被占，不是等超时。占着的容器是核心不认识的。
#[tokio::test]
#[ignore = "需要 Docker，端口被占时安装立刻失败"]
async fn a_port_published_by_another_container_fails_the_install_right_away() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let mut names = Names::new();
    // 端口开在 Docker 默认网桥的网关上：那也是一个宿主地址，不用碰真网卡。
    let network = names.instance().network("port-taken");
    engine
        .create_relay_network(&network, &names.instance().core_labels())
        .await
        .unwrap();
    let address: IpAddr = engine
        .network_gateway(&network)
        .await
        .unwrap()
        .parse()
        .unwrap();
    // 别的容器占着 0.0.0.0 上的一个端口，和报出来的现象一样。宿主端口让 Docker 分配、事后再问它：先在测试里绑一个
    // 再放掉，放掉到 Docker 绑上之间，并行的测试或别的进程可能拿走同一个端口。
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    let hold = names.oneshot_container("occupy");
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec![
            "httpd".to_owned(),
            "-f".to_owned(),
            "-p".to_owned(),
            "8000".to_owned(),
        ]),
        exposed_ports: Some(vec!["8000/tcp".to_owned()]),
        host_config: Some(HostConfig {
            port_bindings: Some(
                [(
                    "8000/tcp".to_owned(),
                    Some(vec![PortBinding {
                        host_ip: Some("0.0.0.0".to_owned()),
                        host_port: None,
                    }]),
                )]
                .into(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&hold, body).await.unwrap();
    engine.start_container(&hold).await.unwrap();
    let port: u16 = engine
        .inspect_container(&hold)
        .await
        .unwrap()
        .and_then(|info| info.network_settings?.ports?.remove("8000/tcp")?)
        .and_then(|bindings| {
            bindings
                .into_iter()
                .find_map(|b| b.host_port.filter(|p| !p.is_empty()))
        })
        .expect("Docker 给占位容器发布了端口")
        .parse()
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    manager.lan = lan(address);
    let source = compose(port);
    let started = Instant::now();
    let error = manager
        .install(&source, &super::lan::granted(&source), super::langs())
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.kind(), "port_taken", "{error}");
    assert!(error.to_string().contains(&port.to_string()), "{error}");
    assert!(
        elapsed < Duration::from_secs(20),
        "用了 {elapsed:?}，像是等超时了"
    );
    assert_eq!(manager.list().len(), 0, "没留下插件");
    assert!(
        engine
            .inspect_container(&names.container("app.test", "lan"))
            .await
            .unwrap()
            .is_none()
    );
}
