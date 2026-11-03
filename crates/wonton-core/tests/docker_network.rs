//! 核心接入、退出插件网络时，经发布端口连进核心的连接不断（#90）。要真的 Docker Engine：默认网关和发布端口
//! 落在哪个端点，是 Engine 的行为。
//!
//! 用一个回显的容器扮演核心：它在“核心的网络”上发布端口，再像装、卸能出网的插件时那样接入、退出一个名字排在
//! 前面的普通桥接网络。

use std::collections::HashMap;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, HostConfig, NetworkCreateRequest, NetworkInspect, PortBinding,
    RestartPolicy, RestartPolicyNameEnum,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::core_address::{Endpoint, last_usable};
use wonton_core::plugin::docker::{Engine, NetworkReach};
use wonton_core::plugin::network_migration::MigrationStore;

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

/// 测试发布的固定端口，按进程分开，避开系统给出站连接分配的端口（32768 起）。
fn fixed_port(test: u32) -> u16 {
    u16::try_from(20000 + (std::process::id() + test) % 12000).unwrap()
}

/// 建网络的名字前缀，和别的测试、别的进程分开。
fn prefix(test: &str) -> String {
    format!("wonton-pintest{}-{test}", std::process::id())
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn containers_get_the_lower_half_and_the_core_keeps_its_address_across_recreation() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
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

/// 核心升级前的建法：不指定网段，服务 `db`、`web` 带别名，`web` 发布一个固定的端口（插件的局域网端口都是固定的）。
async fn legacy(engine: &Engine, network: &str, web: &str, db: &str, port: u16) {
    docker()
        .create_network(NetworkCreateRequest {
            name: network.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(HashMap::from([(
                "wonton.plugin".to_owned(),
                "app.test".to_owned(),
            )])),
            ..Default::default()
        })
        .await
        .unwrap();
    for (name, alias) in [(web, "web"), (db, "db")] {
        let binding = PortBinding {
            host_ip: Some("127.0.0.1".to_owned()),
            host_port: Some(port.to_string()),
        };
        let published = alias == "web";
        let body = ContainerCreateBody {
            image: Some(IMAGE.to_owned()),
            cmd: Some(vec!["sleep".to_owned(), "3600".to_owned()]),
            exposed_ports: published.then(|| vec![PORT.to_owned()]),
            host_config: Some(HostConfig {
                network_mode: Some(network.to_owned()),
                port_bindings: published
                    .then(|| HashMap::from([(PORT.to_owned(), Some(vec![binding]))])),
                ..Default::default()
            }),
            networking_config: Some(bollard::models::NetworkingConfig {
                endpoints_config: Some(HashMap::from([(
                    network.to_owned(),
                    bollard::models::EndpointSettings {
                        aliases: Some(vec![alias.to_owned()]),
                        ..Default::default()
                    },
                )])),
            }),
            ..Default::default()
        };
        engine.create_container(name, body).await.unwrap();
        engine.start_container(name).await.unwrap();
    }
}

/// 等 `text` 出现在容器的 /tmp/got 里，最多 30 秒。
async fn appears(container: &str, text: &str) -> bool {
    for _ in 0..60 {
        if run_in(container, &["grep", "-q", text, "/tmp/got"]).await == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// 没启动过的容器，接在 `network` 上，如退出了的一次性服务。
async fn stopped(engine: &Engine, name: &str, network: &str) {
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec!["true".to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(network.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(name, body).await.unwrap();
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

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn migration_keeps_addresses_and_aliases_and_resumes_after_an_interruption() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
        .await
        .unwrap();
    let network = prefix("migrate");
    let (web, db) = (format!("{network}--web"), format!("{network}--db"));
    let init = format!("{network}--init");
    cleanup(&engine, &network, &[&web, &db, &init]).await;
    let port = fixed_port(0);
    legacy(&engine, &network, &web, &db, port).await;
    stopped(&engine, &init, &network).await;
    // 每 0.5 秒写一行的长连接，迁移前连上，迁移后还在写。nc 的标准输入读到头会关掉连接，要一直开着。后台进程不能占着
    // exec 的输出，否则 exec 要等它退出。
    let listen = "exec > /dev/null 2>&1 < /dev/null; sleep 3600 | nc -l -p 9000 > /tmp/got &";
    assert_eq!(run_in(&db, &["sh", "-c", listen]).await, 0);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let write = "exec > /dev/null 2>&1 < /dev/null; \
                 (i=0; while [ $i -lt 30 ]; do echo line$i.; i=$((i+1)); sleep 0.5; done; sleep 3600) \
                 | nc db 9000 &";
    assert_eq!(run_in(&web, &["sh", "-c", write]).await, 0);
    assert!(appears(&db, "line0\\.").await, "迁移前要先连上");
    let before_subnet = inspect(&network).await.ipam.unwrap().config.unwrap()[0]
        .subnet
        .clone();
    let web_before = engine.endpoint_address(&web, &network).await.unwrap();
    let db_before = engine.endpoint_address(&db, &network).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = MigrationStore::new(dir.path().to_owned());
    assert!(
        engine.core_address(&network).await.is_err(),
        "没迁移前没有留地址"
    );

    engine
        .migrate_network(&network, None, &store)
        .await
        .unwrap();

    let config = inspect(&network)
        .await
        .ipam
        .unwrap()
        .config
        .unwrap()
        .remove(0);
    assert_eq!(config.subnet, before_subnet);
    assert!(config.ip_range.is_some_and(|r| !r.is_empty()));
    assert_eq!(
        engine.endpoint_address(&web, &network).await.unwrap(),
        web_before
    );
    assert_eq!(
        engine.endpoint_address(&db, &network).await.unwrap(),
        db_before
    );
    assert_eq!(run_in(&web, &["ping", "-c", "1", "-W", "2", "db"]).await, 0);
    // 停着的容器还能启动：记着的是新网络。
    engine.start_container(&init).await.unwrap();
    assert!(
        appears(&db, "line29\\.").await,
        "迁移时经这个网络的长连接断了"
    );
    let serve = "exec > /dev/null 2>&1 < /dev/null; while true; do nc -l -p 7000 -e cat; done &";
    assert_eq!(run_in(&web, &["sh", "-c", serve]).await, 0);
    connect(port).await;
    // 迁移过的不再迁移，也不留记录。
    engine
        .migrate_network(&network, None, &store)
        .await
        .unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // 记录写好、网络删了、还没重建时核心退出：从记录接着做。
    let db_address = db_before.clone().unwrap();
    engine.disconnect_network(&network, &web).await.unwrap();
    engine.disconnect_network(&network, &db).await.unwrap();
    engine.remove_network(&network).await.unwrap();
    let saved = serde_json::json!({
        "subnet": before_subnet,
        "gateway": null,
        "driver": "bridge",
        "internal": false,
        "options": {},
        "labels": {},
        "endpoints": {&db: {"address": db_address, "aliases": ["db"], "gw_priority": null}},
    });
    std::fs::write(
        dir.path().join(format!("{network}.json")),
        saved.to_string(),
    )
    .unwrap();
    engine
        .migrate_network(&network, None, &store)
        .await
        .unwrap();
    assert_eq!(
        engine.endpoint_address(&db, &network).await.unwrap(),
        db_before
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    cleanup(&engine, &network, &[&web, &db, &init]).await;
}

/// 不能出网的插件：端口发布在 `--lan` 网络上，默认路由也走它。迁移的是插件网络，`--lan` 不能受影响。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn migrating_a_plugin_network_leaves_the_lan_network_and_its_port_alone() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
        .await
        .unwrap();
    let network = prefix("lan");
    let lan = format!("{network}--lan");
    let web = format!("{network}--web");
    cleanup(&engine, &network, &[&web]).await;
    engine.remove_network(&lan).await.unwrap();
    docker()
        .create_network(NetworkCreateRequest {
            name: network.clone(),
            driver: Some("bridge".to_owned()),
            internal: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    engine
        .create_lan_network(&lan, &HashMap::new())
        .await
        .unwrap();
    let port = fixed_port(1);
    let binding = PortBinding {
        host_ip: Some("127.0.0.1".to_owned()),
        host_port: Some(port.to_string()),
    };
    let endpoint = |aliases: Vec<String>, gw_priority| bollard::models::EndpointSettings {
        aliases: Some(aliases),
        gw_priority: Some(gw_priority),
        ..Default::default()
    };
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(
            ["sh", "-c", "while true; do nc -l -p 7000 -e cat; done"]
                .map(str::to_owned)
                .into(),
        ),
        exposed_ports: Some(vec![PORT.to_owned()]),
        host_config: Some(HostConfig {
            network_mode: Some(network.clone()),
            port_bindings: Some(HashMap::from([(PORT.to_owned(), Some(vec![binding]))])),
            ..Default::default()
        }),
        networking_config: Some(bollard::models::NetworkingConfig {
            endpoints_config: Some(HashMap::from([
                (network.clone(), endpoint(vec!["web".to_owned()], 0)),
                (lan.clone(), endpoint(Vec::new(), 1)),
            ])),
        }),
        ..Default::default()
    };
    engine.create_container(&web, body).await.unwrap();
    engine.start_container(&web).await.unwrap();
    drop(connect(port).await);
    let lan_address = engine.endpoint_address(&web, &lan).await.unwrap();
    let address = engine.endpoint_address(&web, &network).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = MigrationStore::new(dir.path().to_owned());
    assert!(
        engine
            .migrate_network(&network, None, &store)
            .await
            .unwrap()
    );

    assert_eq!(
        engine.endpoint_address(&web, &network).await.unwrap(),
        address
    );
    assert_eq!(
        engine.endpoint_address(&web, &lan).await.unwrap(),
        lan_address
    );
    assert_eq!(inspect(&network).await.internal, Some(true));
    drop(connect(port).await);
    let lan_gateway = inspect(&lan).await.ipam.unwrap().config.unwrap()[0]
        .gateway
        .clone()
        .unwrap();
    let route = format!("ip route show default | grep -q {lan_gateway}");
    let default_route = run_in(&web, &["sh", "-c", &route]).await;
    cleanup(&engine, &network, &[&web]).await;
    engine.remove_network(&lan).await.unwrap();
    assert_eq!(default_route, 0, "默认路由要走 --lan 网络");
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
        .ensure_image(IMAGE, PullPolicy::Missing)
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

/// 重启循环中的容器没有端点，直接断开时 Docker 报“没接着”，配置里却还记着旧网络的 ID：网络重建后它再也起不来。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_crash_looping_container_keeps_looping_after_migration() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
        .await
        .unwrap();
    let network = prefix("loop");
    let looping = format!("{network}--app");
    cleanup(&engine, &network, &[&looping]).await;
    let request = NetworkCreateRequest {
        name: network.clone(),
        driver: Some("bridge".to_owned()),
        ..Default::default()
    };
    docker().create_network(request).await.unwrap();
    let body = ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(["sh", "-c", "exit 1"].map(str::to_owned).into()),
        host_config: Some(HostConfig {
            network_mode: Some(network.clone()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::ALWAYS),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&looping, body).await.unwrap();
    engine.start_container(&looping).await.unwrap();
    let state = async || {
        engine
            .inspect_container(&looping)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
    };
    // 重启的间隔每次翻倍。前几次太短，迁移开始时可能已经过去了。
    let mut settled = false;
    for _ in 0..400 {
        let info = engine.inspect_container(&looping).await.unwrap().unwrap();
        settled = info.restart_count.unwrap_or_default() >= 4
            && info.state.unwrap().restarting == Some(true);
        if settled {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(settled, "要在重启循环里迁移");

    let dir = tempfile::tempdir().unwrap();
    let store = MigrationStore::new(dir.path().to_owned());
    engine
        .migrate_network(&network, None, &store)
        .await
        .unwrap();

    // 起不来时几秒内就变成 exited，错误是找不到旧网络。
    tokio::time::sleep(Duration::from_secs(8)).await;
    let after = state().await;
    cleanup(&engine, &network, &[&looping]).await;
    assert_eq!(after.error.as_deref().unwrap_or_default(), "");
    assert!(
        after.restarting == Some(true) || after.running == Some(true),
        "{after:?}"
    );
}

/// 迁移删网络的时候，别的网络拿走了这个网段：换一段建回去，插件的容器接回来，地址由 Docker 重新分配。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_subnet_taken_during_migration_moves_the_network_to_another() {
    let _serial = SERIAL.lock().await;
    let engine = Engine::connect().unwrap();
    engine
        .ensure_image(IMAGE, PullPolicy::Missing)
        .await
        .unwrap();
    let network = prefix("taken");
    let squatter = format!("{network}-squatter");
    let app = format!("{network}--app");
    cleanup(&engine, &network, &[&app]).await;
    engine.remove_network(&squatter).await.unwrap();
    let plain = |name: &str| NetworkCreateRequest {
        name: name.to_owned(),
        driver: Some("bridge".to_owned()),
        ..Default::default()
    };
    // 记录写好、网络删了、还没重建时，网段被占了。占着的网段由 Docker 分配：先删再用同一段建，CI 上共用 Docker 的
    // 别的作业可能抢先拿走。
    docker().create_network(plain(&squatter)).await.unwrap();
    let subnet = inspect(&squatter).await.ipam.unwrap().config.unwrap()[0]
        .subnet
        .clone()
        .unwrap();
    let held: ipnet::IpNet = subnet.parse().unwrap();
    let address = held.hosts().nth(5).map(|ip| ip.to_string());
    docker().create_network(plain(&network)).await.unwrap();
    sleeper(&engine, &app, &network).await;
    engine.disconnect_network(&network, &app).await.unwrap();
    engine.remove_network(&network).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = MigrationStore::new(dir.path().to_owned());
    let saved = serde_json::json!({
        "subnet": subnet,
        "gateway": null,
        "driver": "bridge",
        "internal": false,
        "options": {},
        "labels": {},
        "endpoints": {&app: {"address": address, "aliases": ["app"], "gw_priority": null}},
    });
    std::fs::write(
        dir.path().join(format!("{network}.json")),
        saved.to_string(),
    )
    .unwrap();

    let migrated = engine.migrate_network(&network, None, &store).await;

    let moved = inspect(&network).await.ipam.unwrap().config.unwrap()[0].clone();
    let now = engine.endpoint_address(&app, &network).await.unwrap();
    let reserved = engine.core_address(&network).await;
    cleanup(&engine, &network, &[&app]).await;
    engine.remove_network(&squatter).await.unwrap();
    migrated.unwrap();
    assert_ne!(moved.subnet.as_ref(), Some(&subnet));
    assert!(moved.ip_range.is_some_and(|r| !r.is_empty()));
    assert!(now.is_some() && now != address, "{now:?}");
    reserved.unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
