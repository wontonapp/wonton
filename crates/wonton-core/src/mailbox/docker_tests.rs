use std::sync::Arc;
use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use wonton_protocol::MAIL_FIELD;

use super::api::Api;
use super::test_support::{Home, PASSWORD, PLUGIN, home};
use crate::http::HttpClient;
use crate::plugin::Permission;
use crate::plugin::docker::Engine;

fn image() -> String {
    wonton_test_images::image(
        "axllent/mailpit:v1.33.0@sha256:9b3a463be15354c0f0eba91697d80e3e0d8a5623ef0e627cacc836bc17fd5231",
    )
}

#[tokio::test]
#[ignore = "需要 Docker，真实 Mailpit SMTP 收信后投进收件人自己的私聊"]
async fn smtp_mail_reaches_each_persons_private_room() {
    let home = home().await;
    let mut names = crate::test_docker::Names::new();
    let engine = Engine::connect().unwrap();
    // 建容器用 ensure_image 返回的引用：它可能和 image() 不同（加速站），按另一个名字建容器会
    // 报 `No such image`，即使镜像其实已经在本地。
    let image = crate::test_docker::ensure_image(&engine, &image()).await;
    let name = names.oneshot_container("mailpit");
    let pair = certificate(home.dir.path());
    create_mailpit(&engine, &name, &image, home.dir.path()).await;
    engine.start_container(&name).await.unwrap();
    let settings = engine
        .inspect_container(&name)
        .await
        .unwrap()
        .unwrap()
        .network_settings
        .unwrap();
    let bindings = settings.ports.unwrap();
    let port = |key: &str| {
        bindings[key].as_ref().unwrap()[0]
            .host_port
            .as_ref()
            .unwrap()
            .parse::<u16>()
            .unwrap()
    };
    let gateway = settings
        .networks
        .unwrap()
        .into_values()
        .next()
        .unwrap()
        .gateway
        .unwrap();
    home.gateway.plugins.register_source_for_test(
        PLUGIN,
        &format!("{gateway}/32"),
        vec![Permission::Notify],
    );
    let authority = format!("127.0.0.1:{}", port("8025/tcp"));
    let api = Api::new(&authority, PASSWORD, HttpClient::new()).unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while api.list().await.is_err() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let worker = super::start(
        &authority,
        PASSWORD,
        HttpClient::new(),
        home.gateway.clone(),
        home.dir.path(),
    )
    .unwrap();
    smtp(port("1025/tcp")).await;
    smtp_over_starttls(port("1025/tcp"), pair.cert.der().to_vec()).await;
    assert_delivered(&home, &api).await;
    drop(worker);
    engine.remove_container(&name).await.unwrap();
}

/// 写一张给 `smtp.wonton.internal` 的自签证书，返回它好让客户端信任。
fn certificate(dir: &std::path::Path) -> rcgen::CertifiedKey<rcgen::KeyPair> {
    let pair = rcgen::generate_simple_self_signed(vec!["smtp.wonton.internal".into()]).unwrap();
    std::fs::write(dir.join("tls.crt"), pair.cert.pem()).unwrap();
    std::fs::write(dir.join("tls.key"), pair.signing_key.serialize_pem()).unwrap();
    pair
}

/// 等到两封信都投进各自的私聊并从 Mailpit 删掉，再核对收件人和主题。
async fn assert_delivered(home: &Home, api: &Api) {
    let delivered = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if home.matrix.state().sent.len() == 3 && api.list().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if delivered.is_err() {
        let left = api.list().await.unwrap();
        panic!(
            "45 秒没投完：已发 {} 条，Mailpit 还剩 {}",
            home.matrix.state().sent.len(),
            left.len()
        );
    }
    let state = home.matrix.state();
    let mut recipients = Vec::new();
    let mut subjects = Vec::new();
    for (room, content) in &state.sent {
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content[MAIL_FIELD]["plugin_id"], PLUGIN);
        subjects.push(content[MAIL_FIELD]["subject"].as_str().unwrap().to_owned());
        recipients.extend(
            state.rooms[room]
                .members
                .iter()
                .filter(|user| user.as_str() != "@wonton-core:test")
                .cloned(),
        );
    }
    // 明文那封发给 alice 和 owner，STARTTLS 那封发给 owner。
    recipients.sort();
    assert_eq!(recipients, ["@alice:test", "@owner:test", "@owner:test"]);
    subjects.sort();
    assert_eq!(
        subjects,
        [
            "SMTP integration",
            "SMTP integration",
            "STARTTLS integration"
        ]
    );
}

