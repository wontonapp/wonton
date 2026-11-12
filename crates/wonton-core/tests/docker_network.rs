//! 核心接入、退出插件网络时，经发布端口连进核心的连接不断（#90）。要真的 Docker Engine：默认网关和发布端口
//! 落在哪个端点，是 Engine 的行为。
//!
//! 用一个回显的容器扮演核心：它在“核心的网络”上发布端口，再像装、卸能出网的插件时那样接入、退出一个名字排在
//! 前面的普通桥接网络。

use std::collections::HashMap;
use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig, NetworkInspect, PortBinding};
use futures_util::future::join_all;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::core_address::{Endpoint, last_usable};
use wonton_core::plugin::docker::{Engine, NetworkReach};

/// npchy 和 CI 的 runner 连不上 Docker Hub，从 ECR Public 拉。
const IMAGE: &str = "public.ecr.aws/docker/library/busybox:1.37";
const PORT: &str = "7000/tcp";
/// 这些测试都建、删网络。并行时一个测试的网段会被另一个拿走（实测），它们要测的是别的事，所以一次跑一个。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Names {
    core_network: String,
    plugin_network: String,
    container: String,
}

impl Names {
    /// 插件网络 `…-egress` 排在核心网络 `…_core` 前面（`-` 小于 `_`），和真实的名字一样。
    fn new() -> Self {
        let prefix = format!("wonton-gwtest{}", std::process::id());
        Self {
            core_network: format!("{prefix}_core"),
            plugin_network: format!("{prefix}-egress"),
            container: format!("{prefix}--core"),
        }
    }

    async fn remove(&self, engine: &Engine) {
        engine.remove_container(&self.container).await.unwrap();
        engine.remove_network(&self.plugin_network).await.unwrap();
        engine.remove_network(&self.core_network).await.unwrap();
    }
}

/// 发一行，等它原样回来。
async fn echo(stream: &mut TcpStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes()).await?;
    let mut back = vec![0; line.len()];
    timeout(Duration::from_secs(5), stream.read_exact(&mut back))
        .await
        .map_err(|_| std::io::Error::other("5 秒内没有回显"))??;
    assert_eq!(back, line.as_bytes());
    Ok(())
}

