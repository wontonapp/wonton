//! 真实 Tuwunel v2.0.0、lk-jwt-service 和 LiveKit（ADR-0096）：设备拿 OpenID 令牌经网关的
//! `/_wonton/matrix-rtc/sfu/get` 换回 LiveKit 令牌；授权服务经核心的服务器名别名（TLS）验令牌。
//!
//! 核心跑在宿主上（测试进程），别名监听开不了 443：Tuwunel 的服务器名带上监听的端口，授权服务按 Matrix 的规则
//! 直接连这个端口，不走 `.well-known`（那条路径由单元测试和 ADR 里的实测覆盖）。

use std::process::Command;
use std::sync::Arc;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use axum::body::Body;
use axum::extract::Request;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hyper::{Method, StatusCode, header};
use iroh::SecretKey;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::matrix_rtc::KIND_UNAVAILABLE;
use super::matrix_rtc_alias::{self, TlsListener};
use super::tests::{Harness, Reply, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::owner::OwnerStore;
use crate::plugin::PluginStatus;
use crate::plugin::manifest::{Manifest, PluginId};
use crate::plugin::upstreams::Endpoint;
use crate::test_docker::Names;

const TUWUNEL: &str = "ghcr.io/matrix-construct/tuwunel:v2.0.0@sha256:3dde1e9b3503c68218f95415ec3ebff522bd945065ba76019d408543540d1730";
const LIVEKIT: &str = "docker.io/livekit/livekit-server:v1.13.11@sha256:25cd902de7cacbd5f1f779f6a6ac1eae20ba8f56b0935934ce6b0ab80d1e3d20";
const LK_JWT: &str = "ghcr.io/element-hq/lk-jwt-service:0.9.0@sha256:d210fd778ba38b7e42e6fc64e8a24a7d66ccc3b938fb20d73bd713224af2dfea";
const CONFIG: &str = include_str!("../../../../deploy/dev/matrix/tuwunel.toml");
const PROVIDER: &str = "app.call";
const PEM: &str = "PRIVATE KEY";
const HOST: &str = "rtc.test";
/// LiveKit 在测试网络里的别名。地址用 `format!` 拼：`ws://livekit:7880` 这样的字面量会被镜像清单检查当成镜像引用。
const LIVEKIT_ALIAS: &str = "livekit";

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

fn provider() -> Manifest {
    serde_json::from_value(json!({
        "spec": "0.13",
        "id": PROVIDER,
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Call"},
        "description": {"en": "Calls"},
        "permissions": [],
        "provides": {"matrix_rtc": {"service": "jwt", "port": 8080}}
    }))
    .unwrap()
}

/// 起 Tuwunel，返回连它的配置（地址是发布到宿主回环上的端口）。
fn tuwunel(
    names: &mut Names,
    network: &str,
    server_name: &str,
    dir: &std::path::Path,
) -> MatrixConfig {
    let path = dir.join("tuwunel.toml");
    std::fs::write(&path, CONFIG).unwrap();
    let pair = Ed25519KeyPair::generate().unwrap();
    let private = STANDARD.encode(pair.to_pkcs8().unwrap().as_ref());
    let public = STANDARD.encode(pair.public_key().as_der().unwrap().as_ref());
    let config = MatrixConfig {
        url: String::new(),
        server_name: server_name.to_owned(),
        // PEM 的标签拼进去：整段字面量会被密钥扫描当成私钥（它一路匹配到后面的 `LIVEKIT_KEY`）。
        jwt_key: format!("-----BEGIN {PEM}-----\n{private}\n-----END {PEM}-----\n"),
        registration_secret: crate::secret::random_token(),
    };
    let name = names.oneshot_container("tuwunel");
    docker(&[
        "run",
        "--detach",
        "--name",
        &name,
        "--network",
        network,
        "--publish",
        "127.0.0.1::8008",
        "--volume",
        &format!("{}:/etc/tuwunel/tuwunel.toml:ro", path.display()),
        "--env",
        "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml",
        "--env",
        &format!("TUWUNEL_SERVER_NAME={server_name}"),
        "--env",
        &format!(
            "TUWUNEL_JWT__KEY=-----BEGIN PUBLIC KEY-----\n{public}\n-----END PUBLIC KEY-----\n"
        ),
        "--env",
        &format!(
            "TUWUNEL_REGISTRATION_SHARED_SECRET={}",
            config.registration_secret
        ),
        &wonton_test_images::image(TUWUNEL),
    ]);
    let address = docker(&["port", &name, "8008/tcp"]);
    MatrixConfig {
        url: format!("http://{address}"),
        ..config
    }
}

/// 起 LiveKit 和授权服务，返回授权服务发布到宿主回环上的地址。服务器名解析到插件网络的网关（宿主），
/// 和核心跑在宿主上时插件拿到的 `extra_hosts` 一样。
fn call_plugin(names: &mut Names, network: &str, server_name: &str, gateway: &str) -> String {
    let livekit = names.oneshot_container("livekit");
    docker(&[
        "run",
        "--detach",
        "--name",
        &livekit,
        "--network",
        network,
        "--network-alias",
        LIVEKIT_ALIAS,
        &wonton_test_images::image(LIVEKIT),
        "--dev",
        "--bind",
        "0.0.0.0",
    ]);
    let jwt = names.oneshot_container("lk-jwt");
    docker(&[
        "run",
        "--detach",
        "--name",
        &jwt,
        "--network",
        network,
        "--publish",
        "127.0.0.1::8080",
        "--add-host",
        &format!("{HOST}:{gateway}"),
        "--env",
        &format!("LIVEKIT_URL=ws://{LIVEKIT_ALIAS}:7880"),
        "--env",
        "LIVEKIT_KEY=devkey",
        "--env",
        "LIVEKIT_SECRET=secret",
        "--env",
        "LIVEKIT_JWT_BIND=:8080",
        "--env",
        &format!("LIVEKIT_FULL_ACCESS_HOMESERVERS={server_name}"),
        // 实测选定的信任方式，见 ADR-0096 的“验证结果”。
        "--env",
        "LIVEKIT_INSECURE_SKIP_VERIFY_TLS=YES_I_KNOW_WHAT_I_AM_DOING",
        &wonton_test_images::image(LK_JWT),
    ]);
    docker(&["port", &jwt, "8080/tcp"])
}

async fn from_phone(h: &Harness, gateway: &Arc<Gateway>, body: &Value) -> Reply {
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/_wonton/matrix-rtc/sfu/get")
        .header(header::HOST, "matrix.wonton.localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    req.extensions_mut().insert(TunnelPeer(phone));
    Reply::of(router(gateway.clone()).oneshot(req).await.unwrap()).await
}

/// 设备的 OpenID 令牌（`POST /user/{id}/openid/request_token`）和一个它在里面的房间。
async fn openid(config: &MatrixConfig, matrix: &Matrix) -> (Value, String) {
    let login = matrix.login("owner", "phone").await.unwrap();
    let http = reqwest::Client::new();
    let room: Value = http
        .post(format!("{}/_matrix/client/v3/createRoom", config.url))
        .bearer_auth(&login.access_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token: Value = http
        .post(format!(
            "{}/_matrix/client/v3/user/{}/openid/request_token",
            config.url, login.user_id
        ))
        .bearer_auth(&login.access_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (token, room["room_id"].as_str().unwrap().to_owned())
}

/// 核心这一侧：接上 Tuwunel 的网关，加上已经绑好端口的服务器名别名。
async fn core(
    h: &Harness,
    config: &MatrixConfig,
    dir: &std::path::Path,
    tcp: TcpListener,
) -> Arc<Gateway> {
    let matrix = Matrix::connect(config, &dir.join("matrix"), HttpClient::new())
        .await
        .unwrap();
    let mut gateway = Gateway::new(
        crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: super::tests::CORE.into(),
            port: h.addr.port(),
        },
        OwnerStore::for_test(&h.data_dir.join("rtc-owner"), &owner()),
        TOKEN.into(),
        h.members.clone(),
        h.manager.clone(),
        None,
        h.http.clone(),
        h.tunnel.clone(),
    );
    gateway.matrix = Some(Arc::new(matrix));
    let gateway = Arc::new(gateway);
    let listener = TlsListener {
        tcp,
        acceptor: matrix_rtc_alias::acceptor(&config.server_name).unwrap(),
    };
    let alias = matrix_rtc_alias::router(gateway.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            alias.into_make_service_with_connect_info::<matrix_rtc_alias::AliasPeer>(),
        )
        .await
        .unwrap();
    });
    gateway
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，真实 Tuwunel v2.0.0、lk-jwt-service 0.9.0 和 LiveKit 经网关换 LiveKit 令牌"]
async fn an_openid_token_is_exchanged_for_a_livekit_token_through_the_gateway() {
    let mut names = Names::new();
    let network = names.oneshot_network("rtc");
    docker(&["network", "create", &network]);
    let gateway_ip = docker(&[
        "network",
        "inspect",
        &network,
        "--format",
        "{{(index .IPAM.Config 0).Gateway}}",
    ]);
    let subnet = docker(&[
        "network",
        "inspect",
        &network,
        "--format",
        "{{(index .IPAM.Config 0).Subnet}}",
    ]);
    // 先占住别名监听的端口：服务器名要带上它。
    let tcp = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let server_name = format!("{HOST}:{}", tcp.local_addr().unwrap().port());

    let dir = tempfile::tempdir().unwrap();
    let config = tuwunel(&mut names, &network, &server_name, dir.path());
    let h = Harness::start(None).await;
    let gateway = core(&h, &config, dir.path(), tcp).await;

    let (token, room) = openid(&config, gateway.matrix.as_ref().unwrap()).await;
    let body = json!({"room": room, "device_id": "PHONE", "openid_token": token});

    // 开发栈和正式安装的基础配置都关着联邦（`allow_federation = false`），OpenID 用户信息照样要答。
    let userinfo = reqwest::Client::new()
        .get(format!("{}{}", config.url, matrix_rtc_alias::USERINFO_PATH))
        .query(&[("access_token", token["access_token"].as_str().unwrap())])
        .send()
        .await
        .unwrap();
    assert_eq!(userinfo.status(), reqwest::StatusCode::OK);

    // 没装提供方：如实 503。
    let reply = from_phone(&h, &gateway, &body).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some(KIND_UNAVAILABLE));

    let jwt = call_plugin(&mut names, &network, &server_name, &gateway_ip);
    h.manager
        .register_for_test(provider(), PluginStatus::Running, None);
    h.manager
        .register_source_for_test(PROVIDER, &subnet, Vec::new());
    h.manager
        .matrix_rtc_route_for_test(&PluginId::parse(PROVIDER).unwrap(), Endpoint::Tcp(jwt));

    // 授权服务刚起来时还没监听：等它答出 Matrix 的错误或成功为止，最多 60 秒。
    let mut reply = from_phone(&h, &gateway, &body).await;
    for _ in 0..60 {
        if reply.status != StatusCode::SERVICE_UNAVAILABLE {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        reply = from_phone(&h, &gateway, &body).await;
    }
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let answer = reply.json();
    assert_eq!(answer["url"], format!("ws://{LIVEKIT_ALIAS}:7880"));
    assert!(
        answer["jwt"]
            .as_str()
            .is_some_and(|jwt| jwt.split('.').count() == 3),
        "{answer}"
    );

    // 令牌是假的：授权服务经别名问 Tuwunel，被拒。
    let mut forged = body.clone();
    forged["openid_token"]["access_token"] = json!("forged");
    let reply = from_phone(&h, &gateway, &forged).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
    println!(
        "{}：OpenID 令牌经网关换回 LiveKit 令牌；没装提供方 503；假令牌 401",
        names.name()
    );
}
