//! 核心接入、退出插件网络时，经发布端口连进核心的连接不断（#90）。要真的 Docker Engine：默认网关和发布端口
//! 落在哪个端点，是 Engine 的行为。
//!
//! 用一个回显的容器扮演核心：它在“核心的网络”上发布端口，再像装、卸能出网的插件时那样接入、退出一个名字排在
//! 前面的普通桥接网络。

use std::collections::HashMap;
use std::time::Duration;

use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, HostConfig, Ipam, IpamConfig, NetworkCreateRequest, NetworkInspect,
    PortBinding,
};
use futures_util::{FutureExt, future::join_all};
use ipnet::Ipv4Net;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::core_address::{Endpoint, last_usable};
use wonton_core::plugin::docker::{Engine, NetworkReach};
use wonton_core::plugin::lan::detect::Detector;
use wonton_core::plugin::network_pool::NetworkPool;

// 共用的真实 Docker 测试辅助：每次运行唯一的实例名和资源回收。
#[path = "../src/test_docker.rs"]
mod test_docker;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn image() -> String {
    wonton_test_images::image("public.ecr.aws/docker/library/busybox:1.37")
}
const PORT: &str = "7000/tcp";

struct Names {
    core_network: String,
    plugin_network: String,
    container: String,
}

impl Names {
    /// 插件网络 `…-egress` 排在核心网络 `…_core` 前面（`-` 小于 `_`），和真实的名字一样。
    fn new(names: &mut test_docker::Names) -> Self {
        Self {
            core_network: names.oneshot_network("gateway_core"),
            plugin_network: names.oneshot_network("gateway-egress"),
            container: names.oneshot_container("gateway-core"),
        }
    }
}

/// 候选次序对进程和测试固定；占位网络用最后一个 /26 把整个 /16 真正占住。
fn candidate_octets(start: u8) -> impl Iterator<Item = u8> {
    (0u16..256)
        .map(move |offset| start.wrapping_add(offset as u8))
        .filter(|&octet| octet != 213)
}

#[derive(Debug)]
struct PrivatePoolClaim {
    pool: NetworkPool,
    network: String,
}

fn docker_subnet_taken(error: &BollardError) -> bool {
    let message = match error {
        BollardError::DockerResponseServerError { message, .. }
        | BollardError::DockerStreamError { error: message } => message,
        _ => return false,
    };
    message.contains("overlap") || message.contains("already in use")
}

