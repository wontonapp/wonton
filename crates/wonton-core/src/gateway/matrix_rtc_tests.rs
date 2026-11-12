//! MatrixRTC 授权服务的两个入口（ADR-0096）：设备经 Matrix 对外地址的 `/_wonton/matrix-rtc` 到达提供方；
//! 提供方网络里的服务器名别名只答联邦发现和 OpenID 用户信息。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::response::IntoResponse;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;
use tower::ServiceExt;

use super::fakes::{bind, serve};
use super::matrix_rtc::KIND_UNAVAILABLE;
use super::matrix_rtc_alias::{DELAYED_EVENTS_PREFIX, USERINFO_PATH, WELL_KNOWN_SERVER_PATH};
use super::tests::{Harness, Reply, TOKEN, owner};
use super::{Gateway, TunnelPeer, matrix_rtc_alias};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::owner::OwnerStore;
use crate::plugin::PluginStatus;
use crate::plugin::manifest::{Manifest, PluginId};
use crate::plugin::upstreams::Endpoint;

const PROVIDER: &str = "app.call";
const PROVIDER_NET: &str = "10.77.0.0/24";

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

/// 经隧道的已配对设备发往 `matrix.<base>` 的请求。
async fn from_phone(h: &Harness, method: Method, path: &str) -> Reply {
    let phone = SecretKey::generate().public();
    h.tunnel.devices.add(phone, None).unwrap();
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "matrix.wonton.localhost")
        .header(header::ORIGIN, "http://app-call.wonton.localhost")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(TunnelPeer(phone));
    Reply::of(h.app.clone().oneshot(req).await.unwrap()).await
}

#[tokio::test]
async fn without_a_provider_the_prefix_says_so_and_the_page_can_read_it() {
    let h = Harness::start(None).await;
    let reply = from_phone(&h, Method::POST, "/_wonton/matrix-rtc/sfu/get").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some(KIND_UNAVAILABLE));
    assert_eq!(reply.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(reply.json()["errcode"], "M_UNKNOWN");
}

#[tokio::test]
async fn a_stopped_provider_is_reported_as_unavailable() {
    let h = Harness::start(None).await;
    h.manager
        .register_for_test(provider(), PluginStatus::Stopped, None);
    let reply = from_phone(&h, Method::POST, "/_wonton/matrix-rtc/sfu/get").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some(KIND_UNAVAILABLE));
    assert!(reply.text().contains(PROVIDER), "{}", reply.text());
}

#[tokio::test]
async fn the_prefix_is_stripped_and_forwarded_without_a_matrix_token() {
    let h = Harness::start(None).await;
    h.manager
        .register_for_test(provider(), PluginStatus::Running, None);
    h.manager.matrix_rtc_route_for_test(
        &PluginId::parse(PROVIDER).unwrap(),
        Endpoint::Tcp(h.upstream.to_string()),
    );
    let reply = from_phone(&h, Method::POST, "/_wonton/matrix-rtc/sfu/get?x=1").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["path"], "/sfu/get?x=1");
    // 预检也到提供方：CORS 由授权服务自己答。
    let reply = from_phone(&h, Method::OPTIONS, "/_wonton/matrix-rtc/get_token").await;
    assert_eq!(reply.json()["path"], "/get_token");
}

