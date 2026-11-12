//! 测试替身：假插件上游（echo）、假 Hydra 管理接口、只走回环地址的 iroh endpoint 和隧道。
//! 只存在于测试里（AGENTS.md：测试替身不出现在生产路径）。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{RawQuery, Request};
use axum::response::Response;
use axum::routing::get;
use hyper::StatusCode;
use hyper::header;
use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wonton_tunnel::relays::RelayTable;

use crate::device::Devices;
use crate::members::Members;
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

pub(super) async fn serve(listener: TcpListener, router: Router) {
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
}

pub(super) async fn bind() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

/// 假身份提供方的发现文档：和 Hydra v26.2.0 实测的一样混着两种主机名，端口留给网关改写。
async fn discovery() -> Json<Value> {
    Json(json!({
        "issuer": "http://auth.wonton.internal",
        "authorization_endpoint": "http://auth.wonton.localhost/oauth2/auth",
        "token_endpoint": "http://auth.wonton.localhost/oauth2/token",
        "userinfo_endpoint": "http://auth.wonton.localhost/userinfo",
        "jwks_uri": "http://auth.wonton.internal/.well-known/jwks.json",
        "revocation_endpoint": "http://auth.wonton.internal/oauth2/revoke",
        "end_session_endpoint": "http://auth.wonton.localhost/oauth2/sessions/logout",
        "scopes_supported": ["openid"],
    }))
}

/// 假插件：把收到的请求（路径和请求头）原样回显；`/ws` 支持协议升级后回显字节；
/// `/set-cookie?c=..&c=..` 把每个 `c` 原样作为一条 `Set-Cookie` 返回。
pub(super) async fn echo_upstream(hits: Arc<AtomicUsize>) -> SocketAddr {
    let router = Router::new()
        .route("/ws", get(upgrade_echo))
        .route("/set-cookie", get(set_cookies))
        .route("/.well-known/openid-configuration", get(discovery))
        .fallback(move |req: Request| {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let headers: serde_json::Map<String, Value> = req
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(String::from_utf8_lossy(v.as_bytes()))))
                    .collect();
                Json(json!({"path": req.uri().to_string(), "headers": headers}))
            }
        });
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr
}

async fn set_cookies(RawQuery(query): RawQuery) -> Response {
    let mut response = Response::new(Body::empty());
    for (_, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        let value = value.parse().unwrap();
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn upgrade_echo(mut req: Request) -> Response {
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let mut io = hyper_util::rt::TokioIo::new(upgraded);
            let mut buf = [0u8; 64];
            if let Ok(n) = io.read(&mut buf).await {
                let _ = io.write_all(&buf[..n]).await;
            }
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "echo")
        .body(Body::empty())
        .unwrap()
}

/// 假 Hydra 管理接口：返回预设的登录、授权请求，记录核心提交的接受内容。授权请求 challenge `def` 的
/// `subject` 是 `family_subject`（家人的声明测试传家人的用户名），其余 challenge 都是 `owner`。
pub(super) async fn fake_hydra(accepted: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    fake_hydra_for(accepted, "owner").await
}

pub(super) async fn fake_hydra_for(accepted: Arc<Mutex<Vec<Value>>>, subject: &str) -> SocketAddr {
    fake_hydra_scopes(
        accepted,
        subject,
        &["openid", "profile", "email", "groups", "admin"],
    )
    .await
}

/// 同 [`fake_hydra_for`]，授权请求只申请给定的 scope。
pub(super) async fn fake_hydra_scopes(
    accepted: Arc<Mutex<Vec<Value>>>,
    subject: &str,
    scopes: &'static [&'static str],
) -> SocketAddr {
    let subject = subject.to_owned();
    let record = |accepted: Arc<Mutex<Vec<Value>>>, redirect: &'static str| {
        move |Json(body): Json<Value>| async move {
            accepted.lock().unwrap().push(body);
            Json(json!({"redirect_to": redirect}))
        }
    };
    let router = Router::new()
        .route(
            "/admin/oauth2/auth/requests/login",
            get(|| async {
                Json(json!({"skip": false, "subject": "", "client": {"client_id": "app.whoami"}}))
            }),
        )
        .route(
            "/admin/oauth2/auth/requests/login/accept",
            axum::routing::put(record(
                accepted.clone(),
                "http://auth.wonton.localhost/after-login",
            )),
        )
        .route(
            "/admin/oauth2/auth/requests/consent",
            get(
                move |axum::extract::Query(challenge): axum::extract::Query<
                    HashMap<String, String>,
                >| {
                    // challenge 兼作客户端 ID：按客户端区分声明的测试用（oidc.owner_groups）。
                    let client_id = challenge
                        .get("consent_challenge")
                        .cloned()
                        .unwrap_or_else(|| "app.whoami".to_owned());
                    let subject =
                        if challenge.get("consent_challenge").map(String::as_str) == Some("def") {
                            subject.clone()
                        } else {
                            "owner".to_owned()
                        };
                    async move {
                        Json(json!({
                            "subject": subject,
                            "client": {"client_id": client_id},
                            "requested_scope": scopes,
                        }))
                    }
                },
            ),
        )
        .route(
            "/admin/oauth2/auth/requests/consent/accept",
            axum::routing::put(record(
                accepted,
                "http://auth.wonton.localhost/after-consent",
            )),
        );
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr
}

/// 只走回环地址的 iroh endpoint：不连中继，不做地址发现。
pub(super) async fn loopback_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// 测试用的隧道：空的设备登记表和家人记录，加一个只走回环地址的 endpoint。返回家人记录，
/// 和隧道里的配对用的是同一份。局域网地址给一个测试值（ADR-0032 的直连地址选择）。
pub(super) async fn loopback_tunnel(dir: &Path) -> (Tunnel, Arc<Members>) {
    loopback_tunnel_with(dir, ["192.168.1.2".parse().unwrap()].into()).await
}

/// 同 [`loopback_tunnel`]，局域网地址由调用方给（宣告地址或检测到的地址）。
pub(super) async fn loopback_tunnel_with(
    dir: &Path,
    lan: std::collections::BTreeSet<std::net::IpAddr>,
) -> (Tunnel, Arc<Members>) {
    let members = Arc::new(Members::open(dir.join("members.json")).unwrap());
    (
        Tunnel::new(
            Arc::new(Devices::open(dir.join("devices.json")).unwrap()),
            members.clone(),
            Arc::new(|| Languages::from_header(None)),
            loopback_endpoint(SecretKey::generate()).await,
            0,
            RelayTable::builtin_n0(),
            tokio::sync::watch::channel(lan).1,
        ),
        members,
    )
}
