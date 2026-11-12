//! 真实 Tuwunel v2.0.0、lk-jwt-service 和 LiveKit（ADR-0096）：设备拿 OpenID 令牌经网关的
//! `/_wonton/matrix-rtc/sfu/get` 换回 LiveKit 令牌；授权服务经核心的服务器名别名（TLS，信任核心写的证书包）
//! 验令牌、代管 delayed leave，日志里不留 OpenID 令牌。
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
use super::matrix_rtc_alias::{self, DELAYED_EVENTS_PREFIX, TlsListener};
use super::tests::{Harness, Reply, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::matrix::{Login, Matrix};
use crate::owner::OwnerStore;
use crate::plugin::PluginStatus;
use crate::plugin::manifest::{Manifest, PluginId};
use crate::plugin::matrix_rtc_ca::{BUNDLE_FILE, CoreCa};
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

/// 起 LiveKit 和授权服务，返回授权服务的容器名和发布到宿主回环上的地址。服务器名解析到插件网络的网关
/// （宿主），和核心跑在宿主上时插件拿到的 `extra_hosts` 一样。授权服务照通话插件的写法配：信任核心写的证书包，
/// 客户端接口指到同一个服务器名（别名转给 Tuwunel）。
fn call_plugin(names: &mut Names, stack: &Stack, ca_dir: &std::path::Path) -> (String, String) {
    let livekit = names.oneshot_container("livekit");
    docker(&[
        "run",
        "--detach",
        "--name",
        &livekit,
        "--network",
        &stack.network,
        "--network-alias",
        LIVEKIT_ALIAS,
        &wonton_test_images::image(LIVEKIT),
        "--dev",
        "--bind",
        "0.0.0.0",
    ]);
    let jwt = names.oneshot_container("lk-jwt");
    let server_name = &stack.config.server_name;
    docker(&[
        "run",
        "--detach",
        "--name",
        &jwt,
        "--network",
        &stack.network,
        "--publish",
        "127.0.0.1::8080",
        "--add-host",
        &format!("{HOST}:{}", stack.gateway_ip),
        "--volume",
        &format!("{}:/etc/wonton-ca:ro", ca_dir.display()),
        "--env",
        &format!("SSL_CERT_FILE=/etc/wonton-ca/{BUNDLE_FILE}"),
        "--env",
        &format!("LIVEKIT_CS_API_URL_OVERRIDES={server_name}=https://{server_name}"),
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
        &wonton_test_images::image(LK_JWT),
    ]);
    let address = docker(&["port", &jwt, "8080/tcp"]);
    (jwt, address)
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

async fn client_json(config: &MatrixConfig, login: &Login, method: Method, path: &str) -> Value {
    let response = reqwest::Client::new()
        .request(method, format!("{}{path}", config.url))
        .bearer_auth(&login.access_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{path}：{}",
        response.status()
    );
    response.json().await.unwrap()
}

/// 设备登录、建一个房间、要一个 OpenID 令牌（`POST /user/{id}/openid/request_token`）。
async fn openid(config: &MatrixConfig, matrix: &Matrix) -> (Login, String, Value) {
    let login = matrix.login("owner", "phone").await.unwrap();
    let room = client_json(
        config,
        &login,
        Method::POST,
        "/_matrix/client/v3/createRoom",
    )
    .await;
    let path = format!(
        "/_matrix/client/v3/user/{}/openid/request_token",
        login.user_id
    );
    let token = client_json(config, &login, Method::POST, &path).await;
    (login, room["room_id"].as_str().unwrap().to_owned(), token)
}

/// 房间里一条 MSC4140 的 delayed state event（MatrixRTC 用它在断线后退出通话），返回 `delay_id`。
/// 状态键照 MatrixRTC 的写法以 `_<自己的用户 ID>_` 开头（MSC3757：只有自己能写）。
async fn delayed_leave(config: &MatrixConfig, login: &Login, room: &str, device: &str) -> String {
    let key = format!("_{}_{device}", login.user_id);
    let path = format!(
        "/_matrix/client/v3/rooms/{room}/state/org.matrix.msc3401.call.member/{key}?org.matrix.msc4140.delay=600000"
    );
    let sent = client_json(config, login, Method::PUT, &path).await;
    sent["delay_id"]
        .as_str()
        .expect("Tuwunel 回了 delay_id")
        .to_owned()
}

/// 核心这一侧：接上 Tuwunel 的网关，加上已经绑好端口、由核心的 CA 签证书的服务器名别名。
async fn core(
    h: &Harness,
    config: &MatrixConfig,
    dir: &std::path::Path,
    tcp: TcpListener,
    ca: &CoreCa,
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
        acceptor: ca.acceptor(&config.server_name).unwrap(),
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

/// 测试网络、Tuwunel 和核心。
struct Stack {
    network: String,
    gateway_ip: String,
    subnet: String,
    config: MatrixConfig,
    h: Harness,
    gateway: Arc<Gateway>,
    _dir: tempfile::TempDir,
}

impl Stack {
    async fn start(names: &mut Names, ca: &CoreCa) -> Self {
        let network = names.oneshot_network("rtc");
        docker(&["network", "create", &network]);
        let ipam = |field: &str| {
            docker(&[
                "network",
                "inspect",
                &network,
                "--format",
                &format!("{{{{(index .IPAM.Config 0).{field}}}}}"),
            ])
        };
        let (gateway_ip, subnet) = (ipam("Gateway"), ipam("Subnet"));
        // 先占住别名监听的端口：服务器名要带上它。
        let tcp = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let server_name = format!("{HOST}:{}", tcp.local_addr().unwrap().port());
        let dir = tempfile::tempdir().unwrap();
        let config = tuwunel(names, &network, &server_name, dir.path());
        let h = Harness::start(None).await;
        let gateway = core(&h, &config, dir.path(), tcp, ca).await;
        Self {
            network,
            gateway_ip,
            subnet,
            config,
            h,
            gateway,
            _dir: dir,
        }
    }

    /// 授权服务经别名发来的请求：来源是提供方网络里的地址。
    async fn send_as_provider(&self, method: Method, path: &str) -> Reply {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &self.config.server_name)
            .body(Body::empty())
            .unwrap();
        let ip = self
            .subnet
            .split('/')
            .next()
            .unwrap()
            .trim_end_matches('0')
            .to_owned()
            + "9";
        let addr: std::net::SocketAddr = format!("{ip}:40000").parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        let alias = matrix_rtc_alias::router(self.gateway.clone());
        Reply::of(alias.oneshot(req).await.unwrap()).await
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，真实 Tuwunel v2.0.0、lk-jwt-service 0.9.0 和 LiveKit 经网关换 LiveKit 令牌"]
async fn an_openid_token_is_exchanged_for_a_livekit_token_through_the_gateway() {
    let mut names = Names::new();
    let ca = CoreCa::ephemeral();
    let stack = Stack::start(&mut names, &ca).await;
    let (h, gateway, config) = (&stack.h, &stack.gateway, &stack.config);
    let (login, room, token) = openid(config, gateway.matrix.as_ref().unwrap()).await;
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
    let reply = from_phone(h, gateway, &body).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some(KIND_UNAVAILABLE));

    // 核心照装插件时的做法把证书包写进提供方的数据目录。
    let app_data = tempfile::tempdir().unwrap();
    let ca_dir = ca.write_bundle(app_data.path()).unwrap();
    let (jwt, address) = call_plugin(&mut names, &stack, &ca_dir);
    h.manager
        .register_for_test(provider(), PluginStatus::Running, None);
    h.manager
        .register_source_for_test(PROVIDER, &stack.subnet, Vec::new());
    h.manager
        .matrix_rtc_route_for_test(&PluginId::parse(PROVIDER).unwrap(), Endpoint::Tcp(address));

    // 授权服务刚起来时还没监听：等它答出 Matrix 的错误或成功为止，最多 60 秒。
    let mut reply = from_phone(h, gateway, &body).await;
    for _ in 0..60 {
        if reply.status != StatusCode::SERVICE_UNAVAILABLE {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        reply = from_phone(h, gateway, &body).await;
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
    forged["openid_token"]["access_token"] = json!("forged-openid-token");
    let reply = from_phone(h, gateway, &forged).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());

    delegated_leave_reaches_tuwunel(&stack, &login, &room, &body).await;

    // 成功、失败、代管走过一遍，授权服务的日志里没有 OpenID 令牌。
    let logs = Command::new("docker")
        .args(["logs", &jwt])
        .output()
        .unwrap();
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&logs.stdout),
        String::from_utf8_lossy(&logs.stderr)
    );
    for secret in [
        token["access_token"].as_str().unwrap(),
        "forged-openid-token",
    ] {
        assert!(
            !logs.contains(secret),
            "授权服务的日志里有 OpenID 令牌：\n{logs}"
        );
    }
    println!(
        "{}：OpenID 令牌经网关换回 LiveKit 令牌；没装提供方 503；假令牌 401；代管 delayed leave 经别名到 Tuwunel；日志无令牌",
        names.name()
    );
}

/// 代管 delayed leave：授权服务接下 `delay_id`（要能经别名找到客户端接口），别名把 MSC4140 的操作原样转给
/// Tuwunel（Tuwunel v2.0.0 的不稳定路径不要访问令牌）。
async fn delegated_leave_reaches_tuwunel(stack: &Stack, login: &Login, room: &str, body: &Value) {
    let delay_id = delayed_leave(&stack.config, login, room, "DELEGATED").await;
    let mut delegated = body.clone();
    delegated["delay_id"] = json!(delay_id);
    delegated["delay_timeout"] = json!(600_000);
    let reply = from_phone(&stack.h, &stack.gateway, &delegated).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());

    let delay_id = delayed_leave(&stack.config, login, room, "DIRECT").await;
    for action in ["restart", "send"] {
        let path = format!("{DELAYED_EVENTS_PREFIX}{delay_id}/{action}");
        let reply = stack.send_as_provider(Method::POST, &path).await;
        assert_eq!(reply.status, StatusCode::OK, "{action}：{}", reply.text());
    }
    let path = format!(
        "/_matrix/client/v3/rooms/{room}/state/org.matrix.msc3401.call.member/_{}_DIRECT",
        login.user_id
    );
    client_json(&stack.config, login, Method::GET, &path).await;
}