#[tokio::test]
async fn the_prefix_still_needs_a_paired_device() {
    let h = Harness::start(None).await;
    h.manager
        .register_for_test(provider(), PluginStatus::Running, None);
    h.manager.matrix_rtc_route_for_test(
        &PluginId::parse(PROVIDER).unwrap(),
        Endpoint::Tcp(h.upstream.to_string()),
    );
    let reply = h
        .send(
            Method::POST,
            "matrix.wonton.localhost",
            "/_wonton/matrix-rtc/sfu/get",
            &[],
            Body::empty(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(h.hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// 网关接着一个假 Tuwunel：OpenID 用户信息由前面的记录器答，其余转给假 Tuwunel。
struct Alias {
    app: Router,
    server_name: String,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
    h: Harness,
}

impl Alias {
    async fn start() -> Self {
        let h = Harness::start(None).await;
        let (_fake, mut config) = crate::matrix::fake::start().await;
        let fake_authority = config.url.trim_start_matches("http://").to_owned();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let http = HttpClient::new();
        let upstream = Router::new().fallback(move |req: Request| {
            let (recorded, http, authority) =
                (recorded.clone(), http.clone(), fake_authority.clone());
            async move {
                if req.uri().path() == USERINFO_PATH {
                    recorded.lock().unwrap().push(req.uri().to_string());
                    return axum::Json(json!({"sub": "@owner:test"})).into_response();
                }
                if req.uri().path().starts_with(DELAYED_EVENTS_PREFIX) {
                    recorded
                        .lock()
                        .unwrap()
                        .push(format!("{} {}", req.method(), req.uri()));
                    return axum::Json(json!({})).into_response();
                }
                let (mut parts, body) = req.into_parts();
                parts.uri = format!("http://{authority}{}", parts.uri).parse().unwrap();
                let response = http.send(Request::from_parts(parts, body)).await.unwrap();
                let (parts, body) = response.into_parts();
                axum::response::Response::from_parts(parts, Body::new(body))
            }
        });
        let (listener, addr) = bind().await;
        serve(listener, upstream).await;
        config.url = format!("http://{addr}");
        let matrix = Matrix::connect(&config, &h.data_dir.join("rtc-matrix"), HttpClient::new())
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
        let server_name = matrix.server_name().to_owned();
        gateway.matrix = Some(Arc::new(matrix));
        h.manager
            .register_for_test(provider(), PluginStatus::Running, None);
        h.manager
            .register_source_for_test(PROVIDER, PROVIDER_NET, Vec::new());
        Self {
            app: matrix_rtc_alias::router(Arc::new(gateway)),
            server_name,
            seen,
            h,
        }
    }

    async fn get(&self, from: &str, host: &str, path: &str) -> Reply {
        self.send(Method::GET, from, host, path).await
    }

    async fn send(&self, method: Method, from: &str, host: &str, path: &str) -> Reply {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        let addr: SocketAddr = format!("{from}:40000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        Reply::of(self.app.clone().oneshot(req).await.unwrap()).await
    }
}

#[tokio::test]
async fn the_alias_answers_discovery_and_userinfo_for_the_provider_only() {
    let alias = Alias::start().await;
    let host = format!("{}:443", alias.server_name);
    let reply = alias.get("10.77.0.5", &host, WELL_KNOWN_SERVER_PATH).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({"m.server": format!("{}:443", alias.server_name)})
    );
    let reply = alias
        .get(
            "10.77.0.5",
            &host,
            &format!("{USERINFO_PATH}?access_token=opaque"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["sub"], "@owner:test");
    assert_eq!(
        *alias.seen.lock().unwrap(),
        [format!("{USERINFO_PATH}?access_token=opaque")]
    );
    for path in [
        "/_matrix/federation/v1/version",
        "/_matrix/key/v2/server",
        "/_matrix/client/versions",
        "/_matrix/client/v1/delayed_events/abc/restart",
    ] {
        let reply = alias.get("10.77.0.5", &host, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
    }
    // 授权服务代管 delayed leave：不稳定路径上的操作原样转给 Tuwunel，GET 不转。
    let action = format!("{DELAYED_EVENTS_PREFIX}abc/restart");
    let reply = alias.send(Method::POST, "10.77.0.5", &host, &action).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = alias.get("10.77.0.5", &host, &action).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        alias.seen.lock().unwrap().last().unwrap(),
        &format!("POST {action}")
    );

    // 别的插件网络、别的主机名都不答。
    let reply = alias.get("10.78.0.5", &host, WELL_KNOWN_SERVER_PATH).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = alias
        .get(
            "10.77.0.5",
            "matrix.wonton.localhost",
            WELL_KNOWN_SERVER_PATH,
        )
        .await;
    assert_eq!(reply.status, StatusCode::MISDIRECTED_REQUEST);
    assert_eq!(alias.seen.lock().unwrap().len(), 2);
    drop(alias.h);
}