async fn create_mailpit(engine: &Engine, name: &str, image: &str, dir: &std::path::Path) {
    let auth = dir.join("auth.htpasswd");
    std::fs::write(&auth, format!("core:{PASSWORD}\n")).unwrap();
    let ports = ["1025/tcp", "8025/tcp"];
    engine
        .create_container(
            name,
            ContainerCreateBody {
                image: Some(image.to_owned()),
                env: Some(vec![
                    "MP_SMTP_BIND_ADDR=0.0.0.0:1025".into(),
                    "MP_UI_BIND_ADDR=0.0.0.0:8025".into(),
                    "MP_UI_AUTH_FILE=/auth.htpasswd".into(),
                    "MP_SMTP_ALLOWED_RECIPIENTS=^(owner|alice)@wonton\\.internal$".into(),
                    "MP_MAX_MESSAGE_SIZE=25".into(),
                    "MP_MAX_AGE=7d".into(),
                    "MP_DISABLE_VERSION_CHECK=true".into(),
                    "MP_SMTP_AUTH_ACCEPT_ANY=true".into(),
                    "MP_SMTP_AUTH_ALLOW_INSECURE=true".into(),
                    "MP_SMTP_TLS_CERT=/tls.crt".into(),
                    "MP_SMTP_TLS_KEY=/tls.key".into(),
                    "MP_VERBOSE=true".into(),
                ]),
                exposed_ports: Some(ports.map(str::to_owned).into()),
                host_config: Some(HostConfig {
                    binds: Some(vec![
                        format!("{}:/auth.htpasswd:ro", auth.display()),
                        format!("{}:/tls.crt:ro", dir.join("tls.crt").display()),
                        format!("{}:/tls.key:ro", dir.join("tls.key").display()),
                    ]),
                    port_bindings: Some(
                        ports
                            .into_iter()
                            .map(|port| {
                                (
                                    port.into(),
                                    Some(vec![PortBinding {
                                        host_ip: Some("127.0.0.1".into()),
                                        host_port: Some(String::new()),
                                    }]),
                                )
                            })
                            .collect(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}
async fn smtp(port: u16) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut stream = BufReader::new(stream);
    response(&mut stream, 220).await;
    ehlo(&mut stream, true).await;
    // Apps that insist on credentials get AUTH on the plaintext connection; any pair is accepted.
    for (command, code) in [
        ("AUTH LOGIN\r\n", 334),
        ("dXNlcg==\r\n", 334),
        ("cGFzcw==\r\n", 235),
    ] {
        stream
            .get_mut()
            .write_all(command.as_bytes())
            .await
            .unwrap();
        response(&mut stream, code).await;
    }
    for (command, code) in [
        ("MAIL FROM:<arbitrary@example.com>\r\n", 250),
        ("RCPT TO:<outside@example.com>\r\n", 550),
        ("RCPT TO:<owner@wonton.internal>\r\n", 250),
        ("RCPT TO:<alice@wonton.internal>\r\n", 250),
        ("DATA\r\n", 354),
        (
            "From: arbitrary@example.com\r\nTo: alice@wonton.internal\r\nSubject: SMTP integration\r\n\r\nReset at https://example.com/reset\r\n.\r\n",
            250,
        ),
        ("QUIT\r\n", 221),
    ] {
        stream
            .get_mut()
            .write_all(command.as_bytes())
            .await
            .unwrap();
        response(&mut stream, code).await;
    }
}

/// 要求加密发信的应用（如用 Nodemailer 的）：STARTTLS 升级后照常投递，证书是核心自签的那张。
async fn smtp_over_starttls(port: u16, certificate: Vec<u8>) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    // 容量 1：升级 TLS 前不能有没读完的缓冲。
    let mut stream = BufReader::with_capacity(1, stream);
    response(&mut stream, 220).await;
    ehlo(&mut stream, true).await;
    stream.get_mut().write_all(b"STARTTLS\r\n").await.unwrap();
    response(&mut stream, 220).await;

    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(certificate)).unwrap();
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from("smtp.wonton.internal".to_owned()).unwrap();
    let tls = TlsConnector::from(Arc::new(config))
        .connect(name, stream.into_inner())
        .await
        .unwrap();
    let mut tls = BufReader::with_capacity(1, tls);
    ehlo(&mut tls, false).await;

    for (command, code) in [
        (
            "MAIL FROM:<app.example.monitor@plugins.wonton.internal>\r\n",
            250,
        ),
        ("RCPT TO:<owner@wonton.internal>\r\n", 250),
        ("DATA\r\n", 354),
        (
            "Subject: STARTTLS integration\r\n\r\nA reset link\r\n.\r\n",
            250,
        ),
        ("QUIT\r\n", 221),
    ] {
        tls.get_mut().write_all(command.as_bytes()).await.unwrap();
        response(&mut tls, code).await;
    }
}

async fn ehlo<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut BufReader<S>, plaintext: bool) {
    stream.get_mut().write_all(b"EHLO app\r\n").await.unwrap();
    let mut capabilities = String::new();
    let mut raw = Vec::new();
    loop {
        let mut line = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_until(b'\n', &mut line))
            .await
            .unwrap()
            .unwrap();
        raw.extend_from_slice(&line);
        let line = String::from_utf8_lossy(&line);
        let continued = line.starts_with("250-");
        capabilities.push_str(&line);
        if !continued {
            break;
        }
    }
    let capabilities = capabilities.to_ascii_lowercase();
    assert!(capabilities.contains("auth"), "{capabilities}");
    // 配了自签证书：明文连接宣告 STARTTLS，升级过的连接不再宣告。
    assert_eq!(
        capabilities.contains("starttls"),
        plaintext,
        "{capabilities} | raw={raw:?}"
    );
}

async fn response<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut BufReader<S>, expected: u16) {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.get(..3), Some(expected.to_string().as_str()), "{line}");
}
