//! 两台真实 Tuwunel v2.0.0 走真联邦：批准一个外人只放行这一个人，另一个外人照样被挡；同一台服务器上的人
//! 一直不受限。`blocked_servers` 的 `*` 只由上游单元测试背书，这里让它的跨服务器行为在真联邦上过一遍。
//!
//! TLS 由 haproxy 终止（临时 CA 签的叶证书，两台都把它当根），Tuwunel 本身还是明文 8008；联邦按服务器名的
//! 默认端口 8448 访问。半台服务器上发布到宿主回环的端口只给测试客户端用。

use std::process::Command;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;

use super::{Matrix, MatrixError};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::Names;
use wonton_protocol::Gate;

const TUWUNEL: &str = "ghcr.io/matrix-construct/tuwunel:v2.0.0@sha256:3dde1e9b3503c68218f95415ec3ebff522bd945065ba76019d408543540d1730";
const PROXY: &str =
    "haproxy:3.4-alpine@sha256:8aa6458a9d22a8364a2d2ee9ce93de0cc4a2789e3cf63f0b44adc972863724d7";
const CONFIG: &str = include_str!("../../../../deploy/dev/matrix/tuwunel.toml");

fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("运行 Docker");
    assert!(
        output.status.success(),
        "docker {args:?}：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// 临时 CA 和两张叶证书（`a.test`、`b.test` 各一张，也是各自 TLS 代理的证书）。
fn certificates(dir: &std::path::Path) -> std::path::PathBuf {
    let tls = dir.join("tls");
    std::fs::create_dir_all(&tls).unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    std::fs::write(tls.join("ca.pem"), ca.pem()).unwrap();
    for name in ["a.test", "b.test"] {
        let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let key = rcgen::KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
        let leaf = params.signed_by(&key, &issuer).unwrap();
        std::fs::write(
            tls.join(format!("{name}.pem")),
            format!("{}{}", leaf.pem(), key.serialize_pem()),
        )
        .unwrap();
    }
    tls
}

/// 起一台能联邦的 Tuwunel 和它前面的 TLS 代理；返回客户端要用的配置。
fn server(
    names: &mut Names,
    dir: &std::path::Path,
    network: &str,
    name: &str,
    tls: &std::path::Path,
) -> MatrixConfig {
    let path = dir.join(format!("{name}.toml"));
    std::fs::write(&path, CONFIG).unwrap();
    let pair = Ed25519KeyPair::generate().unwrap();
    let private = STANDARD.encode(pair.to_pkcs8().unwrap().as_ref());
    let public = STANDARD.encode(pair.public_key().as_der().unwrap().as_ref());
    let secret = crate::secret::random_token();
    let container = names.oneshot_container(&format!("tuwunel-{}", name.replace('.', "-")));
    docker(&[
        "run",
        "--detach",
        "--name",
        &container,
        "--label",
        &format!("{LABEL_INSTANCE}={}", names.name()),
        "--network",
        network,
        "--publish",
        "127.0.0.1::8008",
        "--volume",
        &format!("{}:/etc/tuwunel/tuwunel.toml:ro", path.display()),
        "--volume",
        &format!("{}:/tls:ro", tls.display()),
        "--env",
        "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml",
        "--env",
        &format!("TUWUNEL_SERVER_NAME={name}"),
        "--env",
        "TUWUNEL_ALLOW_FEDERATION=true",
        "--env",
        "SSL_CERT_FILE=/tls/ca.pem",
        "--env",
        &format!(
            "TUWUNEL_JWT__KEY=-----BEGIN PUBLIC KEY-----\n{public}\n-----END PUBLIC KEY-----\n"
        ),
        "--env",
        &format!("TUWUNEL_REGISTRATION_SHARED_SECRET={secret}"),
        &wonton_test_images::image(TUWUNEL),
    ]);
    let address = docker(&["port", &container, "8008/tcp"]);
    let proxy = names.oneshot_container(&format!("tls-{}", name.replace('.', "-")));
    let config = dir.join(format!("{name}.haproxy"));
    std::fs::write(
        &config,
        format!(
            "defaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\n\
             frontend fed\n  bind :8448 ssl crt /tls/{name}.pem\n  default_backend tuwunel\n\
             backend tuwunel\n  server tuwunel {container}:8008\n"
        ),
    )
    .unwrap();
    docker(&[
        "run",
        "--detach",
        "--name",
        &proxy,
        "--label",
        &format!("{LABEL_INSTANCE}={}", names.name()),
        "--entrypoint",
        "haproxy",
        "--network",
        network,
        "--network-alias",
        name,
        "--volume",
        &format!("{}:/tls:ro", tls.display()),
        "--volume",
        &format!("{}:/etc/tls/haproxy.cfg:ro", config.display()),
        &wonton_test_images::image(PROXY),
        "-W",
        "-db",
        "-f",
        "/etc/tls/haproxy.cfg",
    ]);
    MatrixConfig {
        url: format!("http://{address}"),
        server_name: name.into(),
        jwt_key: format!("-----BEGIN PRIVATE KEY-----\n{private}\n-----END PRIVATE KEY-----\n"),
        registration_secret: secret,
    }
}

async fn wait_ready(url: &str) {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if client
            .get(format!("{url}/_matrix/client/versions"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "等不到 {url}");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

fn banned(error: &MatrixError) -> bool {
    matches!(
        error,
        MatrixError::Rejected {
            status: 403,
            errcode,
            ..
        } if errcode.ends_with("M_INVITE_BLOCKED")
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，起两台 Tuwunel 走真联邦验证允许名单"]
async fn approving_one_outsider_lets_only_that_person_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut names = Names::new();
    let network = names.oneshot_network("federation");
    docker(&["network", "create", &network]);
    let tls = certificates(dir.path());
    let a = server(&mut names, dir.path(), &network, "a.test", &tls);
    let b = server(&mut names, dir.path(), &network, "b.test", &tls);
    // 先等两台都起来再连：容器慢时 connect 自己的版本查询会被取消，裸 unwrap 直接 panic。
    wait_ready(&a.url).await;
    wait_ready(&b.url).await;
    let matrix_a = std::sync::Arc::new(
        Matrix::connect(&a, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let matrix_b = std::sync::Arc::new(
        Matrix::connect(&b, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );

    let bob = matrix_a.login("bob", "bob phone").await.unwrap();
    let carol = matrix_a.login("carol", "carol phone").await.unwrap();
    let kid = matrix_b.login("kid", "kid phone").await.unwrap();
    let mia = matrix_b.login("mia", "mia phone").await.unwrap();
    matrix_b
        .ensure_invite_filter("kid", Gate::AskAdmin)
        .await
        .unwrap();

    // 家外的邀请在联邦上就被挡住；同一台服务器上的人照常（`allowed_servers`）。
    let bob_room = matrix_a
        .session(&bob)
        .create_direct_room(&kid.user_id)
        .await
        .unwrap();
    let error = matrix_a
        .session(&bob)
        .invite(&bob_room, &kid.user_id)
        .await
        .unwrap_err();
    assert!(banned(&error), "家外邀请应被挡住：{error:?}");
    let mia_room = matrix_b
        .session(&mia)
        .create_direct_room(&kid.user_id)
        .await
        .unwrap();
    matrix_b
        .session(&mia)
        .invite(&mia_room, &kid.user_id)
        .await
        .unwrap();

    // 只批准 bob：名单里恰好是他。
    matrix_b
        .allow_invitees(
            "kid",
            &[super::invite_filter::ExactUserId::parse(&bob.user_id).unwrap()],
        )
        .await
        .unwrap();
    let stored = matrix_b.global_account_data(&kid.user_id).await.unwrap();
    assert_eq!(
        stored[super::invite_filter::INVITE_FILTER_TYPE]["allowed_users"],
        json!([bob.user_id.to_string()])
    );

    // bob 重发就进得去；carol 还是被挡。
    let room = matrix_a
        .session(&bob)
        .create_direct_room(&kid.user_id)
        .await
        .unwrap();
    matrix_a
        .session(&bob)
        .invite(&room, &kid.user_id)
        .await
        .unwrap();
    let carol_room = matrix_a
        .session(&carol)
        .create_direct_room(&kid.user_id)
        .await
        .unwrap();
    let error = matrix_a
        .session(&carol)
        .invite(&carol_room, &kid.user_id)
        .await
        .unwrap_err();
    assert!(banned(&error), "没获批的外人还是被挡：{error:?}");
}
