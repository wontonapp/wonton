use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use wonton_protocol::MAIL_FIELD;

use super::api::Api;
use super::test_support::{PASSWORD, PLUGIN, home};
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
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if home.matrix.state().sent.len() == 2 && api.list().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    {
        let state = home.matrix.state();
        let mut recipients = Vec::new();
        for (room, content) in &state.sent {
            assert_eq!(content["msgtype"], "m.text");
            assert_eq!(content[MAIL_FIELD]["plugin_id"], PLUGIN);
            assert_eq!(content[MAIL_FIELD]["subject"], "SMTP integration");
            recipients.extend(
                state.rooms[room]
                    .members
                    .iter()
                    .filter(|user| user.as_str() != "@wonton-core:test")
                    .cloned(),
            );
        }
        recipients.sort();
        assert_eq!(recipients, ["@alice:test", "@owner:test"]);
    }
    drop(worker);
    engine.remove_container(&name).await.unwrap();
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
                ]),
                exposed_ports: Some(ports.map(str::to_owned).into()),
                host_config: Some(HostConfig {
                    binds: Some(vec![format!("{}:/auth.htpasswd:ro", auth.display())]),
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
    ehlo(&mut stream).await;
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

async fn ehlo(stream: &mut BufReader<TcpStream>) {
    stream.get_mut().write_all(b"EHLO app\r\n").await.unwrap();
    let mut capabilities = String::new();
    loop {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let continued = line.starts_with("250-");
        capabilities.push_str(&line);
        if !continued {
            break;
        }
    }
    let capabilities = capabilities.to_ascii_lowercase();
    assert!(capabilities.contains("auth"), "{capabilities}");
    assert!(!capabilities.contains("starttls"), "{capabilities}");
}

async fn response(stream: &mut BufReader<TcpStream>, expected: u16) {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.get(..3), Some(expected.to_string().as_str()), "{line}");
}
