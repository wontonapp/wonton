//! 家人网关规则的行为测试（ADR-0045 的“网关管房间规则”“频率和跳数”）：公开房间不放行、Bot 发消息的频率和
//! 跳数。网关后面接假的 Tuwunel（`crate::matrix::fake`），时钟是注入的，不许 sleep。

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use iroh::{EndpointId, SecretKey};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fakes::loopback_tunnel;
use super::send_limits::{Clock, SendLimiter, SendLimits};
use super::tests::{CORE, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::config::{Config, PublicUrls};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::Room;
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginManager};
use crate::tunnel::Tunnel;

const MATRIX_HOST: &str = "matrix.wonton.localhost";
const BOT_HOST: &str = "matrix.wonton.internal";
/// 插件网络里的来源：注册成有 `matrix.bot` 权限的插件。
const BOT: &str = "172.30.1.5:40000";
const BOT_TOKEN: &str = "bot-token";
const DEVICE_TOKEN: &str = "device-token";
const ROOM: &str = "!r:test";

struct Home {
    app: Router,
    fake: crate::matrix::fake::Fake,
    limiter: Arc<SendLimiter>,
    clock: Arc<AtomicU64>,
    phone: EndpointId,
    _tunnel: Tunnel,
    _dir: tempfile::TempDir,
}

impl Home {
    /// 核心接着假的 Tuwunel；一个插件带 `matrix.bot` 权限，一台设备经隧道接入。
    async fn start(limits: SendLimits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (fake, config) = crate::matrix::fake::start().await;
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
            install_timeout: std::time::Duration::from_secs(5),
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
        plugins.register_source_for_test(
            "app.echo",
            "172.30.1.0/24",
            vec!["matrix.bot".parse::<Permission>().unwrap()],
        );
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let phone = SecretKey::generate().public();
        tunnel.devices.add(phone, None).unwrap();
        // Bot 和设备的访问令牌：假 Tuwunel 的 `whoami` 认它们。
        fake.state().tokens.insert(
            BOT_TOKEN.into(),
            ("@hermes:test".into(), "bot-device".into()),
        );
        fake.state().tokens.insert(
            DEVICE_TOKEN.into(),
            ("@owner:test".into(), "phone-device".into()),
        );
        let clock = Arc::new(AtomicU64::new(1_000));
        let now: Clock = {
            let clock = clock.clone();
            Arc::new(move || clock.load(Ordering::Relaxed))
        };
        let limiter = Arc::new(SendLimiter::with_clock(limits, now));
        let gateway = Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members,
            plugins,
            None,
            http,
            tunnel.clone(),
        )
        .with_matrix(Some(Arc::new(matrix)))
        .with_send_limiter(limiter.clone());
        Self {
            app: router(Arc::new(gateway)),
            fake,
            limiter,
            clock,
            phone,
            _tunnel: tunnel,
            _dir: dir,
        }
    }

    fn tick(&self, ms: u64) {
        self.clock.fetch_add(ms, Ordering::Relaxed);
    }

    /// 建一个 Bot 和人都在的房间，省得上游把消息拒在门外。
    fn seed_room(&self, room: &str) {
        self.fake.state().rooms.insert(
            room.to_owned(),
            Room {
                members: BTreeSet::from(["@hermes:test".to_owned(), "@owner:test".to_owned()]),
                ..Room::default()
            },
        );
    }

    #[allow(clippy::too_many_arguments)] // 测试替身的请求参数就这几样，再包一层也是逐个填。
    async fn send(
        &self,
        method: Method,
        host: &str,
        path: &str,
        peer: Option<EndpointId>,
        from: &str,
        token: Option<&str>,
        body: String,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut builder = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.len());
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let mut request = builder.body(Body::from(body)).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(from.parse::<SocketAddr>().unwrap()));
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

    /// 设备（人）的请求，走 `matrix.<base>`。
    async fn device(
        &self,
        method: Method,
        path: &str,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        self.send(
            method,
            MATRIX_HOST,
            path,
            Some(self.phone),
            "127.0.0.1:50000",
            Some(DEVICE_TOKEN),
            body.to_string(),
        )
        .await
    }

    /// Bot 的请求，走 `matrix.wonton.internal`。
    async fn bot(&self, method: Method, path: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
        self.send(
            method,
            BOT_HOST,
            path,
            None,
            BOT,
            Some(BOT_TOKEN),
            body.to_string(),
        )
        .await
    }

    /// Bot 在房间里说一句话。
    async fn bot_says(&self, room: &str, txn: &str) -> (StatusCode, HeaderMap, Value) {
        self.bot(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}"),
            json!({"msgtype": "m.text", "body": "hi"}),
        )
        .await
    }

    fn refusal<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        headers
            .get("x-wonton-error")
            .and_then(|value| value.to_str().ok())
    }
}

