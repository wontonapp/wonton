//! 推送的网关行为：登记只认隧道身份，推送代理只给 Matrix 服务器，吊销设备时推送登记一并作废。推送服务的替身和
//! 解密见 `crate::push` 的测试；这里只看网关。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use iroh::{EndpointId, SecretKey};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::tests::{CORE, TOKEN, owner};
use super::tunnel_tests::loopback_tunnel;
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::push::Push;
use crate::tunnel::Tunnel;

/// RFC 8291 附录 A 里设备的公钥和认证密钥。
const P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const PUSH_HOST: &str = "push.wonton.internal";
/// 假的 Tuwunel 在 127.0.0.1 上，它经出站代理发来的请求的 TCP 来源。
const TUWUNEL: &str = "127.0.0.1:41000";

struct Home {
    app: Router,
    tunnel: Tunnel,
    plugins: Arc<PluginManager>,
    push: Arc<Push>,
    /// 一台已登记的设备。
    phone: EndpointId,
    _dir: tempfile::TempDir,
}

impl Home {
    /// 核心接着假的 Tuwunel（`crate::matrix::fake`），它在 127.0.0.1 上：从 127.0.0.1 来的请求就是 Tuwunel 发的。
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (_, config) = crate::matrix::fake::start().await;
        let matrix = Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap();
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: 18480,
        };
        let profile = OwnerStore::for_test(dir.path(), &owner());
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
        let http = HttpClient::new();
        let plugins = Arc::new(PluginManager::new(
            Engine::unreachable_for_test(),
            Store::open(dir.path().join("plugins")).unwrap(),
            http.clone(),
            None,
            lifecycle,
            ImageWorker::in_process(),
        ));
        let tunnel = loopback_tunnel(dir.path()).await;
        let phone = SecretKey::generate().public();
        tunnel.devices.add(phone, None).unwrap();
        let push =
            Arc::new(Push::open(dir.path().join("push.json"), tunnel.devices.clone()).unwrap());
        let gateway = Gateway::new(
            public,
            profile,
            TOKEN.into(),
            plugins.clone(),
            None,
            http,
            tunnel.clone(),
        )
        .with_matrix(Some(Arc::new(matrix)))
        .with_push(Some(push.clone()));
        Self {
            app: router(Arc::new(gateway)),
            tunnel,
            plugins,
            push,
            phone,
            _dir: dir,
        }
    }

    /// 发一个请求：`peer` 是隧道认出的设备，`from` 是 TCP 来源地址（局域网或插件网络）。
    async fn send(
        &self,
        method: Method,
        host: &str,
        path: &str,
        peer: Option<EndpointId>,
        from: &str,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        self.send_raw(method, host, path, peer, from, body.to_string())
            .await
    }

    /// 同 [`Self::send`]，请求体原样发出（比如有重复的键）。
    async fn send_raw(
        &self,
        method: Method,
        host: &str,
        path: &str,
        peer: Option<EndpointId>,
        from: &str,
        body: String,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .unwrap();
        let from: SocketAddr = from.parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(from));
        if let Some(id) = peer {
            request.extensions_mut().insert(TunnelPeer(id));
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }

    async fn register(&self, peer: Option<EndpointId>) -> (StatusCode, HeaderMap, Value) {
        let subscription = json!({
            "endpoint": "https://ntfy.example/upWonton123?up=1",
            "keys": {"p256dh": P256DH, "auth": AUTH},
        });
        self.send(
            Method::PUT,
            CORE,
            "/_wonton/push/registration",
            peer,
            "127.0.0.1:50000",
            subscription,
        )
        .await
    }

    /// Tuwunel 经出站代理发来的通知：TCP 来源是它在核心网络里的地址。
    async fn notify(
        &self,
        from: &str,
        peer: Option<EndpointId>,
        pushkey: &str,
    ) -> (StatusCode, Value) {
        let notification = json!({"notification": {
            "counts": {"unread": 1},
            "devices": [{"app_id": "app.wonton.android", "pushkey": pushkey}],
        }});
        let (status, _, body) = self
            .send(
                Method::POST,
                PUSH_HOST,
                "/_matrix/push/v1/notify",
                peer,
                from,
                notification,
            )
            .await;
        (status, body)
    }
}

