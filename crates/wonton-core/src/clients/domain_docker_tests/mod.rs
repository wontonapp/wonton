//! `entry_certificate_contract`：同一组断言跑两个 DNS provider（lego `httpreq` 和 `acme-dns`）。
//!
//! 真 Pebble、`pebble-challtestsrv` 和一个只在测试里存在的 HTTP 写入器充当 DNS 服务商；入口是一条真的
//! HAProxy，热加载用正在下载的连接验证。要 Docker，标 `#[ignore]`。

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::entry_certificate::{
    AcmeDnsCredentials, CertificateConfig, CertificateManager, LEGO_IMAGE,
};
use crate::entry_compose::HAPROXY_IMAGE;
use crate::entry_haproxy::{Model, Rendered, render};
use crate::entry_settings::EntrySettings;
use crate::plugin::docker::Engine;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::{Names, ensure_image};

mod acme_dns;
mod certificate;
mod connector;
mod helper;
mod infra;
mod issuance;
mod pebble;

const ZONE: &str = "example.test";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    HttpReq,
    AcmeDns,
}

/// 一次用例里跨阶段要用的容器、镜像和地址。
struct Case {
    names: Names,
    dir: tempfile::TempDir,
    engine: Engine,
    haproxy: String,
    pebble: String,
    python: String,
    base: String,
    network: String,
    challenge_ip: String,
    helper_ip: String,
    pebble_ip: String,
    entry_ip: String,
    connector_ip: String,
    chall: String,
    helper: String,
    entry: String,
    connector: String,
    install: PathBuf,
    dns_zone_servers: Vec<String>,
    acme_dns_ip: Option<String>,
    acme_dns_container: Option<String>,
    acme_dns: Option<(String, AcmeDnsCredentials, AcmeDnsCredentials)>,
}

