//! 真实 Tuwunel（v1.9.4）经核心的联邦转发组件互通（ADR-0050）：两套同时起。Tuwunel 的配置就是
//! `wonton-core init` 写的那份，只把代理地址换成测试进程里的转发组件（Linux 上还换了监听地址，见
//! [`HOST_NETWORK`]）；两台的 `/etc/hosts` 和 DNS 里都没有对方的名字，联邦只能经 CONNECT 代理和 iroh 走。

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyper::{Method, StatusCode};
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::test_utils::run_relay_server;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, SecretKey};
use tokio::sync::watch;
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::{
    FEDERATION_ALPN, FederationForwarder, FederationProtocol, OutboundErrors, PeerStatus,
};

use super::PROXY_ADDR;
use super::tests::{
    Account, ClusterServer, create_room, image, invite, join, register_user, wait_ready,
};
use crate::http::{HttpClient, RequestBody};

const WAIT: Duration = Duration::from_secs(15);

/// Tuwunel 要连回测试进程里的转发组件。Linux 上宿主的防火墙（比如 ufw）常常挡住容器到宿主的连接（CI 的
/// runner 上实测），回环地址不受影响，所以 Tuwunel 用宿主的网络、只听回环地址，端口由它自己挑（同
/// wonton-matrix 的 testkit）。别处（Docker Desktop、OrbStack）两套各在自己的 Docker 网络上，经
/// `host.docker.internal` 到宿主。
const HOST_NETWORK: bool = cfg!(target_os = "linux");

/// 服务器名 → 对方的 endpoint ID 和地址，两个转发组件共用（核心按 `core/peers.json` 查的形状）。
type Pairings = Arc<Mutex<BTreeMap<String, (EndpointId, EndpointAddr)>>>;

#[test]
#[ignore = "需要 Docker，跑一遍约一分钟"]
fn two_installs_on_one_host_federate_through_the_core_proxy() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    // 失败时 iroh 的 warn（重新绑定失败、endpoint 关闭）跟着测试输出一起打出来。
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();
    runtime.block_on(async {
        let pairings: Pairings = Arc::default();
        let (relays, _, _relay) = run_relay_server().await.unwrap();
        let a = Core::start(&pairings, &relays).await;
        let b = Core::start(&pairings, &relays).await;
        // 两套同时起，谁也不指定子网。
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (stack_a, stack_b) = std::thread::scope(|scope| {
            let started_a = scope.spawn(|| Stack::start(dir_a.path(), a.proxy_port));
            let started_b = scope.spawn(|| Stack::start(dir_b.path(), b.proxy_port));
            (started_a.join().unwrap(), started_b.join().unwrap())
        });
        pairings.lock().unwrap().extend([
            (stack_a.server_name.clone(), (a.id(), a.endpoint.addr())),
            (stack_b.server_name.clone(), (b.id(), b.endpoint.addr())),
        ]);
        let _routers = [
            a.accept_from(&b, stack_a.federation_port),
            b.accept_from(&a, stack_b.federation_port),
        ];
        a.paired
            .send_replace(BTreeSet::from([stack_b.server_name.clone()]));
        b.paired
            .send_replace(BTreeSet::from([stack_a.server_name.clone()]));

        let server_a = stack_a.server();
        let server_b = stack_b.server();
        wait_ready(&server_a.address).await;
        wait_ready(&server_b.address).await;
        // 配对成立时核心做的事：重写名单、发 SIGUSR1。要等 Tuwunel 起好，启动中收到的信号不会重读配置。
        // 代理在 tuwunel.toml 里，热加载以后照样生效。
        stack_a.allow(&stack_b.server_name);
        stack_b.allow(&stack_a.server_name);
        let alice = stack_a.register("alice").await;
        let bob = stack_b.register("bob").await;

        // A → B（邀请）和 B → A（加入）都经各自的代理；第一次要先等 well-known 失败、取对方的签名密钥。
        let room = create_room(&server_a, &alice).await;
        let (on_a, on_b, alice, bob, room) = (&server_a, &server_b, &alice, &bob, room.as_str());
        let invited = until("跨机邀请和加入", || async move {
            invite(on_a, alice, room, &bob.user_id).await?;
            join(on_b, &bob.token, room).await
        })
        .await;
        invited.unwrap_or_else(|error| panic!("{error}\n{}", diagnose(&a, &b, &pairings)));
        let body = format!("你好 {}", crate::secret::random_token());
        send_text(on_a, alice, room, &body).await;
        let body = body.as_str();
        let delivered = until("B 收到 A 发的消息", || async move {
            received(on_b, bob, room, body).await
        })
        .await;
        delivered.unwrap_or_else(|error| panic!("{error}\n{}", diagnose(&a, &b, &pairings)));

        for core in [&a, &b] {
            assert_eq!(
                core.forwarder.outbound_errors(),
                OutboundErrors::default(),
                "Tuwunel 经代理的请求都应认得出、都在配对名单里"
            );
        }
        let states = a.forwarder.watch_status().borrow().clone();
        assert_eq!(
            states.get(&stack_b.server_name),
            Some(&PeerStatus::Connected),
            "{states:?}"
        );
    });
}

