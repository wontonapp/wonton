//! 入口拓扑下隧道端口的行为（ADR-0021）：绑在核心 core 网络地址上的套接字，收得到经发布端口进来的
//! 包（局域网和公网的手机走这条），收不到入口容器所在的 clients 网络上发到隧道端口的包，core 网络里
//! 照常连得上。要真的 Docker Engine：发布的端口落在哪个端点、isolated 的网络宿主一侧有没有地址，
//! 都是 Engine 的行为。
//!
//! 容器形态照 compose 的：渲染出的核心服务 networks 映射里 clients 排在 core 前面（serde_json 的
//! map 按字母序渲染），compose 把排在前面的网络设成容器的主端点（`HostConfig.NetworkMode`）。
//! 三个方向的包各自从对应网络里的探针容器发出，走 `bind-probe`（wonton-tunnel 的示例，绑定和
//! `tunnel.rs` 同一条规则、收到包原样回给发送方）；它的回包出现在 exec 的标准输出里。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, EndpointSettings, HostConfig, NetworkingConfig, PortBinding,
};
use wonton_core::entry_settings::EntrySettings;
use wonton_core::plugin::docker::{Engine, NetworkReach};
use wonton_core::plugin::docker_exec::ExecOutput;
use wonton_core::plugin::lan::detect::Detector;
use wonton_core::plugin::network_pool::NetworkPool;

#[path = "../src/test_docker.rs"]
mod test_docker;

/// 测试容器是 Debian：bind-probe 在宿主编译，glibc 向后兼容。
fn image() -> String {
    wonton_test_images::image("public.ecr.aws/docker/library/debian:trixie-slim")
}

const TUNNEL_PORT: &str = "18482/udp";
const PORT: u16 = 18482;

