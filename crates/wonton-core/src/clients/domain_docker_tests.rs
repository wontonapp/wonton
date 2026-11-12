//! `entry_certificate_contract`：同一组断言跑两个 DNS provider（lego `httpreq` 和 `acme-dns`）。
//!
//! 真 Pebble、`pebble-challtestsrv` 和一个只在测试里存在的 HTTP 写入器充当 DNS 服务商；入口是一条真的
//! HAProxy，USR2 用正在下载的连接验证。要 Docker，标 `#[ignore]`。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::entry_certificate::{CertificateConfig, CertificateManager, LEGO_IMAGE};
use crate::entry_compose::HAPROXY_IMAGE;
use crate::entry_haproxy::{Model, render};
use crate::entry_settings::EntrySettings;
use crate::plugin::docker::Engine;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::{Names, ensure_image};

const PEBBLE: &str = "ghcr.io/letsencrypt/pebble:2.8.0@sha256:d9080f68f6cb6af8d82134ab26de0aaaf312ac9cba42aecc6d3aede6cb63007b";
const CHALLTESTSRV: &str = "ghcr.io/letsencrypt/pebble-challtestsrv:latest@sha256:c390a96cc878a7d5bf5e27aaf99af532c46c4b38416d72f7219ab9ded51bb79f";
const ACME_DNS: &str =
    "joohoi/acme-dns:v1.0@sha256:08435bf0c6cfe7c29816ff48beabcc333743b5bb060cae3867777d9b0d956517";
const ZONE: &str = "example.test";
/// 按 amd64 的镜像摘要固定（不是 manifest list 的摘要：编译机对 list 摘要的 HEAD 不稳）。
const PYTHON: &str =
    "python:3.12-alpine@sha256:e6dae589c1c05976188fd3bd019afd091445fc533ed2ea29001beddc7368a9c8";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    HttpReq,
    AcmeDns,
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

