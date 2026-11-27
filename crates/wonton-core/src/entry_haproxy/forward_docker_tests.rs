//! 真 HAProxy 跑局域网转发（ADR-0021“局域网直连”）：三张网络照入口的布局搭——`lanpub` 关掉地址伪装，
//! `lanfwd` 内部，`edge` 上是连接器的位置。验证放行谁、来源怎么传、谁够不着入口的局域网前端。
//!
//! 局域网客户端直接连转发在 `lanpub` 上的地址：端口发布在宿主局域网地址上那一步由 Docker 做，
//! 在开发栈上用真实局域网地址测（见 PR）。

use std::io::Read;
use std::net::{SocketAddrV4, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::docker_tests::{
    BACKEND_CONFIG, CURL, SUFFIX, connected_engine, docker, must_docker, real_is, subnets,
    wait_loaded,
};
use super::{Forward, Model, render, render_forward};
use crate::entry_compose::HAPROXY_IMAGE;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::{Names, ensure_image};

const LAN_NAME: &str = "forgejo";

/// 一组 /24 的前三段：从实例名和用途派生，几张网络彼此不撞。
fn base(names: &Names, what: &str) -> String {
    let (v4, _) = subnets(&format!("{}-{what}", names.name()));
    v4.trim_end_matches(".0/24").to_owned()
}

struct Lan {
    _names: Names,
    _dir: tempfile::TempDir,
    /// 转发在 `lanpub` 上的地址；`lanpub` 的网关是 `<pub>.1`。
    forward: String,
    gateway: String,
    /// 入口在 `lanfwd` 上的地址（局域网前端听在这里）和在 `edge` 上的地址。
    entry_lanfwd: String,
    entry_edge: String,
    /// 放行网段里的局域网客户端、网段外的、`lanfwd` 上冒充转发的、连接器位置上的。
    client: String,
    client_ip: String,
    outsider: String,
    intruder: String,
    connector: String,
}

fn network(names: &mut Names, what: &str, extra: &[&str]) -> (String, String) {
    let name = names.oneshot_network(what);
    let base = base(names, what);
    let mut args = vec![
        "network".to_owned(),
        "create".to_owned(),
        "--label".to_owned(),
        format!("{LABEL_INSTANCE}={}", names.name()),
        "--subnet".to_owned(),
        format!("{base}.0/24"),
        "--gateway".to_owned(),
        format!("{base}.1"),
    ];
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    args.push(name.clone());
    must_docker(&args.iter().map(String::as_str).collect::<Vec<_>>());
    (name, base)
}

/// 建容器（还不启动）：第一张网络在建时给，其余用 [`attach`] 接上再 `docker start`。
fn create(
    name: &str,
    network: &str,
    address: &str,
    aliases: &[&str],
    mounts: &[(&Path, &str)],
    image: &str,
    command: &[&str],
) {
    let mut args = vec![
        "create".to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--network".to_owned(),
        network.to_owned(),
        "--ip".to_owned(),
        address.to_owned(),
    ];
    for alias in aliases {
        args.extend(["--network-alias".to_owned(), (*alias).to_owned()]);
    }
    for (host, guest) in mounts {
        args.extend(["-v".to_owned(), format!("{}:{guest}:ro", host.display())]);
    }
    args.extend([
        "--entrypoint".to_owned(),
        command[0].to_owned(),
        image.to_owned(),
    ]);
    args.extend(command[1..].iter().map(|arg| (*arg).to_owned()));
    must_docker(&args.iter().map(String::as_str).collect::<Vec<_>>());
}

fn attach(container: &str, network: &str, address: &str, aliases: &[&str]) {
    let mut args = vec!["network", "connect", "--ip", address];
    for alias in aliases {
        args.extend(["--alias", alias]);
    }
    args.extend([network, container]);
    must_docker(&args);
}

fn write_entry(dir: &Path, connector: &str, forward_lanfwd: &str) {
    let mut model = Model::federation_only(SUFFIX, "cloud.example.test");
    model.public_labels = vec!["immich".to_owned()];
    model.lan_labels = vec![LAN_NAME.to_owned()];
    model.connectors = vec![connector.to_owned()];
    model.lan_forward = Some(forward_lanfwd.to_owned());
    let rendered = render(&model).unwrap();
    let certs = dir.join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let tls =
        rcgen::generate_simple_self_signed(vec![format!("*.{SUFFIX}"), format!("*.lan.{SUFFIX}")])
            .unwrap();
    std::fs::write(
        certs.join("site.pem"),
        format!("{}{}", tls.cert.pem(), tls.signing_key.serialize_pem()),
    )
    .unwrap();
    std::fs::write(certs.join("public.crt-list"), &rendered.public_crt_list).unwrap();
    std::fs::write(certs.join("lan.crt-list"), &rendered.lan_crt_list).unwrap();
    std::fs::write(dir.join("haproxy.cfg"), &rendered.config).unwrap();
    std::fs::write(dir.join("gateway.cfg"), BACKEND_CONFIG).unwrap();
}

/// 发请求用的客户端容器：只有 curl 和根证书。
fn client(name: &str, network: &str, address: &str, certs: &Path, curl: &str) {
    let sleep = ["sleep", "infinity"];
    create(
        name,
        network,
        address,
        &[],
        &[(certs, "/certs")],
        curl,
        &sleep,
    );
    must_docker(&["start", name]);
}

impl Lan {
    async fn start() -> Self {
        let engine = connected_engine();
        let haproxy = ensure_image(&engine, &wonton_test_images::image(HAPROXY_IMAGE)).await;
        let curl = ensure_image(&engine, &wonton_test_images::image(CURL)).await;
        let dir = tempfile::tempdir().unwrap();
        let mut names = Names::new();
        let (lanpub, pub_) = network(
            &mut names,
            "lanpub",
            &["-o", "com.docker.network.bridge.enable_ip_masquerade=false"],
        );
        let (lanfwd, fwd) = network(&mut names, "lanfwd", &["--internal"]);
        let (edge, edge_base) = network(&mut names, "edge", &[]);
        let gateway = names.oneshot_container("lan-backend");
        let entry = names.oneshot_container("lan-entry");
        let forward = names.oneshot_container("lan-forward");
        let lan = Self {
            forward: format!("{pub_}.62"),
            gateway: format!("{pub_}.1"),
            entry_lanfwd: format!("{fwd}.62"),
            entry_edge: format!("{edge_base}.11"),
            client: names.oneshot_container("lan-client"),
            client_ip: format!("{pub_}.31"),
            outsider: names.oneshot_container("lan-outsider"),
            intruder: names.oneshot_container("lan-intruder"),
            connector: names.oneshot_container("lan-connector"),
            _names: names,
            _dir: dir,
        };
        let forward_lanfwd = format!("{fwd}.61");
        write_entry(lan._dir.path(), &format!("{edge_base}.21"), &forward_lanfwd);
        let forward_cfg = lan._dir.path().join("forward.cfg");
        std::fs::write(
            &forward_cfg,
            render_forward(&Forward {
                listen: SocketAddrV4::new(lan.forward.parse().unwrap(), 8483),
                // 放行前一半：`.200` 的客户端在网段外。
                allow: format!("{pub_}.0/25").parse().unwrap(),
                deny: lan.gateway.parse().unwrap(),
                entry: SocketAddrV4::new(lan.entry_lanfwd.parse().unwrap(), 8443),
            }),
        )
        .unwrap();
        let http = ["haproxy", "-W", "-db", "-f", "/c/haproxy.cfg"];
        let dir = lan._dir.path();
        let certs = dir.join("certs");
        let config = dir.join("haproxy.cfg");
        let gateway_cfg = dir.join("gateway.cfg");
        create(
            &gateway,
            &edge,
            &format!("{edge_base}.10"),
            &["entry-gateway.wonton.internal"],
            &[(&gateway_cfg, "/c/haproxy.cfg")],
            &haproxy,
            &http,
        );
        create(
            &entry,
            &edge,
            &lan.entry_edge,
            &["entry-edge.wonton.internal", "entry.wonton.internal"],
            &[(&certs, "/certs"), (&config, "/c/haproxy.cfg")],
            &haproxy,
            &http,
        );
        attach(
            &entry,
            &lanfwd,
            &lan.entry_lanfwd,
            &["entry-lanfwd.wonton.internal"],
        );
        create(
            &forward,
            &lanpub,
            &lan.forward,
            &[],
            &[(&forward_cfg, "/c/haproxy.cfg")],
            &haproxy,
            &http,
        );
        attach(&forward, &lanfwd, &forward_lanfwd, &[]);
        let clients = [
            (&lan.client, &lanpub, lan.client_ip.clone()),
            (&lan.outsider, &lanpub, format!("{pub_}.200")),
            (&lan.intruder, &lanfwd, format!("{fwd}.30")),
            (&lan.connector, &edge, format!("{edge_base}.21")),
        ];
        for (name, network, address) in &clients {
            client(name, network, address, &certs, &curl);
        }
        for container in [&gateway, &entry, &forward] {
            must_docker(&["start", container]);
            wait_loaded(container);
        }
        lan
    }

    /// 从 `container` 发一个 HTTPS 请求到 `address:port`；返回 (状态码, 响应体, curl 的 stderr)。
    fn request(
        &self,
        container: &str,
        address: &str,
        port: u16,
        extra: &[&str],
    ) -> (u16, String, String) {
        let host = format!("{LAN_NAME}.lan.{SUFFIX}");
        let output = Command::new("docker")
            .args(["exec", container, "curl", "-sS", "--noproxy", "*"])
            .args(["--connect-timeout", "3", "-m", "10"])
            .args(["--resolve", &format!("{host}:{port}:{address}")])
            .args(["--cacert", "/certs/site.pem", "-o", "/tmp/body"])
            .args(["-w", "%{http_code}"])
            .args(extra)
            .arg(format!("https://{host}:{port}/"))
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        let body = docker(&["exec", container, "cat", "/tmp/body"]).1;
        (
            status,
            body,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

#[test]
#[ignore = "需要 Docker，起局域网转发、入口和客户端验证来源"]
fn lan_client_through_forward_reaches_the_lan_frontend_with_its_own_source() {
    let lan = runtime().block_on(Lan::start());
    let (status, body, error) = lan.request(
        &lan.client,
        &lan.forward,
        8483,
        &["-H", "X-Real-IP: 9.9.9.9", "-H", "X-Wonton-Entry: public"],
    );
    assert_eq!(status, 200, "{error}");
    assert!(body.contains("entry=lan"), "{body}");
    assert!(real_is(&body, &lan.client_ip), "{body}");
    // 放行网段外的局域网地址连不进来。
    let (status, _, error) = lan.request(&lan.outsider, &lan.forward, 8483, &[]);
    assert_eq!(status, 0, "{error}");
}

#[test]
#[ignore = "需要 Docker，从宿主经网桥网关连局域网转发"]
fn bridge_gateway_source_is_reset() {
    let lan = runtime().block_on(Lan::start());
    // 宿主连网桥上的容器，来源就是网关地址：和经 docker-proxy 进来的连接同一个来源。
    let address: SocketAddrV4 = format!("{}:8483", lan.forward).parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(3))
        .unwrap_or_else(|error| panic!("宿主连不上网桥 {} 上的转发：{error}", lan.gateway));
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("来源是网关 {} 的连接没被断开：{other:?}", lan.gateway),
    }
}

#[test]
#[ignore = "需要 Docker，在 lanfwd 上冒充转发发 PROXY 头"]
fn forged_proxy_to_lan_frontend_is_rejected() {
    let lan = runtime().block_on(Lan::start());
    let (status, _, error) = lan.request(
        &lan.intruder,
        &lan.entry_lanfwd,
        8443,
        &["--haproxy-protocol", "--haproxy-clientip", "192.0.2.10"],
    );
    assert_eq!(status, 0, "{error}");
    let (status, _, error) = lan.request(&lan.intruder, &lan.entry_lanfwd, 8443, &[]);
    assert_eq!(status, 0, "{error}");
}

#[test]
#[ignore = "需要 Docker，从连接器的位置找入口的局域网前端"]
fn connector_position_cannot_reach_the_lan_frontend() {
    let lan = runtime().block_on(Lan::start());
    // `lanfwd` 上的地址不通。
    let (status, _, error) = lan.request(&lan.connector, &lan.entry_lanfwd, 8443, &[]);
    assert_eq!(status, 0, "{error}");
    // 公网前端不认局域网名字。
    let (status, _, error) = lan.request(
        &lan.connector,
        &lan.entry_edge,
        8443,
        &["--haproxy-protocol"],
    );
    assert_eq!(status, 0, "{error}");
    // `lanfwd` 是内部网络，转发也不在 `edge` 上：连接器连不到转发。
    let (status, _, error) = lan.request(&lan.connector, &lan.forward, 8483, &[]);
    assert_eq!(status, 0, "{error}");
}