fn error_kind(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-wonton-error")
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn only_paired_devices_through_the_tunnel_can_register_for_push() {
    let home = Home::start().await;

    let (status, _, body) = home.register(Some(home.phone)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["gateway_url"],
        "http://push.wonton.internal/_matrix/push/v1/notify"
    );
    let pushkey = body["pushkey"].as_str().unwrap().to_owned();
    assert_eq!(pushkey.len(), 64, "256 位随机数");
    let (_, _, again) = home.register(Some(home.phone)).await;
    assert_eq!(
        again["pushkey"],
        pushkey.as_str(),
        "再登记一次，pushkey 不变"
    );

    // 局域网（没有隧道身份）和没登记的设备都不行。
    let (status, headers, _) = home.register(None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_kind(&headers), Some("unauthenticated"));
    let stranger = SecretKey::generate().public();
    let (status, _, _) = home.register(Some(stranger)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, body) = home
        .send(
            Method::PUT,
            CORE,
            "/_wonton/push/registration",
            Some(home.phone),
            "127.0.0.1:50000",
            json!({"endpoint": "ftp://ntfy.example/up", "keys": {"p256dh": P256DH, "auth": AUTH}}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error_kind(&headers), Some("invalid_input"));

    let unregister = |peer| {
        home.send(
            Method::DELETE,
            CORE,
            "/_wonton/push/registration",
            peer,
            "127.0.0.1:50000",
            Value::Null,
        )
    };
    let (status, _, body) = unregister(Some(home.phone)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"removed": true})));
    let (_, _, body) = unregister(Some(home.phone)).await;
    assert_eq!(body, json!({"removed": false}));
}

#[tokio::test]
async fn the_push_gateway_answers_only_the_matrix_server() {
    let home = Home::start().await;
    let (_, _, body) = home.register(Some(home.phone)).await;
    let pushkey = body["pushkey"].as_str().unwrap().to_owned();
    home.plugins
        .register_source_for_test("app.echo", "172.30.1.0/24", Vec::new());

    // 只有计数的通知不转发，这里不碰推送服务。
    let (status, body) = home.notify(TUWUNEL, None, &pushkey).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"rejected": []})));
    let (status, body) = home.notify(TUWUNEL, None, "guessed").await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, json!({"rejected": ["guessed"]})),
        "不认识的 pushkey 回 rejected，Tuwunel 删掉这个 pusher"
    );

    // 只认 Tuwunel 的地址：插件网络里的容器、局域网、经隧道的设备都冒充不了它。
    for (from, peer, who) in [
        ("172.30.1.7:41000", None, "插件网络"),
        ("192.168.1.20:41000", None, "局域网"),
        (TUWUNEL, Some(home.phone), "经隧道的设备"),
    ] {
        let (status, body) = home.notify(from, peer, &pushkey).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who}");
        assert_eq!(body["errcode"], "M_FORBIDDEN", "{who}");
    }

    let (status, _, body) = home
        .send(
            Method::GET,
            PUSH_HOST,
            "/_matrix/push/v1/notify",
            None,
            TUWUNEL,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["errcode"], "M_UNRECOGNIZED");
    let (status, _, body) = home
        .send(
            Method::POST,
            PUSH_HOST,
            "/_matrix/push/v1/notify",
            None,
            TUWUNEL,
            json!({"not": "a notification"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["errcode"], "M_BAD_JSON");
}

#[tokio::test]
async fn revoking_a_device_drops_its_push_registration() {
    let home = Home::start().await;
    let (_, _, body) = home.register(Some(home.phone)).await;
    let pushkey = body["pushkey"].as_str().unwrap().to_owned();
    assert!(home.push.registration(&home.phone).is_some());

    let (status, _, body) = home
        .send(
            Method::DELETE,
            CORE,
            &format!("/_wonton/api/devices/{}", home.phone),
            None,
            "127.0.0.1:50000",
            Value::Null,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "管理 API 要主人令牌：{body}"
    );
    let mut request = hyper::Request::builder()
        .method(Method::DELETE)
        .uri(format!("/_wonton/api/devices/{}", home.phone))
        .header(header::HOST, CORE)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:50000".parse::<SocketAddr>().unwrap(),
    ));
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(!home.tunnel.devices.contains(&home.phone));
    assert!(
        home.push.registration(&home.phone).is_none(),
        "吊销设备时删掉推送登记"
    );
    let (_, body) = home.notify(TUWUNEL, None, &pushkey).await;
    assert_eq!(body, json!({"rejected": [pushkey]}));
}

