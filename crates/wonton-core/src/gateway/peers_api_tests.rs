//! 服务器配对接口的行为测试（ADR-0050）：只收主人设备经隧道来的请求，错误类别写在 `X-Wonton-Error`
//! 头和错误体里且两者一致。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{PEER_ERROR_HEADER, PEERS_PATH};

use super::tunnel_tests::loopback_tunnel;
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::owner::OwnerStore;
use crate::peers::{OwnIdentity, PeerRequest, Peers};
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, docker::Engine};

use super::tests::{CORE, TOKEN};

const PORT: u16 = 18480;

/// 起一台带配对的核心：回环隧道、空的设备表加一台登记好的设备。`pending` 是预置的待处理请求。
async fn home_with(
    pending: Vec<(&str, &str)>,
) -> (Router, Arc<Peers>, iroh::EndpointId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let requests: Vec<PeerRequest> = pending
        .iter()
        .map(|(id, server_name)| PeerRequest {
            id: (*id).to_owned(),
            endpoint_id: SecretKey::generate().public(),
            server_name: (*server_name).to_owned(),
            owner_user_id: format!("@owner:{server_name}"),
            owner_display_name: "Jiale".to_owned(),
            relays: Vec::new(),
            addrs: Vec::new(),
            received_at_ms: 1_799_000_000_000,
        })
        .collect();
    std::fs::create_dir_all(dir.path().join("core")).unwrap();
    std::fs::write(
        dir.path().join("core/peers.json"),
        serde_json::to_string(&crate::peers::Stored {
            requests,
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: PORT,
    };
    let profile = OwnerStore::for_test(
        dir.path(),
        &crate::owner::Owner {
            username: "owner".into(),
            display_name: "云吞".into(),
            email: None,
        },
    );
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    let tunnel = loopback_tunnel(dir.path()).await;
    let peers = Peers::open(
        dir.path().join("core/peers.json"),
        OwnIdentity {
            server_name: "a.example.internal".to_owned(),
            owner_user_id: "@owner:a.example.internal".to_owned(),
        },
        tunnel.clone(),
        None,
        Arc::new(|| "云吞".to_owned()),
        Arc::new(|| crate::plugin::languages::Languages::from_header(Some("zh"))),
    )
    .unwrap();
    // 登记一台设备：经隧道来的请求都带着它的身份。
    let device = SecretKey::generate().public();
    tunnel.devices.add(device, None).unwrap();
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        manager,
        None,
        HttpClient::new(),
        tunnel,
    )
    .with_peers(Some(peers.clone()));
    let app = router(Arc::new(gateway));
    (app, peers, device, dir)
}

async fn home() -> (Router, Arc<Peers>, iroh::EndpointId, tempfile::TempDir) {
    home_with(Vec::new()).await
}

/// 以设备身份发请求；`device` 为 `None` 时请求不带隧道身份。
async fn request(
    app: &Router,
    method: Method,
    path: &str,
    body: &str,
    device: Option<iroh::EndpointId>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(format!("http://{CORE}:{PORT}{path}"))
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(device) = device {
        // 和隧道入口一样：扩展里放的是 TunnelPeer 本身，网关按类型取。
        builder = builder.extension(TunnelPeer(device));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, value)
}

fn error_kind(value: &Value) -> &str {
    value["error"]["kind"].as_str().unwrap_or_default()
}

fn view(peers: &Peers) -> wonton_protocol::PeersView {
    peers.view(&crate::plugin::languages::Languages::from_header(Some(
        "zh",
    )))
}

/// 不是经隧道的已配对设备：一律 401，头和错误体里的类别一致。
#[tokio::test]
async fn requests_without_a_paired_device_are_rejected() {
    let (app, _peers, _device, _dir) = home().await;
    let stranger = SecretKey::generate().public();
    for (method, path, device) in [
        (Method::GET, PEERS_PATH.to_owned(), None),
        (Method::GET, PEERS_PATH.to_owned(), Some(stranger)),
        (Method::POST, PEERS_PATH.to_owned(), None),
        (
            Method::GET,
            wonton_protocol::peer_code_path(),
            Some(stranger),
        ),
        (
            Method::POST,
            wonton_protocol::peer_code_reset_path(),
            Some(stranger),
        ),
        (
            Method::POST,
            wonton_protocol::peer_request_approve_path("r1"),
            Some(stranger),
        ),
        (
            Method::POST,
            wonton_protocol::peer_request_reject_path("r1"),
            Some(stranger),
        ),
        (
            Method::DELETE,
            wonton_protocol::peer_rejected_path("r1"),
            Some(stranger),
        ),
    ] {
        let (status, headers, value) = request(&app, method.clone(), &path, "", device).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(
            headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
            Some("unauthenticated"),
            "{method} {path}"
        );
        assert_eq!(error_kind(&value), "unauthenticated");
    }
}

/// 已配对设备：列表、取码、重置、坏码和删除都按类别回。
#[tokio::test]
async fn paired_devices_get_the_view_and_the_documented_errors() {
    let (app, peers, device, _dir) = home().await;

    // 列表：空，码不在列表里。
    let (status, _, value) = request(&app, Method::GET, PEERS_PATH, "", Some(device)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["peers"], json!([]));
    assert_eq!(value["requests"], json!([]));
    assert_eq!(value["rejected"], json!([]));

    // 取码：长期有效的一张，再取还是它。
    let (status, _, value) = request(
        &app,
        Method::GET,
        &wonton_protocol::peer_code_path(),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = value["code"].as_str().expect("码是字符串").to_owned();
    assert!(code.starts_with("wonton://peer?v=1&"), "{code}");
    let (_, _, again) = request(
        &app,
        Method::GET,
        &wonton_protocol::peer_code_path(),
        "",
        Some(device),
    )
    .await;
    assert_eq!(again["code"].as_str().unwrap(), code);

    // 重置：换一张。
    let (status, _, latest) = request(
        &app,
        Method::POST,
        &wonton_protocol::peer_code_reset_path(),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(latest["code"].as_str().unwrap(), code);

    // 扫一个读不懂的码：400 code_invalid，头和体一致。
    let (status, headers, value) = request(
        &app,
        Method::POST,
        PEERS_PATH,
        &json!({"code": "wonton://pair?v=0"}).to_string(),
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
        Some("code_invalid")
    );
    assert_eq!(error_kind(&value), "code_invalid");

    // 扫自己的码：400 self_pairing。
    let own = latest["code"].as_str().unwrap().to_owned();
    let (status, headers, value) = request(
        &app,
        Method::POST,
        PEERS_PATH,
        &json!({"code": own}).to_string(),
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
        Some("self_pairing")
    );
    assert_eq!(error_kind(&value), "self_pairing");

    // 处理一条不在的请求：404 request_not_found。
    for path in [
        wonton_protocol::peer_request_approve_path("gone"),
        wonton_protocol::peer_request_reject_path("gone"),
    ] {
        let (status, headers, value) = request(&app, Method::POST, &path, "", Some(device)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(
            headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
            Some("request_not_found")
        );
        assert_eq!(error_kind(&value), "request_not_found");
    }

    // 删没配对过的：404。
    let (status, _, _) = request(
        &app,
        Method::DELETE,
        &wonton_protocol::peer_path("ghost.example.internal"),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = peers;
}

/// 请求的同意、拒绝和撤销：逐条经接口处理，列表跟着变。
#[tokio::test]
async fn requests_are_approved_rejected_and_unrejected_over_the_api() {
    let (app, peers, device, _dir) = home_with(vec![
        ("r1", "b.example.internal"),
        ("r2", "c.example.internal"),
    ])
    .await;

    let (status, _, value) = request(&app, Method::GET, PEERS_PATH, "", Some(device)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["requests"].as_array().unwrap().len(), 2);

    // 同意：请求没了，名单里多一台。
    let (status, _, _) = request(
        &app,
        Method::POST,
        &wonton_protocol::peer_request_approve_path("r1"),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = view(&peers);
    assert_eq!(listed.requests.len(), 1);
    assert_eq!(listed.peers.len(), 1);
    assert_eq!(listed.peers[0].server_name, "b.example.internal");
    assert_eq!(
        listed.peers[0].status,
        wonton_protocol::PeerStatus::Checking
    );

    // 同意同一个请求第二次：已经不在了。
    let (status, headers, _) = request(
        &app,
        Method::POST,
        &wonton_protocol::peer_request_approve_path("r1"),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
        Some("request_not_found")
    );

    // 拒绝：进“已拒绝”。
    let (status, _, _) = request(
        &app,
        Method::POST,
        &wonton_protocol::peer_request_reject_path("r2"),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = view(&peers);
    assert!(listed.requests.is_empty());
    assert_eq!(listed.rejected.len(), 1);
    let rejected = listed.rejected[0].id.clone();

    // 撤销拒绝。
    let (status, _, _) = request(
        &app,
        Method::DELETE,
        &wonton_protocol::peer_rejected_path(&rejected),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(view(&peers).rejected.is_empty());

    // 撤销第二次：不在了。
    let (status, _, _) = request(
        &app,
        Method::DELETE,
        &wonton_protocol::peer_rejected_path(&rejected),
        "",
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// 错误类别和状态码的对应关系照 `PeerError::http_status`；类别的写法就是 `X-Wonton-Error` 头的值。
#[tokio::test]
async fn a_scan_failure_carries_the_header_and_the_body_kind() {
    let (app, _peers, device, _dir) = home().await;
    // 连不上的码：502 peer_unreachable，头和错误体里的类别一致。
    let unreachable = "wonton://peer?v=1&id=1111111111111111111111111111111111111111111111111111111111111111&relay=https%3A%2F%2Frelay.example.com.%2F&name=far.example.internal&token=01010101010101010101010101010101";
    let (status, headers, value) = request(
        &app,
        Method::POST,
        PEERS_PATH,
        &json!({"code": unreachable}).to_string(),
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{value}");
    assert_eq!(
        headers.get(PEER_ERROR_HEADER).and_then(|v| v.to_str().ok()),
        Some("peer_unreachable")
    );
    assert_eq!(error_kind(&value), "peer_unreachable");
}
