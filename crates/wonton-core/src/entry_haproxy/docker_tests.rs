//! 真 HAProxy（[`super::super::entry_compose::HAPROXY_IMAGE`]）跑渲染结果：先用 `-c` 校验三种模型，
//! 再起一个入口、两个连接器和一个假后端，验证对上一条条的实测行为。

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use super::Model;
use super::model::Trust;
use super::render::render;
use crate::plugin::docker::Engine;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::{Names, ensure_image};

/// 和 `entry_compose::HAPROXY_IMAGE` 相同；`scripts/mirror-test-images.sh` 只认本文件里的字面量，
/// 两边由 [`image_matches_the_entry_image`] 锁住。
const IMAGE: &str =
    "haproxy:3.2.8-alpine@sha256:8c6b55d891584afded3ab1b91063a703ecc8ff2481302b691773648ffa806226";
/// 客户端容器用的 curl（Apache-2.0）。宿主 curl 经 PROXY 声明的来源永远是网桥地址，
/// 分不出「另一个来源」，封禁按来源隔离要两个固定地址的客户端。
const CURL: &str = "curlimages/curl:8.11.1@sha256:c1fe1679c34d9784c1b0d1e5f62ac0a79fca01fb6377cdd33e90473c6f9f9a69";

const CONNECTOR_ALIAS: &str = "connector.wonton.internal";
const SUFFIX: &str = "home.example.test";

fn write_files(dir: &Path, trust: Trust, setup: &Setup, with_lan: bool) -> String {
    let mut model = Model::federation_only(SUFFIX, "cloud.example.test");
    model.public_labels = vec!["immich".to_owned()];
    model.lan_labels = vec!["forgejo".to_owned()];
    model.trust = trust;
    model.connectors = vec![setup.connector4.clone(), setup.connector4b.clone()];
    model.lan_forward = with_lan.then(|| setup.lan4.clone());
    let rendered = render(&model).unwrap();
    let certs = dir.join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    // 证书和私钥同一个 PEM，和入口真正的发布方式一样。
    let tls = rcgen::generate_simple_self_signed(vec![
        format!("*.{SUFFIX}"),
        format!("*.lan.{SUFFIX}"),
        "cloud.example.test".to_owned(),
    ])
    .unwrap();
    std::fs::write(
        certs.join("site.pem"),
        format!("{}{}", tls.cert.pem(), tls.signing_key.serialize_pem()),
    )
    .unwrap();
    std::fs::write(certs.join("public.crt-list"), &rendered.public_crt_list).unwrap();
    std::fs::write(certs.join("lan.crt-list"), &rendered.lan_crt_list).unwrap();
    std::fs::write(dir.join("haproxy.cfg"), &rendered.config).unwrap();
    dir.join("haproxy.cfg").display().to_string()
}

/// 真连本机 Docker：`Engine::for_test` 是给不碰 Docker 的测试用的桩。
fn connected_engine() -> Engine {
    Engine::connect().expect("基础设施失败：连不上 Docker")
}

fn docker(args: &[&str]) -> (bool, String) {
    let output = Command::new("docker").args(args).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    (output.status.success(), text)
}

fn must_docker(args: &[&str]) -> String {
    let (ok, text) = docker(args);
    assert!(ok, "docker {args:?} 失败：{text}");
    text
}

/// 所有容器的 `--network` 参数：测试网络、地址和（有的）别名。
fn network_args(network: &str, labels: &Names, address: &str, aliases: &[&str]) -> Vec<String> {
    let mut args = vec![
        "--network".to_owned(),
        network.to_owned(),
        "--ip".to_owned(),
        address.to_owned(),
        "--label".to_owned(),
        format!("{LABEL_INSTANCE}={}", labels.name()),
    ];
    for alias in aliases {
        args.push("--network-alias".to_owned());
        args.push((*alias).to_owned());
    }
    args
}

/// 本次运行专属的子网：从实例名派生，彼此不撞，也不撞宿主上常见网段。
fn subnets(name: &str) -> (String, String) {
    let hash = name.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let v4 = format!("10.{}.{}.0/24", 16 + hash % 200, (hash >> 8) & 0xff);
    let v6 = format!(
        "fd{:02x}:{:04x}:{:04x}::/64",
        (hash >> 24) & 0xff,
        ((hash >> 16) & 0xff) << 8 | (hash >> 8) & 0xff,
        hash & 0xff
    );
    (v4, v6)
}