/// 本次运行专属的子网，和周围的任务不撞。
fn subnet() -> (String, String) {
    let hash = getrandom::u32().unwrap();
    let v4 = format!("10.{}.{}.0/24", 30 + (hash % 150), (hash >> 8) & 0xff);
    (v4.trim_end_matches(".0/24").to_owned(), v4)
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

/// 证书叶子里的 DNS 名字。
fn subject_names(pem: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(Ok(der)) = rustls_pemfile::certs(&mut std::io::Cursor::new(pem)).next() {
        let (_, certificate) = x509_parser::parse_x509_certificate(&der).unwrap();
        for name in certificate
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value
            .general_names
            .iter()
        {
            if let x509_parser::extensions::GeneralName::DNSName(name) = name {
                names.push((*name).to_owned());
            }
        }
    }
    names
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

/// 辅助容器里的脚本：RAW 模式的 httpreq 写入 challtestsrv、按后缀转发 DNS、慢下载。
const HELPER: &str = r#"
import base64, hashlib, http.client, json, socket, struct, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

chall, acme, zone, apex, lan = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
if not chall.startswith("http"):
    chall = "http://" + chall
chall_host, chall_port = chall.split("://", 1)[1].rsplit(":", 1)
chall_addr = chall.split("://", 1)[1]

txts = {}
lock = threading.Lock()

def name_bytes(name):
    return b"".join(bytes([len(p)]) + p.encode() for p in name.rstrip(".").split(".")) + b"\x00"

def post(path, body):
    connection = http.client.HTTPConnection(chall_host, int(chall_port), timeout=5)
    connection.request("POST", path, body=json.dumps(body), headers={"content-type": "application/json"})
    connection.getresponse().read()
    connection.close()

def txt_values(host):
    labels = host.rstrip(".").split(".")
    question = b"".join(bytes([len(p)]) + p.encode() for p in labels) + b"\x00\x00\x10\x00\x01"
    packet = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + question
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(3)
    try:
        sock.sendto(packet, (chall_host_of(acme), 53))
        data = sock.recvfrom(4096)[0]
    except OSError:
        return []
    position = 12
    while data[position]:
        position += 1 + data[position]
    position += 5
    total = struct.unpack("!H", data[6:8])[0]
    values = []
    for _ in range(total):
        while True:
            length = data[position]
            if length & 0xC0 == 0xC0:
                position += 2
                break
            if length == 0:
                position += 1
                break
            position += 1 + length
        rtype, length = struct.unpack("!H", data[position:position + 2])[0], struct.unpack("!H", data[position + 8:position + 10])[0]
        rdata = data[position + 10:position + 10 + length]
        position += 10 + length
        if rtype == 16:
            values.append(rdata[1:].decode())
    return values

def chall_host_of(address):
    return address.rsplit(":", 1)[0]

def cname_answer(data, end, target):
    question = data[12:end + 5]
    rdata = name_bytes(target)
    header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", 1) + b"\x00\x00\x00\x00"
    answer = b"\xc0\x0c" + struct.pack("!HHIH", 5, 1, 1, len(rdata)) + rdata
    return header + question + answer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        try:
            self.handle_post()
        except Exception:
            import traceback
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()

    def handle_post(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"{}")
        host = "_acme-challenge." + body.get("domain", "").removeprefix("*.") + "."
        if self.path == "/present":
            value = base64.urlsafe_b64encode(hashlib.sha256(body.get("keyAuth", "").encode()).digest()).rstrip(b"=").decode()
            post("/set-txt", {"host": host, "value": value})
            with lock:
                txts.setdefault(host, []).append(value)
        elif self.path == "/cleanup":
            post("/clear-txt", {"host": host})
            with lock:
                txts.pop(host, None)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")
    def do_GET(self):
        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.end_headers()
        for index in range(10):
            self.wfile.write(("chunk-%d\n" % index).encode())
            self.wfile.flush()
            time.sleep(0.15)
    def log_message(self, *args):
        pass

def dns():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", 53))
    while True:
        data, peer = sock.recvfrom(1500)
        end = 12
        labels = []
        while data[end]:
            length = data[end]
            labels.append(data[end + 1:end + 1 + length].decode(errors="ignore"))
            end += 1 + length
        name = ".".join(labels).lower() + "."
        qtype = struct.unpack("!H", data[end + 1:end + 3])[0]
        target_name = None
        if name == "_acme-challenge." + zone + "." and apex != "-":
            target_name = apex
        elif name == "_acme-challenge.lan." + zone + "." and lan != "-":
            target_name = lan
        if target_name:
            values = txt_values(target_name)
            packet = cname_answer(data, end, target_name)
            if qtype == 16 and values:
                question = data[12:end + 5]
                header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", 1 + len(values)) + b"\x00\x00\x00\x00"
                rdata = name_bytes(target_name)
                answers = b"\xc0\x0c" + struct.pack("!HHIH", 5, 1, 1, len(rdata)) + rdata
                for value in values:
                    encoded = bytes([len(value)]) + value.encode()
                    answers += b"\xc0\x0c" + struct.pack("!HHIH", 16, 1, 1, len(encoded)) + encoded
                packet = header + question + answers
            sock.sendto(packet, peer)
            continue
        if qtype == 16 and name.startswith("_acme-challenge."):
            with lock:
                values = list(txts.get(name, []))
            question = data[12:end + 5]
            header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", len(values)) + b"\x00\x00\x00\x00"
            answers = b""
            for value in values:
                encoded = bytes([len(value)]) + value.encode()
                answers += b"\xc0\x0c" + struct.pack("!HHIH", 16, 1, 1, len(encoded)) + encoded
            sock.sendto(header + question + answers, peer)
            continue
        target = acme if acme != "-" and name.endswith(".auth." + zone + ".") else chall_addr
        host = target.rsplit(":", 1)[0]
        port = "53"
        upstream = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        upstream.settimeout(5)
        try:
            upstream.sendto(data, (host, int(port)))
            sock.sendto(upstream.recvfrom(4096)[0], peer)
        except OSError as error:
            print("dns error", error, flush=True)

import threading
threading.Thread(target=dns, daemon=True).start()
ThreadingHTTPServer(("0.0.0.0", 8484), Handler).serve_forever()

"#;

#[allow(clippy::too_many_lines)]
async fn case(provider: Provider) -> (Vec<u8>, Vec<u8>) {
    let mut names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::connect().expect("基础设施失败：连不上 Docker");
    let haproxy = ensure_image(&engine, &wonton_test_images::image(HAPROXY_IMAGE)).await;
    let pebble = ensure_image(&engine, &wonton_test_images::image(PEBBLE)).await;
    let challtestsrv = ensure_image(&engine, &wonton_test_images::image(CHALLTESTSRV)).await;
    let _lego = ensure_image(&engine, &wonton_test_images::image(LEGO_IMAGE)).await;
    let acme_dns_image = ensure_image(&engine, &wonton_test_images::image(ACME_DNS)).await;
    let python = ensure_image(&engine, &wonton_test_images::image(PYTHON)).await;

    let (base, cidr) = subnet();
    let network = names.oneshot_network("cert");
    must_docker(&[
        "network",
        "create",
        "--label",
        &format!("{LABEL_INSTANCE}={}", names.name()),
        "--subnet",
        &cidr,
        &network,
    ]);
    let challenge_ip = format!("{base}.5");
    let pebble_ip = format!("{base}.6");
    let entry_ip = format!("{base}.11");
    let connector_ip = format!("{base}.21");

    let chall = names.oneshot_container("challtestsrv");
    start(
        &names,
        &network,
        &chall,
        None,
        &challenge_ip,
        &[],
        &[],
        &challtestsrv,
        "/app",
        &[
            "-dnsserver",
            ":53",
            "-http01",
            "",
            "-https01",
            "",
            "-tlsalpn01",
            "",
        ],
    );

    let mut acme_dns_ip = None;
    let mut acme_dns_container = None;
    let mut acme_dns = None;
    if provider == Provider::AcmeDns {
        let container = names.oneshot_container("acme-dns");
        let config = dir.path().join("acme-dns/config.cfg");
        write(
            &config,
            &format!(
                "[general]\nlisten = \"0.0.0.0:53\"\nprotocol = \"both4\"\ndomain = \"auth.{ZONE}\"\nnsname = \"auth.{ZONE}\"\nnsadmin = \"admin.{ZONE}\"\nrecords = [\"auth.{ZONE}. A {pebble_ip}\", \"auth.{ZONE}. NS auth.{ZONE}.\"]\ndebug = false\n[database]\nengine = \"sqlite3\"\nconnection = \"/var/lib/acme-dns/acme-dns.db\"\n[api]\nip = \"0.0.0.0\"\nport = \"8080\"\ntls = \"none\"\ndisable_registration = false\ncorsorigins = []\nuse_header = false\n[logconfig]\nloglevel = \"info\"\nlogtype = \"stdout\"\nlogformat = \"text\"\n"
            ),
        );
        let address = format!("{base}.8");
        start(
            &names,
            &network,
            &container,
            None,
            &address,
            &[],
            &[(config, "/etc/acme-dns/config.cfg".into())],
            &acme_dns_image,
            "/root/acme-dns",
            &[],
        );
        wait_http(&format!("http://{address}:8080/health"), None)
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let first: Value = client
            .post(format!("http://{address}:8080/register"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let second: Value = client
            .post(format!("http://{address}:8080/register"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let credentials = |value: &Value| crate::entry_certificate::AcmeDnsCredentials {
            username: value["username"].as_str().unwrap().to_owned(),
            password: value["password"].as_str().unwrap().to_owned(),
            subdomain: value["subdomain"].as_str().unwrap().to_owned(),
            fulldomain: value["fulldomain"].as_str().unwrap().to_owned(),
        };
        acme_dns_ip = Some(address.clone());
        acme_dns_container = Some(container.clone());
        acme_dns = Some((
            format!("http://{address}:8080"),
            credentials(&first),
            credentials(&second),
        ));
    }

    // 辅助容器：lego 的 httpreq 写入器、DNS 转发（`*.auth.<zone>` 给 acme-dns）和慢下载后端。
    // 容器到宿主绑的端口不通，所以这些都放进容器。
    let helper_ip = format!("{base}.7");
    let helper_script = dir.path().join("helper.py");
    write(&helper_script, HELPER);
    let helper = names.oneshot_container("helper");
    start(
        &names,
        &network,
        &helper,
        None,
        &helper_ip,
        &["entry-gateway.wonton.internal"],
        &[(helper_script, "/helper.py".into())],
        &python,
        "python3",
        &[
            "/helper.py",
            &format!("{challenge_ip}:8055"),
            &acme_dns_ip.clone().unwrap_or_else(|| "-".into()),
            ZONE,
            &acme_dns
                .as_ref()
                .map(|(_, apex, _)| apex.fulldomain.clone())
                .unwrap_or_else(|| "-".into()),
            &acme_dns
                .as_ref()
                .map(|(_, _, lan)| lan.fulldomain.clone())
                .unwrap_or_else(|| "-".into()),
        ],
    );
    let dns_zone_servers = vec![format!("{helper_ip}:53")];

    // 连接器：PROXY v2 把客户端的真实来源交给入口。
    let connector = names.oneshot_container("connector");
    let connector_config = dir.path().join("connector.cfg");
    write(
        &connector_config,
        "global\ndefaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\nresolvers docker\n  nameserver docker 127.0.0.11:53\n  resolve_retries 1\n  timeout resolve 1s\n  timeout retry 1s\n  hold valid 10s\nfrontend c\n  bind :8443\n  default_backend entry\nbackend entry\n  server entry entry-edge.wonton.internal:8443 resolvers docker init-addr libc,none send-proxy-v2\n",
    );
    start(
        &names,
        &network,
        &connector,
        None,
        &connector_ip,
        &["connector.wonton.internal"],
        &[(connector_config, "/config/connector.cfg".into())],
        &haproxy,
        "haproxy",
        &["-W", "-db", "-f", "/config/connector.cfg"],
    );

    // Pebble 自己的 HTTPS：临时 CA 签发带 IP SAN 的叶证书。
    let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let mut leaf_params = rcgen::CertificateParams::new(vec![pebble_ip.clone()]).unwrap();
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let pebble_tls = dir.path().join("pebble");
    write(&pebble_tls.join("cert.pem"), &leaf.pem());
    write(&pebble_tls.join("key.pem"), &leaf_key.serialize_pem());
    write(&pebble_tls.join("ca.pem"), &ca.pem());
    write(
        &pebble_tls.join("pebble.json"),
        &serde_json::to_string(&json!({"pebble": {
            "listenAddress": "0.0.0.0:14000",
            "managementListenAddress": "0.0.0.0:15000",
            "certificate": "/config/cert.pem",
            "privateKey": "/config/key.pem",
            "httpPort": 5002,
            "tlsPort": 5001,
            "externalAccountBindingRequired": false
        }}))
        .unwrap(),
    );

    let pebble_container = names.oneshot_container("pebble");
    start(
        &names,
        &network,
        &pebble_container,
        None,
        &pebble_ip,
        &[],
        &[(pebble_tls, "/config".into())],
        &pebble,
        "/app",
        &[
            "-config",
            "/config/pebble.json",
            "-dnsserver",
            &format!("{helper_ip}:53"),
            "-strict=false",
        ],
    );
    wait_http(
        &format!("https://{pebble_ip}:14000/dir"),
        Some(&dir.path().join("pebble/ca.pem")),
    )
    .await
    .unwrap();

    // 入口链路：连接器（PROXY v2）→ 入口 → 辅助容器。
    let install = dir.path().join("install");
    std::fs::create_dir_all(install.join("entry/certs")).unwrap();
    let env_file = dir.path().join("provider.env");
    write(
        &env_file,
        &format!("HTTPREQ_ENDPOINT=http://{helper_ip}:8484\nHTTPREQ_MODE=RAW\n"),
    );
    // 先自检辅助容器的写入器和 DNS 转发，和 lego 要走的路径一样。
    let preflight = reqwest::Client::new()
        .post(format!("http://{helper_ip}:8484/present"))
        .json(&json!({"domain": ZONE, "keyAuth": "preflight"}))
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 200, "辅助写入器不回 200");
    // 自检的那条值要清掉，否则 lego 的传播预检会看到一条对不上的 TXT。
    reqwest::Client::new()
        .post(format!("http://{helper_ip}:8484/cleanup"))
        .json(&json!({"domain": ZONE, "keyAuth": "preflight"}))
        .send()
        .await
        .unwrap();
    let manager = CertificateManager::new(
        install.clone(),
        engine.clone(),
        CertificateConfig {
            directory_url: format!("https://{pebble_ip}:14000/dir"),
            ca_file: Some(dir.path().join("pebble/ca.pem")),
            dns_resolvers: dns_zone_servers.clone(),
            dns_propagation_disable_ans: true,
            email: None,
            accept_terms: true,
            network: network.clone(),
            host_install_dir: install.clone(),
            include_lan: true,
            lan_acme_dns: acme_dns.as_ref().map(|(_, _, lan)| lan.clone()),
            dns_provider: (provider == Provider::HttpReq).then(|| "httpreq".to_owned()),
            dns_env_file: (provider == Provider::HttpReq).then_some(env_file),
        },
    )
    .unwrap();
    let account = manager
        .prepare(
            ZONE,
            acme_dns.as_ref().map(|(_, apex, _)| apex),
            acme_dns.as_ref().map(|(url, _, _)| url.as_str()),
        )
        .await
        .unwrap();
    assert!(account.starts_with("https://"), "账号 URI 不对：{account}");
    if let Err(error) = manager.issue(ZONE).await {
        let (_, helper_logs) = docker(&["logs", "--tail", "40", &helper]);
        let (_, chall_logs) = docker(&["logs", "--tail", "20", &chall]);
        let env = std::fs::read_to_string(install.join("entry/acme/dns.env")).unwrap_or_default();
        let acme_logs = acme_dns_container
            .as_deref()
            .map(|name| docker(&["logs", "--tail", "30", name]).1)
            .unwrap_or_default();
        panic!(
            "签发失败：{error:#}\ndns.env:\n{env}\nacme-dns:\n{acme_logs}\nhelper:\n{helper_logs}\nchalltestsrv:\n{chall_logs}"
        );
    }

    // 证书本身。
    let site = manager.certificate_path();
    assert_eq!(site, install.join("entry/certs/site.pem"));
    let pem = std::fs::read(&site).unwrap();
    assert!(pem.starts_with(b"-----BEGIN CERTIFICATE-----"));
    assert!(
        pem.windows(b"-----BEGIN PRIVATE KEY-----".len())
            .any(|part| part == b"-----BEGIN PRIVATE KEY-----")
    );
    let metadata = std::fs::metadata(&site).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), std::fs::metadata(&install).unwrap().uid());
    let names_in_cert = subject_names(&pem);
    for expected in [
        ZONE.to_owned(),
        format!("*.{ZONE}"),
        format!("*.lan.{ZONE}"),
    ] {
        assert!(
            names_in_cert.contains(&expected),
            "证书没有 {expected}：{names_in_cert:?}"
        );
    }

    // 私钥没挂进 lego：挂载里只有 acme 目录和只读的 CSR。
    let body = manager.container_body(vec!["run".to_owned()]).unwrap();
    let mounts = body.host_config.unwrap().mounts.unwrap();
    assert!(mounts.iter().all(|mount| {
        let source = mount.source.as_deref().unwrap_or_default();
        !source.ends_with("certs") && !source.ends_with("site.key")
    }));
    assert!(mounts.iter().any(|mount| {
        mount.target.as_deref() == Some("/request/site.csr") && mount.read_only == Some(true)
    }));

    // USR2 不新起连接也不断掉正在下载的。
    let entry = names.oneshot_container("entry");
    let certs = install.join("entry/certs");
    let mut model = Model::federation_only(ZONE, ZONE);
    model.public_labels = Vec::new();
    model.connectors = vec![connector_ip.clone()];
    let rendered = render(&model).unwrap();
    write(&certs.join("public.crt-list"), &rendered.public_crt_list);
    write(&certs.join("lan.crt-list"), &rendered.lan_crt_list);
    let entry_config = dir.path().join("entry.cfg");
    write(&entry_config, &rendered.config);
    let owner = format!(
        "{}:{}",
        std::fs::metadata(&site).unwrap().uid(),
        std::fs::metadata(&site).unwrap().gid()
    );
    start(
        &names,
        &network,
        &entry,
        Some(&owner),
        &entry_ip,
        &["entry-edge.wonton.internal", "entry.wonton.internal"],
        &[
            (certs, "/certs".into()),
            (entry_config, "/config/haproxy.cfg".into()),
        ],
        &haproxy,
        "haproxy",
        &["-W", "-db", "-f", "/config/haproxy.cfg"],
    );
    tokio::time::sleep(Duration::from_millis(800)).await;
    let roots = dir.path().join("roots.pem");
    let roots_text = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
        .get(format!("https://{pebble_ip}:15000/roots/0"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    std::fs::write(&roots, roots_text).unwrap();
    let output = dir.path().join("download");
    let mut child = Command::new("curl")
        .args([
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
            &format!("probe.{ZONE}:8443:{connector_ip}"),
            &format!("https://probe.{ZONE}:8443/slow"),
            "-o",
            output.to_str().unwrap(),
            "-w",
            "%{size_download}",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (_, status) = docker(&["inspect", "-f", "{{.State.Status}}", &entry]);
    if status.trim() != "running" {
        let (_, logs) = docker(&["logs", "--tail", "30", &entry]);
        panic!("入口在收 USR2 前已退出：\n{logs}");
    }
    engine.kill_container(&entry, "SIGUSR2").await.unwrap();
    let result = child.wait().unwrap();
    assert!(result.success(), "USR2 后下载失败");
    let body = std::fs::read_to_string(&output).unwrap();
    assert_eq!(body.lines().count(), 10, "连接被截断：{body:?}");

    // 服务商换了，入口配置和插件渲染出来的 compose 都不变。
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
    let base_compose = "services:\n  core:\n    volumes: []\n    networks:\n      core:\n        aliases: []\n    environment: {}\n  tuwunel:\n    volumes: []\n";
    let compose = crate::entry_compose::render(base_compose, Some(&settings), &install).unwrap();
    // 每次的子网不同：把这条随机地址归一成固定值再比。
    (
        rendered
            .config
            .replace(&connector_ip, "connector.test")
            .into_bytes(),
        compose.into_bytes(),
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，在 61 上跑"]
async fn entry_certificate_contract() {
    let httpreq = case(Provider::HttpReq).await;
    let acme_dns = case(Provider::AcmeDns).await;
    assert_eq!(httpreq.0, acme_dns.0, "两个服务商渲染出的入口配置不同");
    assert_eq!(httpreq.1, acme_dns.1, "两个服务商渲染出的 compose 不同");
}

use std::os::unix::fs::{MetadataExt, PermissionsExt};