/// pusher 的注册请求（ruma 的 `set_pusher` 的请求体）。
fn pusher(kind: Value, url: &str) -> Value {
    json!({
        "kind": kind, "app_id": "app.wonton", "pushkey": "k", "app_display_name": "Wonton",
        "device_display_name": "手机", "lang": "zh-CN", "data": {"url": url},
    })
}

#[tokio::test]
async fn bots_may_only_register_pushers_that_point_at_the_push_proxy() {
    let home = Home::start().await;
    let bot = vec!["matrix.bot".parse().unwrap()];
    home.plugins
        .register_source_for_test("agent.echo", "172.30.1.0/24", bot);
    let set = |host: &'static str, peer, from: &'static str, body: Value| {
        home.send(
            Method::POST,
            host,
            "/_matrix/client/v3/pushers/set",
            peer,
            from,
            body,
        )
    };
    let from_bot = |body| set("matrix.wonton.internal", None, "172.30.1.5:40000", body);

    let allowed = pusher(
        json!("http"),
        "http://push.wonton.internal/_matrix/push/v1/notify",
    );
    let (status, _, body) = from_bot(allowed.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["received"], allowed, "请求体完整地转给了 Tuwunel");
    let delete = json!({"kind": null, "app_id": "app.wonton", "pushkey": "k"});
    let (status, _, body) = from_bot(delete.clone()).await;
    assert_eq!(
        (status, &body["received"]),
        (StatusCode::OK, &delete),
        "删除照常放行"
    );
    // 指向外网的 pusher 会让 Tuwunel 替没有出网权限的 Bot 把通知推出去。
    for refused in [
        pusher(
            json!("http"),
            "https://attacker.example/_matrix/push/v1/notify",
        ),
        pusher(json!("http"), "http://hydra:4445/_matrix/push/v1/notify"),
        pusher(json!("email"), "mailto:owner@example.com"),
    ] {
        let (status, headers, reply) = from_bot(refused.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(reply["errcode"], "M_FORBIDDEN");
        assert_eq!(error_kind(&headers), Some("permission_denied"));
    }
    let (status, _, reply) = from_bot(json!({"app_id": "app.wonton", "pushkey": "k"})).await;
    assert_eq!(
        (status, reply["errcode"].as_str()),
        (StatusCode::BAD_REQUEST, Some("M_BAD_JSON")),
        "缺 kind"
    );

    // 主人经隧道的设备不受限：Element X 这类客户端注册自己的推送网关。
    let element = pusher(json!("http"), "https://matrix.org/_matrix/push/v1/notify");
    let (status, _, body) = set(
        "matrix.wonton.localhost",
        Some(home.phone),
        "127.0.0.1:50000",
        element.clone(),
    )
    .await;
    assert_eq!((status, &body["received"]), (StatusCode::OK, &element));
}

/// 原文里重复的键：网关检查的是解析出来的那一份（`serde_json` 留最后一个），转给 Tuwunel 的也必须正好是这一份，
/// 不能让 Tuwunel 自己去解析原文、取到另一个地址。
#[tokio::test]
async fn a_bot_pusher_reaches_tuwunel_exactly_as_checked() {
    let home = Home::start().await;
    let bot = vec!["matrix.bot".parse().unwrap()];
    home.plugins
        .register_source_for_test("agent.echo", "172.30.1.0/24", bot);
    let proxy = "http://push.wonton.internal/_matrix/push/v1/notify";
    let outside = "https://attacker.example/_matrix/push/v1/notify";
    let body = |first: &str, last: &str| {
        format!(
            r#"{{"kind":"http","app_id":"app.wonton","pushkey":"k","app_display_name":"Wonton","device_display_name":"Bot","lang":"zh-CN","data":{{"url":"{first}","url":"{last}"}}}}"#
        )
    };
    let set = |body: String| {
        home.send_raw(
            Method::POST,
            "matrix.wonton.internal",
            "/_matrix/client/v3/pushers/set",
            None,
            "172.30.1.5:40000",
            body,
        )
    };

    let (status, _, reply) = set(body(outside, proxy)).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let raw = reply["raw"].as_str().unwrap();
    assert_eq!(
        raw.matches("\"url\"").count(),
        1,
        "转过去的没有重复的键：{raw}"
    );
    assert_eq!(reply["received"]["data"]["url"], proxy);
    assert!(!raw.contains("attacker"), "{raw}");

    let (status, headers, _) = set(body(proxy, outside)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "检查的是留下来的那一个");
    assert_eq!(error_kind(&headers), Some("permission_denied"));
}