/// 渲染带入口的安装 compose，读出绑定的别名、core 网络的别名和入口别名：被测的配置就是这份渲染。
fn rendered_bind() -> (String, Vec<String>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
    std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
    let settings = EntrySettings {
        domain: "home.example.test".to_owned(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: None,
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_propagation_disable_ans: false,
        acme_email: None,
        acme_accept_terms: false,
        acme_network: "bridge".to_owned(),
        dns_provider: None,
        dns_env_file: None,
        external_certificate: false,
    };
    let compose: serde_json::Value = serde_json::from_str(
        &wonton_core::entry_compose::render(
            wonton_core::install::COMPOSE,
            Some(&settings),
            dir.path(),
        )
        .unwrap(),
    )
    .unwrap();
    let core = &compose["services"]["core"];
    let bind = core["environment"]["WONTON_TUNNEL_BIND"]
        .as_str()
        .unwrap()
        .to_owned();
    let aliases = |network: &str| {
        core["networks"][network]["aliases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|alias| alias.as_str().unwrap().to_owned())
            .collect()
    };
    (bind, aliases("core"), aliases("clients"))
}

/// 跑完 `sleep infinity` 的探针容器，挂在一张网络上。
async fn start_probe(engine: &Engine, name: &str, network: &str, binary: &std::path::Path) {
    engine
        .create_container(
            name,
            ContainerCreateBody {
                image: Some(image()),
                cmd: Some(["sleep", "infinity"].map(str::to_owned).into()),
                host_config: Some(HostConfig {
                    network_mode: Some(network.to_owned()),
                    binds: Some(vec![format!(
                        "{}:/usr/local/bin/bind-probe:ro",
                        binary.display()
                    )]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(name).await.unwrap();
}

/// 从探针容器发一个包，等回包；回包内容就是 `bind-probe send` 打到标准输出的那行。
async fn send(engine: &Engine, probe: &str, target: &SocketAddr, payload: &str) -> ExecOutput {
    engine
        .exec_output(
            probe,
            &[
                "/usr/local/bin/bind-probe".to_owned(),
                "send".to_owned(),
                target.to_string(),
                payload.to_owned(),
                "--timeout-secs".to_owned(),
                "4".to_owned(),
            ],
            None,
            Duration::from_secs(20),
        )
        .await
        .unwrap()
}

/// 编出来的 bind-probe 示例二进制（挂进容器）。
fn bind_probe_binary() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("examples/bind-probe"))
        .filter(|path| path.exists())
        .expect("bind-probe 示例没编出来：cargo test 会编 examples")
}

/// 起隧道套接字的宿主：compose 的形态——主端点是 compose 排在前的 clients 网络，两张网创建时
/// 都接上，别名按渲染的。套接字按渲染出的 WONTON_TUNNEL_BIND 绑定（bind-probe 的 listen 用
/// 同一条解析规则）。返回容器名、core/clients 两张网上的地址和发布的宿主端口。
async fn start_listener(
    engine: &Engine,
    name: &str,
    bind: &str,
    core_net: &str,
    clients_net: &str,
    core_aliases: Vec<String>,
    clients_aliases: Vec<String>,
) -> (SocketAddr, SocketAddr, u16) {
    let binary = bind_probe_binary();
    let mut endpoints = HashMap::new();
    endpoints.insert(
        core_net.to_owned(),
        EndpointSettings {
            aliases: Some(core_aliases),
            ..Default::default()
        },
    );
    endpoints.insert(
        clients_net.to_owned(),
        EndpointSettings {
            aliases: Some(clients_aliases),
            ..Default::default()
        },
    );
    engine
        .create_container(
            name,
            ContainerCreateBody {
                image: Some(image()),
                cmd: Some(vec![
                    "/usr/local/bin/bind-probe".to_owned(),
                    "listen".to_owned(),
                    PORT.to_string(),
                ]),
                env: Some(vec![format!("WONTON_TUNNEL_BIND={bind}")]),
                exposed_ports: Some(vec![TUNNEL_PORT.to_owned()]),
                host_config: Some(HostConfig {
                    network_mode: Some(clients_net.to_owned()),
                    port_bindings: Some(
                        [(
                            TUNNEL_PORT.to_owned(),
                            Some(vec![PortBinding {
                                host_ip: None,
                                host_port: Some(String::new()),
                            }]),
                        )]
                        .into(),
                    ),
                    binds: Some(vec![format!(
                        "{}:/usr/local/bin/bind-probe:ro",
                        binary.display()
                    )]),
                    ..Default::default()
                }),
                networking_config: Some(NetworkingConfig {
                    endpoints_config: Some(endpoints),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(name).await.unwrap();

    let info = engine.inspect_container(name).await.unwrap().unwrap();
    let networks = info.network_settings.clone().unwrap().networks.unwrap();
    let address = |network: &str| -> SocketAddr {
        format!(
            "{}:{PORT}",
            networks[network].ip_address.as_deref().unwrap_or_default()
        )
        .parse()
        .unwrap()
    };
    let core_ip = address(core_net);
    let clients_ip = address(clients_net);
    let published: u16 = info
        .network_settings
        .unwrap()
        .ports
        .unwrap()
        .get(TUNNEL_PORT)
        .unwrap()
        .as_ref()
        .unwrap()
        .first()
        .unwrap()
        .host_port
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    (core_ip, clients_ip, published)
}

#[ignore = "需要 Docker，起一次性的网络和容器"]
#[tokio::test]
async fn the_tunnel_port_answers_the_published_port_and_not_the_clients_network() {
    let (bind, core_aliases, clients_aliases) = rendered_bind();
    let mut names = test_docker::Names::new();
    let engine = Engine::connect()
        .unwrap()
        .with_network_pool(NetworkPool::default(), Detector::Here);
    engine.check_server().await.unwrap();
    test_docker::ensure_image(&engine, &image()).await;

    let core_net = names.oneshot_network("tunnel_core");
    let clients_net = names.oneshot_network("tunnel_clients");
    let outside_net = names.oneshot_network("tunnel_outside");
    engine
        .create_network(&core_net, &HashMap::new(), NetworkReach::Egress)
        .await
        .unwrap();
    engine
        .create_network(&clients_net, &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap();
    engine
        .create_network(&outside_net, &HashMap::new(), NetworkReach::Egress)
        .await
        .unwrap();

    let listener = names.oneshot_container("tunnel_core");
    let (core_ip, clients_ip, published) = start_listener(
        &engine,
        &listener,
        &bind,
        &core_net,
        &clients_net,
        core_aliases,
        clients_aliases,
    )
    .await;
    // 发布端口落点的宿主地址：外部网络的网关（局域网里的手机从这一侧进来）。
    let gateway = engine.network_gateway(&outside_net).await.unwrap();
    let binary = bind_probe_binary();

    // 局域网/公网的手机：包经发布的端口进来，落到绑在 core 网络地址上的套接字。
    let outside_probe = names.oneshot_container("tunnel_outside_probe");
    start_probe(&engine, &outside_probe, &outside_net, &binary).await;
    let outside = send(
        &engine,
        &outside_probe,
        &format!("{gateway}:{published}").parse().unwrap(),
        "hello-outside",
    )
    .await;
    assert_eq!(
        outside.exit_code, 0,
        "经发布端口的包没有落到 core 网络地址的套接字：{}",
        outside.stderr
    );
    assert_eq!(outside.stdout.trim(), "hello-outside");

    // 入口侧：被攻破的入口容器在 clients 网络上发到隧道端口的包，落不到套接字。
    let attacker = names.oneshot_container("tunnel_attacker");
    start_probe(&engine, &attacker, &clients_net, &binary).await;
    let attack = send(&engine, &attacker, &clients_ip, "hello-attacker").await;
    assert_ne!(attack.exit_code, 0, "clients 网络发到隧道端口的包被处理了");

    // core 侧：网络里的服务照常连得上。
    let core_probe = names.oneshot_container("tunnel_core_probe");
    start_probe(&engine, &core_probe, &core_net, &binary).await;
    let core_side = send(&engine, &core_probe, &core_ip, "hello-coreside").await;
    assert_eq!(
        core_side.exit_code, 0,
        "core 网络里的服务连不上隧道：{}",
        core_side.stderr
    );
    assert_eq!(core_side.stdout.trim(), "hello-coreside");

    // 资源回收交给 Names 守卫，这里主动删一遍好让失败也干净。
    for container in [&listener, &outside_probe, &attacker, &core_probe] {
        engine.remove_container(container).await.unwrap();
    }
    for network in [&core_net, &clients_net, &outside_net] {
        engine.remove_network(network).await.unwrap();
    }
}