fn placeholder_request(name: String, subnet: Ipv4Net) -> NetworkCreateRequest {
    NetworkCreateRequest {
        name,
        internal: Some(true),
        ipam: Some(Ipam {
            config: Some(vec![IpamConfig {
                subnet: Some(subnet.to_string()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn occupied_subnets() -> Vec<Ipv4Net> {
    let mut occupied: Vec<Ipv4Net> = docker()
        .list_networks(None)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|network| network.ipam)
        .flat_map(|ipam| ipam.config.unwrap_or_default())
        .filter_map(|config| config.subnet?.parse().ok())
        .collect();
    let routes = std::fs::read_to_string("/proc/net/route").unwrap();
    for line in routes.lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        let destination = u32::from_str_radix(fields[1], 16).unwrap();
        let mask = u32::from_str_radix(fields[7], 16).unwrap();
        if mask != 0 {
            let address = std::net::Ipv4Addr::from(destination.to_le_bytes());
            occupied.push(
                Ipv4Net::new(address, mask.count_ones() as u8)
                    .unwrap()
                    .trunc(),
            );
        }
    }
    occupied
}

async fn claim_private_pool(
    names: &mut test_docker::Names,
    candidates: impl IntoIterator<Item = u8>,
) -> PrivatePoolClaim {
    let occupied = occupied_subnets().await;
    claim_private_pool_from(names, candidates, &occupied).await
}

async fn claim_private_pool_from(
    names: &mut test_docker::Names,
    candidates: impl IntoIterator<Item = u8>,
    occupied: &[Ipv4Net],
) -> PrivatePoolClaim {
    let mut tried = Vec::new();
    for octet in candidates.into_iter().take(16) {
        let pool: NetworkPool = format!("10.{octet}.0.0/16").parse().unwrap();
        let subnet = pool.subnet();
        if occupied.iter().any(|used| overlaps(subnet, *used)) {
            tried.push(subnet);
            continue;
        }
        let network = names.oneshot_network(&format!("private-pool-{octet}"));
        let claim = subnet.subnets(26).unwrap().last().unwrap();
        match docker()
            .create_network(placeholder_request(network.clone(), claim))
            .await
        {
            Ok(_) => return PrivatePoolClaim { pool, network },
            Err(error) if docker_subnet_taken(&error) => tried.push(subnet),
            Err(error) => panic!("创建测试专用 /16 {subnet} 的占位网络 {network} 失败：{error}"),
        }
    }
    let count = tried.len();
    let listed = tried
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("、");
    panic!("{count} 个候选 /16 都被占了：{listed}");
}

async fn private_pool(test: &str, names: &mut test_docker::Names) -> NetworkPool {
    use std::sync::LazyLock;
    static SEED: LazyLock<u32> = LazyLock::new(|| getrandom::u32().unwrap());
    let seed = *SEED ^ std::process::id();
    let start = test
        .bytes()
        .fold(seed, |n, b| n.wrapping_mul(31) ^ u32::from(b)) as u8;
    claim_private_pool(names, candidate_octets(start))
        .await
        .pool
}

fn free_candidate_octets(start: u8, occupied: &[Ipv4Net], limit: usize) -> Vec<u8> {
    candidate_octets(start)
        .filter(|&octet| {
            let subnet: Ipv4Net = format!("10.{octet}.0.0/16").parse().unwrap();
            occupied.iter().all(|used| !overlaps(subnet, *used))
        })
        .take(limit)
        .collect()
}

fn candidate_start(text: &str) -> u8 {
    text.bytes()
        .fold(0x9e37_79b9u32, |n, b| n.wrapping_mul(31) ^ u32::from(b)) as u8
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn private_pool_claim_retries_after_the_first_candidate_is_taken() {
    let mut names = test_docker::Names::new();
    let occupied = occupied_subnets().await;
    let candidates = free_candidate_octets(candidate_start(names.name()), &occupied, 16);
    assert!(candidates.len() >= 2, "需要至少两个未占用的测试候选段");
    let first: Ipv4Net = format!("10.{}.0.0/16", candidates[0]).parse().unwrap();
    let first_claim = first.subnets(26).unwrap().last().unwrap();
    let taken = names.oneshot_network("claim-taken");
    match docker()
        .create_network(placeholder_request(taken, first_claim))
        .await
    {
        Ok(_) => {}
        Err(error) if docker_subnet_taken(&error) => {}
        Err(error) => panic!("预占第一个测试候选 {first} 失败：{error}"),
    }

    let claim = claim_private_pool_from(&mut names, candidates.clone(), &occupied).await;
    assert_ne!(claim.pool.subnet(), first);
    assert!(
        candidates
            .iter()
            .any(|&octet| claim.pool.subnet() == format!("10.{octet}.0.0/16").parse().unwrap())
    );
    let placeholder = inspect(&claim.network).await;
    let expected_claim = claim.pool.subnet().subnets(26).unwrap().last().unwrap();
    assert_eq!(subnet_of(&claim.network).await, expected_claim);
    assert_eq!(placeholder.internal, Some(true));
    assert!(placeholder.containers.unwrap_or_default().is_empty());
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn private_pool_claim_reports_all_exhausted_candidates() {
    let mut names = test_docker::Names::new();
    let occupied = occupied_subnets().await;
    let candidates = free_candidate_octets(candidate_start(names.name()), &occupied, 2);
    assert_eq!(candidates.len(), 2, "需要两个未占用的测试候选段");
    for (index, &octet) in candidates.iter().enumerate() {
        let pool: Ipv4Net = format!("10.{octet}.0.0/16").parse().unwrap();
        let subnet = pool.subnets(26).unwrap().last().unwrap();
        let name = names.oneshot_network(&format!("claim-exhausted-{index}"));
        match docker()
            .create_network(placeholder_request(name, subnet))
            .await
        {
            Ok(_) => {}
            Err(error) if docker_subnet_taken(&error) => {}
            Err(error) => panic!("预占测试候选 {pool} 失败：{error}"),
        }
    }

    let panic = std::panic::AssertUnwindSafe(claim_private_pool_from(
        &mut names,
        candidates.clone(),
        &occupied,
    ))
    .catch_unwind()
    .await
    .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(message.contains("2 个候选 /16 都被占了"), "{message}");
    for octet in candidates {
        assert!(message.contains(&format!("10.{octet}.0.0/16")), "{message}");
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
    let mut names = test_docker::Names::new();
    let pool = private_pool("gateway", &mut names).await;
    let names = Names::new(&mut names);
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    engine.check_server().await.unwrap();
    engine
        .ensure_image(&image(), PullPolicy::Missing, None)
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
        image: Some(image()),
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

    // 资源已经卸完，断言连接还活着；剩下的回收交给 Names 守卫。
    engine.remove_container(&names.container).await.unwrap();
    engine.remove_network(&names.plugin_network).await.unwrap();
    engine.remove_network(&names.core_network).await.unwrap();
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
        image: Some(image()),
        cmd: Some(vec!["sleep".to_owned(), "3600".to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(network.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(name, body).await.unwrap();
    engine.start_container(name).await.unwrap();
}

fn overlaps(left: Ipv4Net, right: Ipv4Net) -> bool {
    left.contains(&right.network()) || right.contains(&left.network())
}

async fn subnet_of(name: &str) -> Ipv4Net {
    inspect(name).await.ipam.unwrap().config.unwrap()[0]
        .subnet
        .as_ref()
        .unwrap()
        .parse()
        .unwrap()
}

async fn assert_core_attach(engine: &Engine, network: &str, core: &str, subnet: Ipv4Net) {
    let address = engine.core_address(network).await.unwrap();
    assert_eq!(last_usable(subnet), Some(address));
    engine
        .connect_network_at(
            network,
            core,
            Endpoint {
                address: Some(address.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let got = engine.endpoint_address(core, network).await.unwrap();
    assert_eq!(got.as_deref(), Some(address.to_string().as_str()));
}

/// 并发分配显式小网段，自动地址不能占用核心保留的最后一个地址。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn plugin_networks_created_at_the_same_time_all_keep_the_core_address_free() {
    let mut names = test_docker::Names::new();
    let pool = private_pool("concurrent", &mut names).await;
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    engine.check_server().await.unwrap();
    engine
        .ensure_image(&image(), PullPolicy::Missing, None)
        .await
        .unwrap();
    let labels = HashMap::new();
    let networks: Vec<String> = (0..20)
        .map(|i| names.oneshot_network(&format!("concurrent-{i}")))
        .collect();
    let core = names.oneshot_container("concurrent-core");
    let own = names.oneshot_network("concurrent-own");
    let created = join_all(
        networks
            .iter()
            .map(|name| engine.create_network(name, &labels, NetworkReach::Isolated)),
    )
    .await;
    for result in created {
        result.unwrap();
    }
    // none 模式的容器不能再接入别的网络（Docker 29 实测），核心容器先放在自己的一次性网络上。
    engine
        .create_network(&own, &labels, NetworkReach::Egress)
        .await
        .unwrap();
    sleeper(&engine, &core, &own).await;
    let mut subnets = Vec::new();
    for name in &networks {
        let subnet = subnet_of(name).await;
        assert_eq!(subnet.prefix_len(), 26);
        assert!(pool.subnet().contains(&subnet));
        for other in &subnets {
            assert!(!overlaps(subnet, *other));
        }
        subnets.push(subnet);
        assert_core_attach(&engine, name, &core, subnet).await;
        engine.disconnect_network(name, &core).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn containers_get_the_lower_half_and_the_core_keeps_its_address_across_recreation() {
    let mut names = test_docker::Names::new();
    let pool = private_pool("recreation", &mut names).await;
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    engine
        .ensure_image(&image(), PullPolicy::Missing, None)
        .await
        .unwrap();
    for reach in [
        NetworkReach::Egress,
        NetworkReach::Internal,
        NetworkReach::Isolated,
    ] {
        let kind = format!("{reach:?}").to_lowercase();
        let network = names.oneshot_network(&format!("recreation-{kind}"));
        let app = names.oneshot_container(&format!("recreation-app-{kind}"));
        let core = names.oneshot_container(&format!("recreation-core-{kind}"));
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
            let other = names.oneshot_network(&format!("recreation-core-net-{kind}"));
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
    }
}

/// 第一个 /26 从创建到断言完成一直被占用，核心必须跳过它，不能靠测试释放网段。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_network_already_on_the_next_subnet_is_skipped() {
    let mut names = test_docker::Names::new();
    let pool = private_pool("occupied", &mut names).await;
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    engine
        .ensure_image(&image(), PullPolicy::Missing, None)
        .await
        .unwrap();
    let taken = names.oneshot_network("occupied-taken");
    let plugin = names.oneshot_network("occupied-plugin");
    let core = names.oneshot_container("occupied-core");
    let own = names.oneshot_network("occupied-own");
    let first = pool.subnet().subnets(26).unwrap().next().unwrap();
    docker()
        .create_network(NetworkCreateRequest {
            name: taken.clone(),
            internal: Some(true),
            ipam: Some(Ipam {
                config: Some(vec![IpamConfig {
                    subnet: Some(first.to_string()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    engine
        .create_network(&plugin, &HashMap::new(), NetworkReach::Internal)
        .await
        .unwrap();
    let chosen = subnet_of(&plugin).await;
    assert_eq!(chosen.prefix_len(), 26);
    assert!(pool.subnet().contains(&chosen));
    assert!(!overlaps(chosen, first));
    assert_eq!(subnet_of(&taken).await, first);
    // none 模式的容器不能再接入别的网络（Docker 29 实测），核心容器先放在自己的一次性网络上。
    engine
        .create_network(&own, &HashMap::new(), NetworkReach::Egress)
        .await
        .unwrap();
    sleeper(&engine, &core, &own).await;
    assert_core_attach(&engine, &plugin, &core, chosen).await;
}

/// 40 个插件网络全部落在配置的地址段里、互不重叠。Docker 默认地址池大约建 30 个网络就用完（#120），
/// 旧做法（让 Docker 分网段）装到第 30 个左右就失败；核心自挑 /26 后一个 /16 够 1024 个。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn forty_plugin_networks_all_fit_the_configured_pool() {
    let mut names = test_docker::Names::new();
    let pool = private_pool("forty", &mut names).await;
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    let networks: Vec<String> = (0..40)
        .map(|i| names.oneshot_network(&format!("forty-{i}")))
        .collect();
    for name in &networks {
        engine
            .create_network(name, &HashMap::new(), NetworkReach::Internal)
            .await
            .unwrap();
    }
    let mut subnets = Vec::new();
    for name in &networks {
        let subnet = subnet_of(name).await;
        assert_eq!(subnet.prefix_len(), 26, "{name}");
        assert!(
            pool.subnet().contains(&subnet),
            "{name}: {subnet} 不在地址段里"
        );
        subnets.push(subnet);
    }
    for (i, left) in subnets.iter().enumerate() {
        for right in &subnets[i + 1..] {
            assert!(!overlaps(*left, *right), "{left} 和 {right} 重叠");
        }
    }
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

/// 运行时关闭核心的转发后，核心仍能以网关优先级 -1 接入插件网络并连到容器。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_core_with_forwarding_off_still_reaches_plugin_networks() {
    let mut names = test_docker::Names::new();
    let pool = private_pool("forwarding", &mut names).await;
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(pool, Detector::Here);
    engine
        .ensure_image(&image(), PullPolicy::Missing, None)
        .await
        .unwrap();
    let plugin = names.oneshot_network("forward");
    let own = names.oneshot_network("forward-core");
    let core = names.oneshot_container("forward-core");
    let app = names.oneshot_container("forward-app");
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
        image: Some(image()),
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
    assert_eq!(forwarded, 0, "核心的 ip_forward 没关");
    assert_eq!(reached, 0, "关了转发后连不到插件网络上的容器");
}
