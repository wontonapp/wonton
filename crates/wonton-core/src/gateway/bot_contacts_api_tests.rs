//! Bot 联系人管理接口的行为测试（ADR-0050）：只收经隧道的已配对设备、归属人门、错误类别写在
//! `X-Wonton-Error` 头和错误体里且两者一致。联系人状态机本身的行为在 `contacts` 模块的测试里。

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
use wonton_protocol::{BotRef, CONTACT_ERROR_HEADER, bot_contact_path, bot_contacts_path};

use super::fakes::loopback_tunnel;
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::contacts::Contacts;
use crate::contacts::test_support::{FakeBots, FakePeers, FakePush};
use crate::http::HttpClient;
use crate::owner::OwnerStore;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, docker::Engine};

use super::tests::{CORE, TOKEN};

const PORT: u16 = 18480;

fn helper_bot() -> BotRef {
    BotRef {
        plugin_id: "com.example.alpha".into(),
        bot: "helper".into(),
    }
}

struct Home {
    app: Router,
    device: iroh::EndpointId,
    _dir: tempfile::TempDir,
}

/// 起一台带联系人表的核心：回环隧道、一台主人的设备、假的配对名单和 Bot 名录。
async fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    // 生产里联系人只在 Matrix 就绪的核心上有（main.rs），这里同样接上假 Tuwunel。
    let (_fake, matrix_config) = crate::matrix::fake::start().await;
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&matrix_config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
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
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let device = SecretKey::generate().public();
    tunnel.devices.add(device, None).unwrap();
    let peers = Arc::new(FakePeers::default());
    let bots = Arc::new(FakeBots::default());
    bots.add(
        &format!("@helper:{}", CORE),
        &helper_bot(),
        "阿福",
        "云吞",
        "owner",
    );
    let push = Arc::new(FakePush::default());
    let contacts = Contacts::open(dir.path().join("contacts.json"), peers)
        .unwrap()
        .with_matrix(matrix.clone())
        .with_bots(bots)
        .with_push(push);
    manager.attach_contacts(contacts.clone());
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members,
        manager,
        None,
        HttpClient::new(),
        tunnel,
    )
    .with_matrix(Some(matrix))
    .with_contacts(Some(contacts.clone()));
    let app = router(Arc::new(gateway));
    Home {
        app,
        device,
        _dir: dir,
    }
}

/// 经隧道的设备请求：网关按隧道连接认设备，测试直接塞 `TunnelPeer` 扩展（同 peers 接口测试）。
async fn call(
    home: &Home,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(format!("http://{CORE}:{PORT}{path}"))
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header(
            "content-type",
            if body.is_some() {
                "application/json"
            } else {
                "text/plain"
            },
        );
    request = request.extension(TunnelPeer(home.device));
    let request = match body {
        Some(body) => request.body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, body)
}

#[tokio::test]
async fn the_list_starts_empty_and_the_shape_is_the_protocol_view() {
    let home = home().await;
    let (status, _, body) = call(&home, Method::GET, &bot_contacts_path(&helper_bot()), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"contacts": []}));
}

#[tokio::test]
async fn requests_need_a_paired_server() {
    let home = home().await;
    let path = bot_contacts_path(&helper_bot());
    let (status, headers, body) = call(
        &home,
        Method::POST,
        &path,
        Some(json!({"user_id": "@helper:elsewhere.example", "kind": "bot"})),
    )
    .await;
    // 没配对是本机就知道的状态：403，不用问对方。
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let kind = body["error"]["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("响应没有错误类别：{body}"));
    assert_eq!(headers[CONTACT_ERROR_HEADER], kind);
    assert_eq!(kind, wonton_protocol::ContactError::NotPaired.as_str());
}

#[tokio::test]
async fn the_404_row_clears_with_delete() {
    let home = home().await;
    let path = bot_contact_path(&helper_bot(), "@ghost:elsewhere.example");
    let (status, headers, body) = call(&home, Method::DELETE, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let kind = body["error"]["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("响应没有错误类别：{body}"));
    assert_eq!(headers[CONTACT_ERROR_HEADER], kind);
    assert_eq!(
        kind,
        wonton_protocol::ContactError::ContactNotFound.as_str()
    );
}

#[tokio::test]
async fn without_a_tunnel_device_the_api_stays_shut() {
    let home = home().await;
    let path = bot_contacts_path(&helper_bot());
    let request = hyper::Request::builder()
        .method(Method::GET)
        .uri(format!("http://{CORE}:{PORT}{path}"))
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body["error"]["kind"].as_str().unwrap(),
        wonton_protocol::ContactError::Unauthenticated.as_str()
    );
}