impl Case {
    fn new() -> Self {
        Self {
            names: Names::new(),
            dir: tempfile::tempdir().unwrap(),
            engine: Engine::connect().expect("基础设施失败：连不上 Docker"),
            haproxy: String::new(),
            pebble: String::new(),
            python: String::new(),
            base: String::new(),
            network: String::new(),
            challenge_ip: String::new(),
            helper_ip: String::new(),
            pebble_ip: String::new(),
            entry_ip: String::new(),
            connector_ip: String::new(),
            chall: String::new(),
            helper: String::new(),
            entry: String::new(),
            connector: String::new(),
            install: PathBuf::new(),
            dns_zone_servers: Vec::new(),
            acme_dns_ip: None,
            acme_dns_container: None,
            acme_dns: None,
        }
    }
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

#[allow(clippy::too_many_arguments)]
fn start(
    names: &Names,
    network: &str,
    container: &str,
    user: Option<&str>,
    address: &str,
    aliases: &[&str],
    mounts: &[(PathBuf, String)],
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
        "--network".to_owned(),
        network.to_owned(),
        "--ip".to_owned(),
        address.to_owned(),
        "--label".to_owned(),
        format!("{LABEL_INSTANCE}={}", names.name()),
    ];
    for alias in aliases {
        args.extend(["--network-alias".to_owned(), (*alias).to_owned()]);
    }
    if let Some(user) = user {
        args.extend(["--user".to_owned(), user.to_owned()]);
    }
    for (host, guest) in mounts {
        args.push("-v".to_owned());
        args.push(format!("{}:{guest}:ro", host.display()));
    }
    args.push(image.to_owned());
    args.extend(command.iter().map(|arg| (*arg).to_owned()));
    must_docker(&args.iter().map(String::as_str).collect::<Vec<_>>());
    std::thread::sleep(Duration::from_millis(400));
    let (_, status) = docker(&["inspect", "-f", "{{.State.Status}}", container]);
    if status.trim() != "running" {
        let (_, logs) = docker(&["logs", "--tail", "20", container]);
        panic!("{container} 起来后立刻退出：\n{logs}");
    }
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

async fn wait_http(url: &str, root: Option<&Path>) -> Result<()> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(2));
    if let Some(root) = root {
        builder = builder.add_root_certificate(
            reqwest::Certificate::from_pem(&std::fs::read(root)?)
                .context("Pebble 的根证书不是 PEM")?,
        );
    }
    let client = builder.build()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if client.get(url).send().await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() > deadline {
            bail!("等不到 {url}");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

// 入口：按渲染出的配置起一条真 HAProxy，属主用证书文件的。

fn entry_up(case: &mut Case, manager: &CertificateManager) -> Rendered {
    let entry = case.names.oneshot_container("entry");
    let certs = case.install.join("entry/certs");
    let mut model = Model::federation_only(ZONE, ZONE);
    model.public_labels = Vec::new();
    model.connectors = vec![case.connector_ip.clone()];
    let rendered = render(&model).unwrap();
    write(&certs.join("public.crt-list"), &rendered.public_crt_list);
    write(&certs.join("lan.crt-list"), &rendered.lan_crt_list);
    let entry_config = case.dir.path().join("entry.cfg");
    write(&entry_config, &rendered.config);
    let site = manager.certificate_path();
    let owner = format!(
        "{}:{}",
        std::fs::metadata(&site).unwrap().uid(),
        std::fs::metadata(&site).unwrap().gid()
    );
    start(
        &case.names,
        &case.network,
        &entry,
        Some(&owner),
        &case.entry_ip,
        &["entry-edge.wonton.internal", "entry.wonton.internal"],
        &[
            (certs, "/certs".into()),
            (entry_config, "/config/haproxy.cfg".into()),
        ],
        &case.haproxy,
        "haproxy",
        // 和产品一样开主进程管理端口（-S）：热加载走它，expose-fd 靠它把监听套接字交给新进程。
        &[
            "-W",
            "-db",
            "-f",
            "/config/haproxy.cfg",
            "-S",
            &format!("{}:8404", case.entry_ip),
        ],
    );
    case.entry = entry;
    rendered
}

// 热加载不新起连接也不断掉正在下载的。
/// 经连接器下载 `/slow`：`-Sv` 的 stderr 留给外面，返回还在下载的子进程。
async fn hot_reload_download(case: &Case, roots: &Path, output: &Path) -> std::process::Child {
    let connector_ip = &case.connector_ip;
    let through = |extra: &[&str]| {
        let resolve = format!("probe.{ZONE}:8443:{connector_ip}");
        let url = format!("https://probe.{ZONE}:8443/slow");
        let mut args = vec![
            "-sS",
            "--noproxy",
            "*",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "--cacert",
            roots.to_str().unwrap(),
            "--resolve",
            resolve.as_str(),
            url.as_str(),
        ];
        args.extend_from_slice(extra);
        let mut command = Command::new("curl");
        command.args(&args);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap()
    };
    // 连接器的 DNS 解析是懒的（init-addr libc,none）：先打通一次，别把解析慢记成重载丢包。
    for _ in 0..60 {
        if through(&["-o", "/dev/null", "--max-time", "3"])
            .wait()
            .unwrap()
            .success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let child = through(&[
        "-Sv",
        "-o",
        output.to_str().unwrap(),
        "-w",
        "%{size_download}",
    ]);

    // 等第一个字节真的落盘再重载：没被 accept 的连接在监听套接字换手时不会跟着走，
    // 固定 sleep 会让“下载中”有时其实还没开始。
    for _ in 0..100 {
        if std::fs::metadata(output).map(|m| m.len()).unwrap_or(0) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child
}

async fn hot_reload_check(case: &Case) {
    tokio::time::sleep(Duration::from_millis(800)).await;
    let roots = case.dir.path().join("roots.pem");
    let roots_text = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
        .get(format!("https://{}:15000/roots/0", case.pebble_ip))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    std::fs::write(&roots, roots_text).unwrap();
    let output = case.dir.path().join("download");
    let mut child = hot_reload_download(case, &roots, &output).await;
    let (_, status) = docker(&["inspect", "-f", "{{.State.Status}}", &case.entry]);
    if status.trim() != "running" {
        let (_, logs) = docker(&["logs", "--tail", "30", &case.entry]);
        panic!("入口在热加载前已退出：\n{logs}");
    }
    crate::entry_containers::reload_haproxy(&format!("{}:8404", case.entry_ip))
        .await
        .unwrap();
    let result = child.wait().unwrap();
    if !result.success() {
        let (_, elogs) = docker(&["logs", "--timestamps", "--tail", "40", &case.entry]);
        let (_, clogs) = docker(&["logs", "--timestamps", "--tail", "20", &case.connector]);
        let (_, info) = docker(&[
            "inspect",
            "-f",
            "{{.State.Status}} {{.RestartCount}}",
            &case.entry,
        ]);
        panic!(
            "热加载后下载失败: {info}
entry:
{elogs}
connector:
{clogs}"
        );
    }
    let body = std::fs::read_to_string(&output).unwrap();
    assert_eq!(body.lines().count(), 10, "连接被截断：{body:?}");
}

// 服务商换了，入口配置和插件渲染出来的 compose 都不变。
fn rendered_compare(case: &Case, rendered: Rendered) -> (Vec<u8>, Vec<u8>) {
    let settings = EntrySettings {
        domain: ZONE.into(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: Some("https://pebble.test:14000/dir".into()),
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_propagation_disable_ans: true,
        acme_email: None,
        acme_accept_terms: true,
        acme_network: "bridge".into(),
        dns_provider: None,
        dns_env_file: None,
        external_certificate: false,
    };
    let install = case.install.clone();
    let base_compose = "services:\n  core:\n    volumes: []\n    networks:\n      core:\n        aliases: []\n    environment: {}\n  tuwunel:\n    volumes: []\n";
    let compose = crate::entry_compose::render(base_compose, Some(&settings), &install).unwrap();
    // 每次的子网不同：把这条随机地址归一成固定值再比。
    (
        rendered
            .config
            .replace(&case.connector_ip, "connector.test")
            .into_bytes(),
        compose.into_bytes(),
    )
}

/// 每个服务商跑一遍：起环境、签发、验证书，再验热加载和渲染结果。
async fn case(provider: Provider) -> (Vec<u8>, Vec<u8>) {
    let mut case = Case::new();
    infra::up(&mut case).await;
    if provider == Provider::AcmeDns {
        acme_dns::up(&mut case).await;
    }
    helper::up(&mut case);
    connector::up(&mut case);
    pebble::up(&mut case).await;
    let manager = issuance::issue(&mut case, provider).await;
    certificate::check(&case, &manager);
    let rendered = entry_up(&mut case, &manager);
    hot_reload_check(&case).await;
    rendered_compare(&case, rendered)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，在 61 上跑"]
async fn entry_certificate_contract() {
    let httpreq = case(Provider::HttpReq).await;
    let acme_dns = case(Provider::AcmeDns).await;
    assert_eq!(httpreq.0, acme_dns.0, "两个服务商渲染出的入口配置不同");
    assert_eq!(httpreq.1, acme_dns.1, "两个服务商渲染出的 compose 不同");
}