fn owned(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

const BACKEND_CONFIG: &str = "global\ndefaults\n  mode http\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\nfrontend b\n  bind [::]:8484\n  http-request set-var(txn.fail) str(1) if { req.hdr(x-fail) -m found }\n  http-after-response set-header X-Wonton-Auth-Failure 1 if { var(txn.fail) -m found }\n  http-request return status 200 content-type text/plain lf-string \"entry=%[req.hdr(X-Wonton-Entry)] real=%[req.hdr(X-Real-IP)] fwd=%[req.hdr(X-Forwarded-For)] host=%[req.hdr(Host)] und=%[req.hdr(X_Remote_User)]\"\n";

/// 连接器：PROXY v2 模式下带 `send-proxy-v2`；头信任模式（Cloudflare）是普通 TCP 转发，不带 PROXY 头。
fn connector_config(proxy: bool) -> String {
    let proxy = if proxy { " send-proxy-v2" } else { "" };
    format!(
        "global\ndefaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\nfrontend p\n  bind [::]:8443\n  default_backend entry\nbackend entry\n  server entry entry-edge.wonton.internal:8443{proxy}\n"
    )
}

struct Setup {
    network: String,
    backend4: String,
    entry4: String,
    connector4: String,
    connector4b: String,
    connector6: String,
    connector6b: String,
    lan4: String,
    client4a: String,
    client4b: String,
    client6a: String,
    client6b: String,
}

/// 请求从哪个来源发：两个客户端容器各自有 IPv4 和 IPv6 地址。
#[derive(Clone, Copy)]
enum From {
    A,
    B,
}

struct Chain {
    /// 清理由它负责（Drop 里按实例名收容器和网络）。
    _names: Names,
    /// 挂给容器的证书目录：删了容器里的 crt-list 还在，客户端也没了根证书。
    _dir: tempfile::TempDir,
    entry4: String,
    connector4: String,
    connector4b: String,
    connector6: String,
    connector6b: String,
    client_a: String,
    client_b: String,
    client4a: String,
}

fn setup_network(names: &mut Names) -> Setup {
    let network = names.oneshot_network("entry");
    let (subnet4, subnet6) = subnets(names.name());
    let base4 = subnet4.trim_end_matches(".0/24").to_owned();
    let base6 = subnet6.trim_end_matches("::/64").to_owned();
    must_docker(&[
        "network",
        "create",
        "--label",
        &format!("{LABEL_INSTANCE}={}", names.name()),
        "--ipv6",
        "--subnet",
        &subnet4,
        "--subnet",
        &subnet6,
        &network,
    ]);
    Setup {
        network,
        backend4: format!("{base4}.10"),
        entry4: format!("{base4}.11"),
        connector4: format!("{base4}.21"),
        connector4b: format!("{base4}.22"),
        connector6: format!("{base6}::21"),
        connector6b: format!("{base6}::22"),
        lan4: format!("{base4}.23"),
        client4a: format!("{base4}.31"),
        client4b: format!("{base4}.32"),
        client6a: format!("{base6}::31"),
        client6b: format!("{base6}::32"),
    }
}

/// 起一个 HAProxy 容器：名字带着本次运行的随机实例名，DNS 里统一用固定别名。
#[allow(clippy::too_many_arguments)] // 网络、名字、地址、挂载、镜像和命令行各管各的。
fn start_container(
    names: &Names,
    network: &str,
    container: &str,
    address: &str,
    ip6: Option<&str>,
    aliases: &[&str],
    mounts: &[(String, String)],
    image: &str,
    entrypoint: &str,
    command: &[&str],
) {
    let mut args = vec![
        "run".to_owned(),
        "-d".to_owned(),
        "--name".to_owned(),
        container.to_owned(),
        "--entrypoint".to_owned(),
        entrypoint.to_owned(),
    ];
    args.extend(network_args(network, names, address, aliases));
    if let Some(ip6) = ip6 {
        args.push("--ip6".to_owned());
        args.push(ip6.to_owned());
    }
    for (host, guest) in mounts {
        args.push("-v".to_owned());
        args.push(format!("{host}:{guest}:ro"));
    }
    args.push(image.to_owned());
    for arg in command {
        args.push((*arg).to_owned());
    }
    must_docker(&owned(&args));
}

struct Spawned {
    gateway: String,
    entry: String,
    connector: String,
    connector_b: String,
    client_a: String,
    client_b: String,
}

#[allow(clippy::too_many_arguments)] // 测试夹具的整套地址、配置和镜像，拆开反而看不清。
fn spawn_chain(
    names: &mut Names,
    setup: &Setup,
    image: &str,
    image_curl: &str,
    certs: &Path,
    config: &str,
    gateway_cfg: &Path,
    connector_cfg: &Path,
) -> Spawned {
    let gateway = names.oneshot_container("gateway");
    let entry = names.oneshot_container("entry");
    let connector = names.oneshot_container("connector-a");
    let connector_b = names.oneshot_container("connector-b");
    let http = ["-W", "-db", "-f", "/c/haproxy.cfg"];
    start_container(
        names,
        &setup.network,
        &gateway,
        &setup.backend4,
        None,
        &["entry-gateway.wonton.internal"],
        &[(
            gateway_cfg.display().to_string(),
            "/c/haproxy.cfg".to_owned(),
        )],
        image,
        "haproxy",
        &http,
    );
    start_container(
        names,
        &setup.network,
        &entry,
        &setup.entry4,
        None,
        &[
            "entry-edge.wonton.internal",
            "entry.wonton.internal",
            "entry-lanfwd.wonton.internal",
            "lanfwd.wonton.internal",
        ],
        &[
            (certs.display().to_string(), "/certs".to_owned()),
            (config.to_owned(), "/c/haproxy.cfg".to_owned()),
        ],
        image,
        "haproxy",
        &http,
    );
    // 两个连接器共用一个别名：入口按名字认对端，两个地址都要能过；IPv6 落在同一个 /64。
    for (name, address, ip6) in [
        (&connector, &setup.connector4, &setup.connector6),
        (&connector_b, &setup.connector4b, &setup.connector6b),
    ] {
        start_container(
            names,
            &setup.network,
            name,
            address,
            Some(ip6),
            &[CONNECTOR_ALIAS],
            &[(
                connector_cfg.display().to_string(),
                "/c/haproxy.cfg".to_owned(),
            )],
            image,
            "haproxy",
            &http,
        );
    }
    let client_a = names.oneshot_container("client-a");
    let client_b = names.oneshot_container("client-b");
    for (name, address, ip6) in [
        (&client_a, &setup.client4a, &setup.client6a),
        (&client_b, &setup.client4b, &setup.client6b),
    ] {
        start_container(
            names,
            &setup.network,
            name,
            address,
            Some(ip6),
            &[],
            &[(certs.display().to_string(), "/certs".to_owned())],
            image_curl,
            "sleep",
            &["infinity"],
        );
    }
    Spawned {
        gateway,
        entry,
        connector,
        connector_b,
        client_a,
        client_b,
    }
}

impl Chain {
    async fn start(trust: Trust, lan_forward: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let certs = dir.path().join("certs");
        let engine = connected_engine();
        let image = ensure_image(&engine, &wonton_test_images::image(IMAGE)).await;
        let image_curl = ensure_image(&engine, &wonton_test_images::image(CURL)).await;
        let mut names = Names::new();
        let setup = setup_network(&mut names);
        let proxy = matches!(trust, Trust::ProxyV2);
        let config = write_files(dir.path(), trust, &setup, lan_forward.is_some());
        let gateway_cfg = dir.path().join("gateway.cfg");
        std::fs::write(&gateway_cfg, BACKEND_CONFIG).unwrap();
        let connector_cfg = dir.path().join("connector.cfg");
        std::fs::write(&connector_cfg, connector_config(proxy)).unwrap();
        let started = spawn_chain(
            &mut names,
            &setup,
            &image,
            &image_curl,
            &certs,
            &config,
            &gateway_cfg,
            &connector_cfg,
        );
        wait_loaded(&started.gateway);
        wait_loaded(&started.entry);
        wait_loaded(&started.connector);
        wait_loaded(&started.connector_b);
        Self {
            _names: names,
            _dir: dir,
            entry4: setup.entry4,
            connector4: setup.connector4,
            connector4b: setup.connector4b,
            connector6: setup.connector6,
            connector6b: setup.connector6b,
            client_a: started.client_a,
            client_b: started.client_b,
            client4a: setup.client4a,
        }
    }

    /// 从客户端容器发一个请求；返回 (状态码, 响应头, 响应体, curl 的 stderr)。
    /// 头和体落在容器里的固定路径，再用 `docker exec cat` 取出来。
    fn request(
        &self,
        url_host: &str,
        from: From,
        address: &str,
        extra: &[&str],
    ) -> (u16, String, String, String) {
        let port = 8443;
        let url = format!("https://{url_host}:{port}/");
        let container = match from {
            From::A => &self.client_a,
            From::B => &self.client_b,
        };
        let output = Command::new("docker")
            .args(["exec", container, "curl"])
            .args([
                "-sS",
                "--noproxy",
                "*",
                "--resolve",
                &format!("{url_host}:{port}:{address}"),
                "--cacert",
                "/certs/site.pem",
                "-D",
                "/tmp/headers",
                "-o",
                "/tmp/body",
                "-w",
                "%{http_code}",
            ])
            .args(extra)
            .arg(&url)
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        let headers = docker(&["exec", container, "cat", "/tmp/headers"]).1;
        let text = docker(&["exec", container, "cat", "/tmp/body"]).1;
        (
            status,
            headers,
            text,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// 对端可能以 IPv4 出现在 PROXY 头里，也可能被写成 IPv4 映射的 IPv6（连接器双栈监听时）。
fn real_is(body: &str, address: &str) -> bool {
    body.contains(&format!("real={address}")) || body.contains(&format!("real=::ffff:{address}"))
}

fn wait_loaded(container: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, logs) = docker(&["logs", container]);
        if logs.contains("Loading success") {
            return;
        }
        assert!(Instant::now() < deadline, "{container} 没起来：{logs}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn check_model(trust: Trust) {
    let dir = tempfile::tempdir().unwrap();
    let engine = connected_engine();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let image = runtime.block_on(ensure_image(&engine, &wonton_test_images::image(IMAGE)));
    let mut names = Names::new();
    let network = names.oneshot_network("check");
    let (subnet4, subnet6) = subnets(names.name());
    must_docker(&[
        "network", "create", "--ipv6", "--subnet", &subnet4, "--subnet", &subnet6, &network,
    ]);
    let base4 = subnet4.trim_end_matches(".0/24");
    let base6 = subnet6.trim_end_matches("::/64");
    let setup = Setup {
        network: network.clone(),
        backend4: format!("{base4}.10"),
        entry4: format!("{base4}.11"),
        connector4: format!("{base4}.21"),
        connector4b: format!("{base4}.22"),
        connector6: format!("{base6}::21"),
        connector6b: format!("{base6}::22"),
        lan4: format!("{base4}.23"),
        client4a: format!("{base4}.31"),
        client4b: format!("{base4}.32"),
        client6a: format!("{base6}::31"),
        client6b: format!("{base6}::32"),
    };
    write_files(dir.path(), trust, &setup, true);
    let aliases = [
        "entry-edge.wonton.internal",
        "entry-lanfwd.wonton.internal",
        "entry.wonton.internal",
        "connector.wonton.internal",
        "lanfwd.wonton.internal",
        "entry-gateway.wonton.internal",
    ];
    let mut args = vec![
        "run".to_owned(),
        "--rm".to_owned(),
        "--network".to_owned(),
        network.clone(),
        "--ip".to_owned(),
        setup.entry4.clone(),
    ];
    for alias in aliases {
        args.push("--network-alias".to_owned());
        args.push(alias.to_owned());
    }
    args.extend([
        "-v".to_owned(),
        format!("{}:/certs:ro", dir.path().join("certs").display()),
        "-v".to_owned(),
        format!(
            "{}:/c/haproxy.cfg:ro",
            dir.path().join("haproxy.cfg").display()
        ),
        image,
        "-c".to_owned(),
        "-f".to_owned(),
        "/c/haproxy.cfg".to_owned(),
    ]);
    let command = owned(&args);
    // 退出码是 `haproxy -c` 的判词；有的版本静默退出，别指望它一定打 NOTICE。
    let (ok, text) = docker(&command);
    assert!(ok, "docker {command:?} 失败：{text}");
    assert!(!text.contains("[ALERT]"), "{text}");
}

#[test]
fn image_matches_the_entry_image() {
    assert_eq!(IMAGE, crate::entry_compose::HAPROXY_IMAGE);
}

#[test]
#[ignore = "需要 Docker，起来回跑的 HAProxy 和两个连接器"]
fn rendered_configs_pass_haproxy_check() {
    check_model(Trust::ProxyV2);
    check_model(Trust::Header("CF-Connecting-IP".to_owned()));
}

#[test]
#[ignore = "需要 Docker，起入口、连接器和后端验证行为"]
fn entry_container_enforces_both_frontends() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let chain = runtime.block_on(Chain::start(Trust::ProxyV2, None));
    let name = format!("immich.{SUFFIX}");
    let client = chain.client4a.clone();

    // 列表里的名字照常过，客户端伪造的来源、入口和带下划线的头都被处理掉。
    let (status, _, body, error) = chain.request(
        &name,
        From::A,
        &chain.connector4,
        &[
            "-H",
            "X-Real-IP: 9.9.9.9",
            "-H",
            "X-Wonton-Entry: lan",
            "-H",
            "X-Forwarded-For: 9.9.9.9",
            "-H",
            "X_Remote_User: evil",
        ],
    );
    assert_eq!(status, 200, "{error}");
    assert!(body.contains("entry=public"), "{body}");
    assert!(real_is(&body, &client), "{body}");
    assert!(body.contains("fwd="), "{body}");
    assert!(body.trim_end().ends_with("und="), "{body}");

    // 列表外的 SNI 握手就失败。
    let (status, _, _, error) = chain.request(
        "unlisted.home.example.test",
        From::A,
        &chain.connector4,
        &[],
    );
    assert_eq!(status, 0);
    assert!(error.contains("unrecognized name"), "{error}");

    // Host 和 SNI 不一致回 421；带端口的形式放行。
    let (status, _, _, _) = chain.request(
        &name,
        From::A,
        &chain.connector4,
        &["-H", "Host: other.example.test"],
    );
    assert_eq!(status, 421);
    let host = format!("Host: {name}:8443");
    let (status, _, body, _) = chain.request(&name, From::A, &chain.connector4, &["-H", &host]);
    assert_eq!(status, 200);
    assert!(body.contains(&format!("host={name}:8443")), "{body}");

    // 不经过连接器（没有 PROXY 头）进不到 HTTP。
    let entry4 = chain.entry4.clone();
    let (status, _, _, _) = chain.request(&name, From::A, &entry4, &[]);
    assert_eq!(status, 0);
}

#[test]
#[ignore = "需要 Docker，起入口、连接器和后端验证头模式"]
fn header_trust_takes_the_source_from_the_header_only_through_the_connector() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let chain = runtime.block_on(Chain::start(
        Trust::Header("CF-Connecting-IP".to_owned()),
        None,
    ));
    let name = format!("immich.{SUFFIX}");
    let _ = chain.client4a.clone();
    // 经连接器来的请求，来源取头里的地址；没带就留在客户端自己身上。直连入口的一律被拒。
    let (status, _, body, error) = chain.request(
        &name,
        From::A,
        &chain.connector4,
        &["-H", "CF-Connecting-IP: 198.51.100.7"],
    );
    assert_eq!(status, 200, "{error}");
    assert!(body.contains("real=198.51.100.7"), "{body}");
    let (status, _, body, error) = chain.request(&name, From::A, &chain.connector4, &[]);
    assert_eq!(status, 200, "{error}");
    // 没带来源头时不读 PROXY，来源留在连接器上。
    assert!(
        body.contains(&format!("real={}", chain.connector4)),
        "{body}"
    );
    let entry4 = chain.entry4.clone();
    let (status, _, _, _) = chain.request(&name, From::A, &entry4, &[]);
    assert_eq!(status, 0);
}

#[test]
#[ignore = "需要 Docker，起入口、连接器和后端验证限速"]
fn auth_failures_are_counted_per_source_and_masked_to_slash_64() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let chain = runtime.block_on(Chain::start(Trust::ProxyV2, None));
    let name = format!("immich.{SUFFIX}");
    let failing = ["-H", "x-fail: 1"];
    let connector4 = chain.connector4.clone();
    let connector4b = chain.connector4b.clone();
    let connector6 = chain.connector6.clone();
    let connector6b = chain.connector6b.clone();
    // 计数超过 10 才封（和 0021-verification 的“6 次、第 7 个 429”同一套计数）：前 11 次失败照常回 200。
    for _ in 0..11 {
        let (status, headers, _, error) = chain.request(&name, From::A, &connector4, &failing);
        assert_eq!(status, 200, "{error}");
        assert!(!headers.contains("x-wonton-auth-failure"), "{headers}");
    }
    let (status, _, _, _) = chain.request(&name, From::A, &connector4, &failing);
    assert_eq!(status, 429);
    // 另一个 IPv4 来源是另一个桶。
    let (status, _, _, _) = chain.request(&name, From::B, &connector4b, &[]);
    assert_eq!(status, 200);

    // IPv6 按 /64：两个来源合起来算，超过 10 次之后两个来源都 429。
    for _ in 0..7 {
        let (status, _, _, error) = chain.request(&name, From::A, &connector6, &failing);
        assert_eq!(status, 200, "{error}");
    }
    for _ in 0..4 {
        let (status, _, _, error) = chain.request(&name, From::B, &connector6b, &failing);
        assert_eq!(status, 200, "{error}");
    }
    let (status, _, _, _) = chain.request(&name, From::B, &connector6b, &[]);
    assert_eq!(status, 429);
    let (status, _, _, _) = chain.request(&name, From::A, &connector6, &[]);
    assert_eq!(status, 429);
}