/// 测试进程里的一台“核心”：endpoint 加联邦转发组件。
struct Core {
    endpoint: Endpoint,
    forwarder: FederationForwarder,
    proxy_port: u16,
    paired: watch::Sender<BTreeSet<String>>,
}

impl Core {
    /// 和正式的核心一样带中继：宿主上 Docker 增删网卡时 iroh 在原端口重新绑定 UDP，同进程正在起子进程
    /// （docker CLI）时子进程在 exec 前还拿着这个 fd，绑定报 AddrInUse；只有 IP 传输的 endpoint 会因此
    /// 整个关掉，有中继的只丢直连（iroh 1.4.1，Linux 上实测）。
    async fn start(pairings: &Pairings, relays: &RelayMap) -> Self {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .relay_mode(RelayMode::Custom(relays.clone()))
            .ca_tls_config(CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        tokio::time::timeout(WAIT, endpoint.online())
            .await
            .expect("测试中继应在几秒内连上");
        let (paired, paired_rx) = watch::channel(BTreeSet::new());
        let table = pairings.clone();
        // 不用宿主网络时容器经 host-gateway 连进来，所以绑在所有地址上；正式安装只绑在核心的内部网络上。
        let forwarder = FederationForwarder::bind(
            endpoint.clone(),
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            Arc::new(move |name: &str| table.lock().unwrap().get(name).cloned()),
            paired_rx,
            Duration::from_millis(500),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        let proxy_port = forwarder.listen_addr().port();
        Self {
            endpoint,
            forwarder,
            proxy_port,
            paired,
        }
    }

    fn id(&self) -> EndpointId {
        self.endpoint.addr().id
    }

    /// 入站：只认 `peer`，流接到本机 Tuwunel 映射出来的联邦端口。
    fn accept_from(&self, peer: &Core, federation_port: u16) -> Router {
        let protocol = FederationProtocol::new(
            Allowlist::new([peer.id()]),
            SocketAddr::from((Ipv4Addr::LOCALHOST, federation_port)),
        );
        Router::builder(self.endpoint.clone())
            .accept(FEDERATION_ALPN, protocol)
            .spawn()
    }
}

/// 一套安装：Tuwunel 和（不用宿主网络时）自己的 Docker 网络，Drop 时删掉。
struct Stack {
    dir: std::path::PathBuf,
    network: Option<String>,
    container: String,
    server_name: String,
    registration_secret: String,
    federation_port: u16,
    proxy: String,
}

impl Stack {
    /// `wonton-core init` 写出安装目录，把配置里的代理地址换成测试进程里转发组件的地址。
    fn start(dir: &Path, proxy_port: u16) -> Self {
        crate::install::init(dir, None).unwrap();
        let secrets = dir.join("secrets");
        let read = |name: &str| std::fs::read_to_string(secrets.join(name)).unwrap();
        let config = std::fs::read_to_string(dir.join("secrets/tuwunel/wonton-fed.toml")).unwrap();
        let proxy_host = if HOST_NETWORK {
            "127.0.0.1"
        } else {
            "host.docker.internal"
        };
        let config = config.replace(PROXY_ADDR, &format!("{proxy_host}:{proxy_port}"));
        std::fs::write(dir.join("secrets/tuwunel/wonton-fed.toml"), config).unwrap();
        let token = crate::secret::random_token();
        let mut stack = Self {
            dir: dir.to_owned(),
            network: (!HOST_NETWORK).then(|| format!("fedproxy-test-{token}")),
            container: format!("fedproxy-test-{token}"),
            server_name: read("matrix-server-name").trim().to_owned(),
            registration_secret: read("matrix-registration-secret").trim().to_owned(),
            proxy: format!("{proxy_host}:{proxy_port}"),
            federation_port: 0,
        };
        if let Some(network) = &stack.network {
            docker(&["network", "create", network]);
        }
        docker(&stack.run_args());
        stack.federation_port = if HOST_NETWORK {
            stack.listening_port()
        } else {
            let mapped = docker(&["port", &stack.container, "8448/tcp"]);
            mapped
                .lines()
                .next()
                .and_then(|line| line.rsplit(':').next())
                .and_then(|port| port.parse().ok())
                .unwrap_or_else(|| panic!("读不出映射的端口：{mapped}"))
        };
        stack
    }

    /// 用宿主网络时 Tuwunel 自己挑的端口，从它的日志里读（客户端和联邦同一个端口）。
    fn listening_port(&self) -> u16 {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let logs = docker(&["logs", &self.container]);
            let port = logs.lines().rev().find_map(|line| {
                let (_, port) = line.split_once("Listening on [\"tcp:127.0.0.1:")?;
                port.split('"').next()?.parse().ok()
            });
            match port {
                Some(port) => return port,
                None if Instant::now() > deadline => {
                    panic!("Tuwunel 的日志里没有它挑上的端口：{logs}")
                }
                None => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    /// 正式 compose 里 tuwunel 服务的命令和安装目录下的绑定挂载（命名卷不挂），加上 [`HOST_NETWORK`]
    /// 定的网络和端口，代理变量照 compose 置空。
    fn run_args(&self) -> Vec<String> {
        let compose: serde_json::Value = serde_saphyr::from_str(crate::install::COMPOSE).unwrap();
        let tuwunel = &compose["services"]["tuwunel"];
        let strings = |value: &serde_json::Value| -> Vec<String> {
            value
                .as_array()
                .expect("compose 里是列表")
                .iter()
                .map(|item| item.as_str().expect("compose 里是字符串").to_owned())
                .collect()
        };
        let mut args: Vec<String> = [
            "run",
            "--detach",
            "--name",
            &self.container,
            "--env",
            "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml",
        ]
        .map(str::to_owned)
        .into();
        let network = match &self.network {
            Some(network) => [
                "--network",
                network,
                "--add-host",
                "host.docker.internal:host-gateway",
                "--publish",
                "127.0.0.1::8448",
            ],
            None => [
                "--network",
                "host",
                "--env",
                "TUWUNEL_ADDRESS=127.0.0.1",
                "--env",
                "TUWUNEL_PORT=0",
            ],
        };
        args.extend(network.map(str::to_owned));
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            args.extend(["--env".to_owned(), format!("{name}=")]);
        }
        let install_dir = self.dir.to_string_lossy();
        for volume in strings(&tuwunel["volumes"]) {
            if volume.starts_with("${WONTON_INSTALL_DIR}/") {
                let mount = volume.replace("${WONTON_INSTALL_DIR}", &install_dir);
                args.extend(["--volume".to_owned(), mount]);
            }
        }
        args.push(image());
        args.extend(strings(&tuwunel["command"]));
        args
    }

    /// 把对方放进名单并热加载（核心配对成立时做的事）。
    fn allow(&self, peer: &str) {
        std::fs::write(
            self.dir.join("secrets/tuwunel/wonton-fed.toml"),
            super::config_toml(
                &self.server_name,
                &BTreeSet::from([peer.to_owned()]),
                &BTreeSet::new(),
            )
            .replace(PROXY_ADDR, &self.proxy),
        )
        .unwrap();
        docker(&["kill", "--signal", "SIGUSR1", &self.container]);
    }

    fn server(&self) -> ClusterServer {
        ClusterServer {
            name: self.container.clone(),
            address: format!("http://127.0.0.1:{}", self.federation_port),
            fed_path: None,
        }
    }

    async fn register(&self, user: &str) -> Account {
        let address = self.server().address;
        register_user(
            &HttpClient::new(),
            &address,
            &self.registration_secret,
            user,
        )
        .await
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let logs = Command::new("docker")
                .args(["logs", "--tail", "30", &self.container])
                .output();
            if let Ok(logs) = logs {
                eprintln!(
                    "{} 的日志：\n{}{}",
                    self.container,
                    String::from_utf8_lossy(&logs.stdout),
                    String::from_utf8_lossy(&logs.stderr)
                );
            }
        }
        let _ = Command::new("docker")
            .args(["rm", "--force", "--volumes", &self.container])
            .output();
        if let Some(network) = &self.network {
            let _ = Command::new("docker")
                .args(["network", "rm", network])
                .output();
        }
    }
}

fn docker<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("运行 Docker");
    assert!(
        output.status.success(),
        "docker {:?}：{}",
        args.iter().map(|arg| arg.as_ref()).collect::<Vec<_>>(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// 重试到成功为止，一分钟内不成功就带最后一次的错误返回。
async fn until<F, Fut>(what: &str, mut attempt: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match attempt().await {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() > deadline => {
                return Err(format!("{what}没有成功：{error}"));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// 失败时两个转发组件看到的：对方的状态（含最近一次的错误）、出站计数和配对表里对方的地址。
fn diagnose(a: &Core, b: &Core, pairings: &Pairings) -> String {
    let side = |name: &str, core: &Core| {
        format!(
            "{name}（{}）：状态 {:?}，出站错误 {:?}，自己现在的地址 {:?}",
            core.id().fmt_short(),
            core.forwarder.watch_status().borrow(),
            core.forwarder.outbound_errors(),
            core.endpoint.addr(),
        )
    };
    format!(
        "{}\n{}\n配对表：{:?}",
        side("A", a),
        side("B", b),
        pairings.lock().unwrap()
    )
}

async fn send_text(server: &ClusterServer, account: &Account, room: &str, body: &str) {
    let txn = crate::secret::random_token();
    let (status, value) = HttpClient::new()
        .request_json(
            Method::PUT,
            &format!(
                "{}/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}",
                server.address
            ),
            Some(&account.token),
            &[],
            RequestBody::Json(&serde_json::json!({"msgtype": "m.text", "body": body})),
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{value}");
}

/// 对方同步到的房间时间线里有没有这条消息。
async fn received(
    server: &ClusterServer,
    account: &Account,
    room: &str,
    body: &str,
) -> Result<(), String> {
    let (_, value) = HttpClient::new()
        .request_json(
            Method::GET,
            &format!("{}/_matrix/client/v3/sync", server.address),
            Some(&account.token),
            &[],
            RequestBody::Empty,
            WAIT,
        )
        .await
        .map_err(|error| format!("同步失败：{error}"))?;
    let events = value["rooms"]["join"][room]["timeline"]["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if events
        .iter()
        .any(|event| event["content"]["body"].as_str() == Some(body))
    {
        Ok(())
    } else {
        Err(format!("时间线里还没有：{events:?}"))
    }
}
