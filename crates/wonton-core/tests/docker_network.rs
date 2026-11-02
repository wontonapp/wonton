//! 核心接入、退出插件网络时，经发布端口连进核心的连接不断（#90）。要真的 Docker Engine：默认网关和发布端口
//! 落在哪个端点，是 Engine 的行为。
//!
//! 用一个回显的容器扮演核心：它在“核心的网络”上发布端口，再像装、卸能出网的插件时那样接入、退出一个名字排在
//! 前面的普通桥接网络。

use std::collections::HashMap;
use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::docker::{Engine, NetworkReach};

/// npchy 和 CI 的 runner 连不上 Docker Hub，从 ECR Public 拉。
const IMAGE: &str = "public.ecr.aws/docker/library/busybox:1.37";
const PORT: &str = "7000/tcp";

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
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let names = Names::new();
    names.remove(&engine).await;
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
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