#[tokio::test]
async fn public_create_room_is_refused_on_both_entries() {
    let home = Home::start(SendLimits::default()).await;
    for (as_device, from) in [(true, "设备"), (false, "Bot")] {
        for body in [
            json!({"visibility": "public"}),
            json!({"preset": "public_chat"}),
            json!({"initial_state": [
                {"type": "m.room.join_rules", "state_key": "", "content": {"join_rule": "public"}}
            ]}),
        ] {
            let (status, headers, responded) = if as_device {
                home.device(Method::POST, "/_matrix/client/v3/createRoom", body)
                    .await
            } else {
                home.bot(Method::POST, "/_matrix/client/v3/createRoom", body)
                    .await
            };
            assert_eq!(status, StatusCode::FORBIDDEN, "{from} {responded}");
            assert_eq!(responded["errcode"], "M_FORBIDDEN", "{from}");
            assert_eq!(home.refusal(&headers), Some("public_room"), "{from}");
            assert!(
                home.fake.state().rooms.is_empty(),
                "{from}：上游不该收到请求"
            );
        }
    }
}

#[tokio::test]
async fn private_create_room_is_forwarded() {
    let home = Home::start(SendLimits::default()).await;
    let (status, _, body) = home
        .device(
            Method::POST,
            "/_matrix/client/v3/createRoom",
            json!({"visibility": "private", "preset": "private_chat", "name": "家里的房间"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["room_id"]
            .as_str()
            .is_some_and(|id| id.starts_with('!'))
    );
    assert_eq!(home.fake.state().rooms.len(), 1, "上游建了房间");
}

#[tokio::test]
async fn making_the_join_rules_public_is_refused_but_other_rules_pass() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    let (status, headers, body) = home
        .bot(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{ROOM}/state/m.room.join_rules"),
            json!({"join_rule": "public"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(home.refusal(&headers), Some("public_room"));
    // 编码过的写法也认得。
    let (status, headers, _) = home
        .bot(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{ROOM}/state/m%2eroom%2ejoin%5frules/"),
            json!({"join_rule": "public"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(home.refusal(&headers), Some("public_room"));
    // 别的 join_rules 照常转给上游（假 Tuwunel 没有这个路由，回显收到的路径）。
    let (status, _, body) = home
        .bot(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{ROOM}/state/m.room.join_rules"),
            json!({"join_rule": "invite"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["path"],
        format!("/_matrix/client/v3/rooms/{ROOM}/state/m.room.join_rules")
    );
    // 带非空状态键的是另一条状态事件，改不到房间的 join_rules，照常转发。
    let path = format!("/_matrix/client/v3/rooms/{ROOM}/state/m.room.join_rules/extra");
    let (status, _, body) = home
        .bot(Method::PUT, &path, json!({"join_rule": "public"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["path"], path, "转发到上游了");
}

#[tokio::test]
async fn an_oversized_or_non_json_body_is_not_forwarded() {
    let home = Home::start(SendLimits::default()).await;
    let (status, _, body) = home
        .device(
            Method::POST,
            "/_matrix/client/v3/createRoom",
            json!({"name": "x".repeat(300 * 1024)}),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["errcode"], "M_TOO_LARGE");
    let (status, _, body) = home
        .send(
            Method::POST,
            MATRIX_HOST,
            "/_matrix/client/v3/createRoom",
            Some(home.phone),
            "127.0.0.1:50000",
            Some(DEVICE_TOKEN),
            "不是 JSON".into(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["errcode"], "M_BAD_JSON");
}

#[tokio::test]
async fn the_bot_bursts_and_then_gets_limited() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    for txn in 0..10 {
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(home.fake.state().sent.len(), 10, "十条都到了上游");
    let (status, _, body) = home.bot_says(ROOM, "t10").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["errcode"], "M_LIMIT_EXCEEDED");
    let retry_after_ms = body["retry_after_ms"]
        .as_u64()
        .expect("要带 retry_after_ms");
    assert!(
        0 < retry_after_ms && retry_after_ms <= 3000,
        "每分钟 20 条，等回一个令牌最多 3 秒：{retry_after_ms}"
    );
    assert_eq!(home.fake.state().sent.len(), 10, "挡下的没到上游");
}

#[tokio::test]
async fn another_room_is_not_affected() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    home.seed_room("!other:test");
    for txn in 0..10 {
        home.bot_says(ROOM, &format!("t{txn}")).await;
    }
    let (status, _, body) = home.bot_says("!other:test", "t0").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_same_txn_resend_is_not_counted() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    for _ in 0..2 {
        let (status, _, body) = home.bot_says(ROOM, "t1").await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    for txn in 2..11 {
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // 重发没计数：第 10 条新消息还在桶里，第 11 条新的才限频。
    let (status, _, body) = home.bot_says(ROOM, "t11").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn state_events_and_receipts_are_not_counted() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    for txn in 0..9 {
        home.bot_says(ROOM, &format!("t{txn}")).await;
    }
    // 状态事件和回执照常转发（假 Tuwunel 没有这些路由，回显路径），但不数。
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/_matrix/client/v3/rooms/{ROOM}/state/m.room.topic"),
            json!({"topic": "今天吃什么"}),
        ),
        (
            Method::POST,
            format!("/_matrix/client/v3/rooms/{ROOM}/receipt/m.read/$event0"),
            json!({}),
        ),
        (
            Method::PUT,
            format!("/_matrix/client/v3/rooms/{ROOM}/typing/@hermes:test"),
            json!({"typing": true}),
        ),
        (
            Method::PUT,
            "/_matrix/client/v3/sendToDevice/m.room.encrypted/txn1".to_owned(),
            json!({"messages": {}}),
        ),
    ] {
        let (status, _, echoed) = home.bot(method, &path, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{echoed}");
        assert_eq!(echoed["path"], path, "转发到上游了");
    }
    let (status, _, body) = home.bot_says(ROOM, "t9").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = home.bot_says(ROOM, "t10").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn the_hop_limit_stops_the_loop_until_someone_speaks() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    let mut events = home.limiter.subscribe();
    // 每条隔 3 秒：频率不挡，跳数到 20 条为止。
    for txn in 0..20 {
        home.tick(3000);
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    home.tick(3000);
    let (status, headers, body) = home.bot_says(ROOM, "t20").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_FORBIDDEN");
    assert_eq!(home.refusal(&headers), Some("hop_limit"));
    assert_eq!(home.fake.state().sent.len(), 20, "第 21 条没到上游");

    // 订阅的任务收到事件，Bot 的名字是 whoami 认出来的。
    let event = events.recv().await.expect("跳数到顶的事件");
    assert_eq!(event.room_id, ROOM);
    assert_eq!(event.bot_user_ids, vec!["@hermes:test"]);

    // 人说一句就清零：Bot 又能连着发 20 条。
    home.tick(3000);
    let (status, _, body) = home
        .device(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{ROOM}/send/m.room.message/from-human"),
            json!({"msgtype": "m.text", "body": "我回来了"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for txn in 21..41 {
        home.tick(3000);
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    home.tick(3000);
    let (status, headers, _) = home.bot_says(ROOM, "t41").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(home.refusal(&headers), Some("hop_limit"));
}

#[tokio::test]
async fn an_hour_without_messages_starts_the_room_over() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    for txn in 0..20 {
        home.tick(3000);
        home.bot_says(ROOM, &format!("t{txn}")).await;
    }
    home.tick(3000);
    let (status, _, _) = home.bot_says(ROOM, "t20").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // 一小时没有任何消息：下一条从头数。
    home.tick(3_600_000);
    let (status, _, body) = home.bot_says(ROOM, "t21").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_refused_message_does_not_consume_a_rate_token() {
    let home = Home::start(SendLimits::default()).await;
    home.seed_room(ROOM);
    // 十条突发把桶打空，跳数到 10。
    for txn in 0..10 {
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // 每条隔 3 秒补回一个令牌又用掉：桶一直是空的，跳数到 20。
    for txn in 10..20 {
        home.tick(3000);
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    home.tick(3000);
    let (status, headers, _) = home.bot_says(ROOM, "t20").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(home.refusal(&headers), Some("hop_limit"));
    // 人说一句，Bot 不挪时钟立刻能发：被跳数拒掉的那条没占频率的令牌。
    let (status, _, body) = home
        .device(
            Method::PUT,
            &format!("/_matrix/client/v3/rooms/{ROOM}/send/m.room.message/from-human"),
            json!({"msgtype": "m.text", "body": "我在"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = home.bot_says(ROOM, "t21").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_configured_numbers_are_enforced() {
    let home = Home::start(SendLimits {
        per_minute: 60,
        burst: 30,
        hop_limit: 5,
        idle_reset: std::time::Duration::from_secs(60),
    })
    .await;
    home.seed_room(ROOM);
    for txn in 0..5 {
        home.tick(1000);
        let (status, _, body) = home.bot_says(ROOM, &format!("t{txn}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    home.tick(1000);
    let (status, headers, body) = home.bot_says(ROOM, "t5").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        home.refusal(&headers),
        Some("hop_limit"),
        "跳数上限用的是配置里的 5"
    );
}

#[test]
fn the_bot_message_numbers_come_from_the_environment() {
    let configured = |pairs: &[(&str, &str)]| {
        let mut settings = vec![("WONTON_DATA_DIR".to_owned(), "/d".to_owned())];
        settings.extend(
            pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string())),
        );
        Config::from_lookup(move |name| {
            settings
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        })
    };
    let limits = configured(&[
        ("WONTON_BOT_MSG_PER_MINUTE", "60"),
        ("WONTON_BOT_MSG_BURST", "30"),
        ("WONTON_BOT_MSG_HOP_LIMIT", "5"),
        ("WONTON_BOT_MSG_IDLE_RESET_SECS", "60"),
    ])
    .unwrap()
    .send_limits;
    assert_eq!(limits.per_minute, 60);
    assert_eq!(limits.burst, 30);
    assert_eq!(limits.hop_limit, 5);
    assert_eq!(limits.idle_reset, std::time::Duration::from_secs(60));
    // 默认值按 ADR-0045；0 没有意义：要么限要么不限，不设开关。
    assert_eq!(configured(&[]).unwrap().send_limits, SendLimits::default());
    for name in [
        "WONTON_BOT_MSG_PER_MINUTE",
        "WONTON_BOT_MSG_BURST",
        "WONTON_BOT_MSG_HOP_LIMIT",
    ] {
        let error = configured(&[(name, "0")]).unwrap_err().to_string();
        assert!(error.contains(name), "{name}：{error}");
    }
}