/// 连上发布的端口，直到容器里的回显服务应答。docker-proxy 先接受连接再连容器，容器还没监听时连接随即关闭。
async fn connect(port: u16) -> TcpStream {
    for _ in 0..100 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await
            && echo(&mut stream, "hello\n").await.is_ok()
        {
            return stream;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("连不上发布的端口 {port}");
}

async fn published_port(engine: &Engine, container: &str) -> u16 {
    let inspect = engine.inspect_container(container).await.unwrap().unwrap();
    let ports = inspect.network_settings.unwrap().ports.unwrap();
    let binding = &ports[PORT].as_ref().unwrap()[0];
    binding.host_port.as_ref().unwrap().parse().unwrap()
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn leaving_an_egress_plugin_network_keeps_connections_through_published_ports() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let names = Names::new();
    names.remove(&engine).await;
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    let labels = HashMap::new();
    engine
        .create_network(&names.core_network, &labels, NetworkReach::Egress)
        .await
        .unwrap();
    engine
        .create_network(&names.plugin_network, &labels, NetworkReach::Egress)
        .await
        .unwrap();
    let binding = PortBinding {
        host_ip: Some("127.0.0.1".to_owned()),
        host_port: Some(String::new()),
    };
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        // 回显第一个连上来的连接。
        cmd: Some(
            ["nc", "-l", "-p", "7000", "-e", "cat"]
                .map(str::to_owned)
                .into(),
        ),
        exposed_ports: Some(vec![PORT.to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(names.core_network.clone()),
            port_bindings: Some([(PORT.to_owned(), Some(vec![binding]))].into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine
        .create_container(&names.container, body)
        .await
        .unwrap();
    engine.start_container(&names.container).await.unwrap();

    // 装上能出网的插件：核心接入它的网络。
    engine
        .connect_network(&names.plugin_network, &names.container, Vec::new())
        .await
        .unwrap();
    let port = published_port(&engine, &names.container).await;
    let mut stream = connect(port).await;
    // 卸载它：核心退出它的网络。
    engine
        .disconnect_network(&names.plugin_network, &names.container)
        .await
        .unwrap();
    let after = echo(&mut stream, "still here\n").await;

    names.remove(&engine).await;
    after.expect("核心退出插件网络后，经发布端口的连接断了");
}

fn docker() -> bollard::Docker {
    bollard::Docker::connect_with_defaults().unwrap()
}

async fn inspect(name: &str) -> NetworkInspect {
    docker().inspect_network(name, None).await.unwrap()
}

/// 一直运行的小容器，接在 `network` 上。
async fn sleeper(engine: &Engine, name: &str, network: &str) {
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec!["sleep".to_owned(), "3600".to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(network.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.remove_container(name).await.unwrap();
    engine.create_container(name, body).await.unwrap();
    engine.start_container(name).await.unwrap();
}

/// 建网络的名字前缀，和别的测试、别的进程分开。
fn prefix(test: &str) -> String {
    format!("wonton-pintest{}-{test}", std::process::id())
}

/// 建网络要先让 Docker 选网段、删掉、再带着留出的地址重建，并发时几个调用会选中同一个网段（#327）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn plugin_networks_created_at_the_same_time_all_keep_the_core_address_free() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let labels = HashMap::new();
    let names: Vec<String> = (0..8)
        .map(|i| format!("{}-{i}", prefix("concurrent")))
        .collect();

    let created = join_all(
        names
            .iter()
            .map(|name| engine.create_network(name, &labels, NetworkReach::Isolated)),
    )
    .await;
    let addresses = join_all(names.iter().map(|name| engine.core_address(name))).await;
    for name in &names {
        engine.remove_network(name).await.unwrap();
    }

    for result in created {
        result.unwrap();
    }
    let mut addresses: Vec<_> = addresses.into_iter().map(Result::unwrap).collect();
    addresses.sort_unstable();
    addresses.dedup();
    assert_eq!(addresses.len(), names.len());
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn containers_get_the_lower_half_and_the_core_keeps_its_address_across_recreation() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    for reach in [
        NetworkReach::Egress,
        NetworkReach::Internal,
        NetworkReach::Isolated,
    ] {
        let network = prefix(&format!("{reach:?}").to_lowercase());
        let (app, core) = (format!("{network}--app"), format!("{network}--core"));
        engine.remove_container(&app).await.unwrap();
        engine.remove_container(&core).await.unwrap();
        engine.remove_network(&network).await.unwrap();
        engine
            .create_network(&network, &HashMap::new(), reach)
            .await
            .unwrap();
        sleeper(&engine, &app, &network).await;
        let address = engine.core_address(&network).await.unwrap();
        let attach = Endpoint {
            address: Some(address.to_string()),
            ..Default::default()
        };
        // 核心容器删掉重建后还是这个地址。每个容器停下要等 10 秒，只在一种网络上重建。
        let cycles = if reach == NetworkReach::Egress { 2 } else { 1 };
        for _ in 0..cycles {
            let other = format!("{core}-net");
            engine.remove_container(&core).await.unwrap();
            engine.remove_network(&other).await.unwrap();
            engine
                .create_network(&other, &HashMap::new(), NetworkReach::Egress)
                .await
                .unwrap();
            sleeper(&engine, &core, &other).await;
            engine
                .connect_network_at(&network, &core, attach.clone())
                .await
                .unwrap();
            let got = engine.endpoint_address(&core, &network).await.unwrap();
            assert_eq!(got.as_deref(), Some(address.to_string().as_str()));
            engine.remove_container(&core).await.unwrap();
            engine.remove_network(&other).await.unwrap();
        }
        let got = engine
            .endpoint_address(&app, &network)
            .await
            .unwrap()
            .unwrap();
        let ip: std::net::Ipv4Addr = got.parse().unwrap();
        let subnet = inspect(&network).await.ipam.unwrap().config.unwrap()[0]
            .subnet
            .clone()
            .unwrap();
        let subnet: ipnet::Ipv4Net = subnet.parse().unwrap();
        let lower: ipnet::Ipv4Net = subnet
            .subnets(subnet.prefix_len() + 1)
            .unwrap()
            .next()
            .unwrap();
        assert!(lower.contains(&ip), "{ip} 不在前一半 {lower}");
        assert_eq!(last_usable(subnet), Some(address));
        engine.remove_container(&app).await.unwrap();
        engine.remove_network(&network).await.unwrap();
    }
}

/// 主机上已经有网络占着 Docker 下一个要分配的网段，别的程序也在建网络：Docker 跳过它，核心照常建好插件网络，
/// 不用自己避开。
///
/// 占着的网络让 Docker 分配后一直留着。读出网段、删掉、再固定这个网段重建会留一个空当，同一台 Docker 上别的
/// 任务这时建网络就拿走这个网段（CI 实测，Docker 29.7.2 从低往高找第一个空网段）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_network_already_on_the_next_subnet_is_skipped() {
    use bollard::models::NetworkCreateRequest;
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    let (taken, other, plugin) = (prefix("taken"), prefix("other"), prefix("plugin"));
    for name in [&plugin, &other, &taken] {
        engine.remove_network(name).await.unwrap();
    }
    let create = |name: &str| NetworkCreateRequest {
        name: name.to_owned(),
        internal: Some(true),
        ..Default::default()
    };
    let subnet_of = async |name: &str| {
        inspect(name).await.ipam.unwrap().config.unwrap()[0]
            .subnet
            .clone()
            .unwrap()
    };
    docker().create_network(create(&taken)).await.unwrap();
    let next = subnet_of(&taken).await;
    docker().create_network(create(&other)).await.unwrap();

    let created = engine
        .create_network(&plugin, &HashMap::new(), NetworkReach::Internal)
        .await;
    let got = match &created {
        Ok(()) => Some(subnet_of(&plugin).await),
        Err(_) => None,
    };
    let address = engine.core_address(&plugin).await;
    for name in [&plugin, &other, &taken] {
        engine.remove_network(name).await.unwrap();
    }

    created.unwrap();
    assert_ne!(got.as_deref(), Some(next.as_str()));
    address.unwrap();
}

async fn cleanup(engine: &Engine, network: &str, containers: &[&str]) {
    for container in containers {
        engine.remove_container(container).await.unwrap();
    }
    engine.remove_network(network).await.unwrap();
}

/// 在容器里跑一条命令，返回退出码。
async fn run_in(container: &str, cmd: &[&str]) -> i64 {
    use bollard::exec::{CreateExecOptions, StartExecResults};
    use futures_util::StreamExt;
    let docker = docker();
    let exec = docker
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| (*s).to_owned()).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    if let StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await.unwrap()
    {
        while output.next().await.is_some() {}
    }
    docker
        .inspect_exec(&exec.id)
        .await
        .unwrap()
        .exit_code
        .unwrap()
}

/// 两份 compose 都关了核心的转发（下面这行 sysctl），关了之后核心还能以网关优先级 -1 接入别的网络、连到上面的容器。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_core_with_forwarding_off_still_reaches_plugin_networks() {
    let _serial = SERIAL.lock().await;
    const FORWARD: &str = "net.ipv4.ip_forward=0";
    for compose in [
        include_str!("../../../deploy/install/compose.yaml"),
        include_str!("../../../deploy/dev/core/compose.yaml"),
    ] {
        assert!(
            compose.contains(&format!("- {FORWARD}")),
            "compose 里要有 {FORWARD}"
        );
    }
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    let plugin = prefix("forward");
    let own = format!("{plugin}_core");
    let (core, app) = (format!("{plugin}--core"), format!("{plugin}--app"));
    cleanup(&engine, &plugin, &[&core, &app]).await;
    engine.remove_network(&own).await.unwrap();
    engine
        .create_network(&own, &HashMap::new(), NetworkReach::Egress)
        .await
        .unwrap();
    engine
        .create_network(&plugin, &HashMap::new(), NetworkReach::Egress)
        .await
        .unwrap();
    sleeper(&engine, &app, &plugin).await;
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec!["sleep".to_owned(), "3600".to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(own.clone()),
            sysctls: Some(HashMap::from([(
                "net.ipv4.ip_forward".to_owned(),
                "0".to_owned(),
            )])),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&core, body).await.unwrap();
    engine.start_container(&core).await.unwrap();
    let address = engine.core_address(&plugin).await.unwrap();
    engine
        .connect_network_at(
            &plugin,
            &core,
            Endpoint {
                address: Some(address.to_string()),
                aliases: Vec::new(),
                gw_priority: Some(-1),
            },
        )
        .await
        .unwrap();
    let app_ip = engine
        .endpoint_address(&app, &plugin)
        .await
        .unwrap()
        .unwrap();
    let forwarded = run_in(
        &core,
        &["grep", "-q", "^0$", "/proc/sys/net/ipv4/ip_forward"],
    )
    .await;
    let reached = run_in(&core, &["ping", "-c", "1", "-W", "2", &app_ip]).await;
    cleanup(&engine, &plugin, &[&core, &app]).await;
    engine.remove_network(&own).await.unwrap();
    assert_eq!(forwarded, 0, "核心的 ip_forward 没关");
    assert_eq!(reached, 0, "关了转发后连不到插件网络上的容器");
}
