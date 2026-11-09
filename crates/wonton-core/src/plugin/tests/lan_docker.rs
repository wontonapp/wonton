//! 真实 Docker：地址变化后重跑一次性服务、更新内联配置，并保留无关服务和数据。
//! 这些测试验证行为不验证速度，期限和生产一致；共用 runner 忙时 Docker 起容器会慢。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bollard::models::{ContainerConfig, ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{CommitContainerOptionsBuilder, RemoveImageOptionsBuilder};

use super::super::compose::PullPolicy;
use super::super::docker::Engine;
use super::super::lan::probe::HostNetwork;
use super::super::lan::{Detector, Lan};
use super::super::languages::Languages;
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
    manager.lan_pass(&Languages::default()).await.unwrap();
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
    let port = wonton_tunnel::test_support::unused_port();
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

/// containerd 镜像存储下，核心镜像的记录被清掉（标签先被移走）后，路由探针退回按容器创建时的引用起，
/// 检测照样能跑（#666）。经典存储下旧镜像 ID 一直可用，复现不了这个行为，本地不跑。
/// 前提：Docker 用 containerd 镜像存储。CI 的 Docker 作业设 `WONTON_TEST_REQUIRE_CONTAINERD=1`，不是就失败，不许
/// 静默跳过。在 npchy 上手动跑：
/// `~/wonton-dev/cargo-slot.sh cargo test -p wonton-core a_gone_core_image_record -- --ignored`
#[tokio::test]
#[ignore = "需要 Docker（containerd 镜像存储）；经典存储下旧镜像 ID 一直可用，复现不了 #666"]
async fn a_gone_core_image_record_falls_back_to_the_creation_reference() {
    let engine = Engine::connect().unwrap();
    let mut names = Names::new();
    // 前提：containerd 镜像存储。
    let info = engine.docker.info().await.unwrap();
    let driver = info
        .driver_status
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",");
    if !driver.contains("io.containerd.snapshotter.v1") {
        assert!(
            std::env::var_os("WONTON_TEST_REQUIRE_CONTAINERD").is_none(),
            "WONTON_TEST_REQUIRE_CONTAINERD 设了，但 Docker 不是 containerd 镜像存储（{driver}），复现不了 #666"
        );
        println!("跳过：Docker 不是 containerd 镜像存储（{driver}），复现不了 #666");
        return;
    }
    // 扮演核心镜像的一份私有镜像（busybox + 一个打印 HostNetwork 的假 lan-probe）：ID 独有，
    // 它的记录可以安全地删掉，不碰别人的镜像。脚本经归档接口原样写进容器，不过 shell，引号原样保留。
    let output = serde_json::to_string(&HostNetwork {
        subnets: vec!["10.213.0.0/24".parse().unwrap()],
        ..Default::default()
    })
    .unwrap();
    let script = format!("#!/bin/sh\necho '{output}'\n");
    let seed = names.oneshot_container("probe-image-seed");
    engine
        .create_container(
            &seed,
            ContainerCreateBody {
                image: Some(IMAGE.to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    let mut archive = Vec::new();
    tar::Builder::new(&mut archive)
        .append_data(&mut header, "lan-probe", script.as_bytes())
        .unwrap();
    engine.upload_archive(&seed, "/bin", archive).await.unwrap();
    let repo = names.oneshot_image("probe-image");
    let old_id = commit_image(&engine, &seed, &repo).await;
    // 扮演核心的容器，跑在旧镜像上。
    let core = names.oneshot_container("fake-core");
    engine
        .create_container(
            &core,
            ContainerCreateBody {
                image: Some(format!("{repo}:v1")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(&core).await.unwrap();
    let probe = names.oneshot_container("route-probe");
    let first = detect_host(&engine, &core, &probe).await;
    assert_eq!(first.subnets, ["10.213.0.0/24".parse().unwrap()]);
    // 镜像重建、标签移到新镜像，旧镜像的记录又被清掉：核心容器还在跑，按它的镜像 ID 起不了容器。
    let new_id = commit_image(&engine, &seed, &repo).await;
    assert_ne!(old_id, new_id, "提交出来的两份镜像 ID 不该相同");
    engine.remove_container(&seed).await.unwrap();
    engine
        .docker
        .remove_image(
            &old_id,
            Some(RemoveImageOptionsBuilder::default().force(true).build()),
            None,
        )
        .await
        .unwrap();
    // 退回按容器创建时的引用（现在是重建后的镜像）起探测容器，检测照常。
    let second = detect_host(&engine, &core, &probe).await;
    assert_eq!(second.subnets, ["10.213.0.0/24".parse().unwrap()]);
    // 清理：私有镜像的标签和记录。
    for image in [format!("{repo}:v1"), new_id, old_id] {
        let _ = engine
            .docker
            .remove_image(
                &image,
                Some(RemoveImageOptionsBuilder::default().force(true).build()),
                None,
            )
            .await;
    }
}

/// 给测试提交一份镜像：内容随 seed 容器（busybox 加假 lan-probe），标签指向它，入口就是假 lan-probe。
/// 同一个标签再提交一次就移到新镜像。
async fn commit_image(engine: &Engine, seed: &str, repo: &str) -> String {
    engine
        .docker
        .commit_container(
            CommitContainerOptionsBuilder::default()
                .container(seed)
                .repo(repo)
                .tag("v1")
                .pause(false)
                .build(),
            ContainerConfig {
                entrypoint: Some(vec![String::from("/bin/lan-probe")]),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

/// 检测一次宿主的网卡（走 [`Detector::Container`]，即路由探针）。
async fn detect_host(engine: &Engine, core: &str, probe: &str) -> HostNetwork {
    Detector::Container {
        core: core.to_owned(),
    }
    .detect(engine, probe, HashMap::new())
    .await
    .unwrap()
    .unwrap()
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
