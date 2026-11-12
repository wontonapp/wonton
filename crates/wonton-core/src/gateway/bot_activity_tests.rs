//! 活动接口走真实的网关与隧道，安装和身份服务用既有 Docker、Matrix 测试替身。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{StatusCode, header};
use iroh::SecretKey;
use serde_json::json;
use tower::ServiceExt;
use wonton_protocol::{
    Answer, BOT_ACTIVITY_PATH, BOT_ACTIVITY_WAIT_SECS, BotActivityReport, BotProfileUpdate,
};

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use super::tunnel_tests::Phone;
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::matrix::{Matrix, fake};
use crate::mcp::test_support::Cards;
use crate::mcp::{Bus, Confirmations};
use crate::owner::OwnerStore;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginId, PluginManager, PluginPackage, PluginStatus};
use crate::tunnel::{Tunnel, TunnelEntry};

const BUS_TOKEN: &str = "activity-token";

const BOT: &str = "services:
  bot:
    image: example/bot
x-wonton:
  spec: '0.4'
  id: app.activity
  version: 1.0.0
  kind: app
  title: {zh: Bot, en: Bot}
  description: {zh: Bot, en: Bot}
  permissions: [matrix.bot, tools]
";

struct Home {
    gateway: Arc<Gateway>,
    app: Router,
    peer: TunnelPeer,
    cards: Arc<Cards>,
    docker: FakeDocker,
    _entry: TunnelEntry,
    _dir: tempfile::TempDir,
}

impl Home {
    async fn start(ttl: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (docker, engine) = FakeDocker::start().await;
        let (_matrix, cfg) = fake::start().await;
        let http = HttpClient::new();
        let matrix = Arc::new(
            Matrix::connect(&cfg, dir.path(), http.clone())
                .await
                .unwrap(),
        );
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
            relay_dir: dir.path().join("relay"),
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
        let plugins = Arc::new(
            PluginManager::new(
                engine,
                Store::open(dir.path().join("plugins")).unwrap(),
                http.clone(),
                None,
                lifecycle,
                ImageWorker::in_process(),
            )
            .with_matrix(Some(matrix)),
        );
        // 有 Bot 记录的 Agent 必须用带总线令牌的地址；先记一个固定令牌的 Bot，装的时候沿用。
        plugins.bot_records_for_test(
            &PluginId::parse("app.activity").unwrap(),
            &[(
                "default",
                "@activity:test",
                wonton_protocol::BotKeeper::User {
                    username: "owner".to_owned(),
                },
                BUS_TOKEN,
            )],
        );
        plugins
            .install(
                BOT,
                &[Permission::MatrixBot, Permission::Tools],
                Languages::from_header(Some("zh-Hans")),
            )
            .await
            .unwrap();
        let cards = Arc::new(Cards::default());
        let bus = Bus::new(
            plugins.clone(),
            Arc::new(Confirmations::new(cards.clone(), ttl)),
            Arc::new(crate::mcp::audit::Audit::new(
                dir.path().join("audit.jsonl"),
            )),
        );
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let peer = TunnelPeer(SecretKey::generate().public());
        tunnel.devices.add(peer.0, None).unwrap();
        let gateway = Arc::new(
            Gateway::new(
                public,
                profile,
                TOKEN.into(),
                members,
                plugins,
                None,
                http,
                tunnel,
            )
            .with_mcp(Some(bus.endpoint())),
        );
        let app = router(gateway.clone());
        let entry = TunnelEntry::spawn(&gateway.tunnel, app.clone());
        Self {
            gateway,
            app,
            peer,
            cards,
            docker,
            _entry: entry,
            _dir: dir,
        }
    }

    fn request(&self, after: Option<u64>, peer: Option<TunnelPeer>) -> hyper::Request<Body> {
        let path = after.map_or_else(
            || BOT_ACTIVITY_PATH.to_owned(),
            |rev| format!("{BOT_ACTIVITY_PATH}?after={rev}"),
        );
        let mut req = hyper::Request::builder()
            .uri(path)
            .header(header::HOST, "wonton.localhost:18480")
            .body(Body::empty())
            .unwrap();
        if let Some(peer) = peer {
            req.extensions_mut().insert(peer);
        }
        req
    }

    async fn report(&self, after: Option<u64>) -> BotActivityReport {
        report(
            self.app
                .clone()
                .oneshot(self.request(after, Some(self.peer)))
                .await
                .unwrap(),
        )
        .await
    }

    fn tunnel(&self) -> &Tunnel {
        &self.gateway.tunnel
    }

    fn ask(&self) -> tokio::task::JoinHandle<axum::response::Response> {
        let app = self.app.clone();
        self.gateway.plugins.register_source_for_test(
            "app.activity",
            "127.0.0.0/8",
            vec![Permission::Tools],
        );
        let mut request = hyper::Request::builder()
            .method("POST")
            .uri(format!("{}/{BUS_TOKEN}", wonton_protocol::AGENT_PERMISSION_PATH))
            .header(header::HOST, "mcp.wonton.internal:8080")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"tool_call_id": "call-1", "title": "Run a tool", "options": [{"choice_id": "once", "kind": "allow_once", "name": "Allow"}]}).to_string())).unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:18080".parse::<std::net::SocketAddr>().unwrap(),
        ));
        tokio::spawn(async move { app.oneshot(request).await.unwrap() })
    }
}

async fn report(response: axum::response::Response) -> BotActivityReport {
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

fn id() -> PluginId {
    PluginId::parse("app.activity").unwrap()
}

fn langs() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

#[tokio::test]
async fn only_paired_tunnel_devices_can_read_activity() {
    let h = Home::start(Duration::from_secs(60)).await;
    for authorization in [None, Some(format!("Bearer {TOKEN}"))] {
        let mut request = h.request(None, None);
        if let Some(authorization) = authorization {
            request
                .headers_mut()
                .insert(header::AUTHORIZATION, authorization.parse().unwrap());
        }
        assert_eq!(
            h.app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let stranger = TunnelPeer(SecretKey::generate().public());
    assert_eq!(
        h.app
            .clone()
            .oneshot(h.request(None, Some(stranger)))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut cross_site = h.request(None, Some(h.peer));
    cross_site.headers_mut().insert(
        header::ORIGIN,
        "http://app-activity.wonton.localhost:18480"
            .parse()
            .unwrap(),
    );
    assert_eq!(
        h.app.clone().oneshot(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let sid = h
        .gateway
        .sessions
        .create_core(&super::Person::owner("owner"));
    let mut cookie_request = h.request(None, None);
    cookie_request.headers_mut().insert(
        header::COOKIE,
        format!("{}={sid}", super::CORE_COOKIE).parse().unwrap(),
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(cookie_request)
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED,
    );
    let observed = h.report(None).await;
    assert_eq!(observed.bots[0].user_id, "@activity:test");
    h.tunnel().devices.remove(&h.peer.0).unwrap();
    assert_eq!(
        h.app
            .clone()
            .oneshot(h.request(None, Some(h.peer)))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_current_revision_waits_and_a_counter_change_releases_it() {
    let h = Home::start(Duration::from_secs(60)).await;
    let initial = h.report(None).await;
    let waiting = h.report(Some(initial.rev));
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), waiting.as_mut())
            .await
            .is_err()
    );
    let active = h.gateway.plugins.activity.start_model(&id());
    let changed = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap();
    assert!(changed.rev > initial.rev);
    assert_eq!(changed.bots[0].model_requests, 1);
    drop(active);
    for after in [initial.rev, u64::MAX] {
        let current = tokio::time::timeout(Duration::from_secs(2), h.report(Some(after)))
            .await
            .unwrap();
        assert_eq!(current.bots[0].model_requests, 0);
        assert!(current.rev > changed.rev);
    }
}

#[tokio::test]
async fn unchanged_activity_returns_when_the_long_poll_limit_expires() {
    let h = Home::start(Duration::from_secs(60)).await;
    let initial = h.report(None).await;
    let start = tokio::time::Instant::now();
    let current = tokio::time::timeout(
        Duration::from_secs(BOT_ACTIVITY_WAIT_SECS + 3),
        h.report(Some(initial.rev)),
    )
    .await
    .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(BOT_ACTIVITY_WAIT_SECS));
    assert_eq!(current, initial);
}

#[tokio::test]
async fn plugin_health_and_stopping_change_running_and_revision() {
    let h = Home::start(Duration::from_secs(60)).await;
    let initial = h.report(None).await;
    assert!(initial.bots[0].running);
    for state in [
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
    ] {
        let before = h.report(None).await;
        h.docker.set_state("wonton-app-activity--bot", state);
        h.gateway.plugins.restore().await.unwrap();
        let snapshot = h.report(Some(before.rev)).await;
        assert!(!snapshot.bots[0].running);
        assert!(snapshot.rev > before.rev);
    }
}

#[tokio::test]
async fn snapshots_only_include_bot_manifests_with_records() {
    let h = Home::start(Duration::from_secs(60)).await;
    let missing = PluginPackage::parse(&BOT.replace("app.activity", "app.missing"), &langs())
        .unwrap()
        .manifest;
    h.gateway
        .plugins
        .register_for_test(missing, PluginStatus::Running, None);
    let plain = PluginPackage::parse(
        &BOT.replace("app.activity", "app.plain")
            .replace("[matrix.bot, tools]", "[]"),
        &langs(),
    )
    .unwrap()
    .manifest;
    h.gateway
        .plugins
        .register_for_test(plain, PluginStatus::Running, None);
    let snapshot = h.report(None).await;
    assert_eq!(
        snapshot
            .bots
            .iter()
            .map(|bot| bot.plugin_id.as_str())
            .collect::<Vec<_>>(),
        ["app.activity"]
    );
}

#[tokio::test]
async fn activity_snapshots_reach_the_owner_over_a_real_tunnel() {
    let h = Home::start(Duration::from_secs(60)).await;
    let device = SecretKey::generate();
    h.tunnel().devices.add(device.public(), None).unwrap();
    let phone = Phone::connect(device, h.tunnel()).await;
    let (status, _, body) = phone.get(CORE, BOT_ACTIVITY_PATH).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: BotActivityReport = serde_json::from_str(&body).unwrap();
    assert!(snapshot.bots[0].running);
    assert_eq!(snapshot.bots[0].user_id, "@activity:test");
    let active = h.gateway.plugins.activity.start_tool(&id());
    let path = format!("{BOT_ACTIVITY_PATH}?after={}", snapshot.rev);
    let (status, _, body) = phone.get(CORE, &path).await;
    assert_eq!(status, StatusCode::OK);
    let active_report: BotActivityReport = serde_json::from_str(&body).unwrap();
    assert_eq!(active_report.bots[0].tool_calls, 1);
    drop(active);
}

#[tokio::test]
async fn acp_confirmation_waits_belong_to_the_caller_and_wake_long_polls() {
    let h = Home::start(Duration::from_secs(60)).await;
    let initial = h.report(None).await;
    let asked = h.ask();
    let card = h.cards.wait_for(1).await;
    assert_eq!(card.plugin_id, None);
    let waiting = h.report(Some(initial.rev)).await;
    assert_eq!(
        (waiting.bots[0].waiting, waiting.bots[0].tool_calls),
        (1, 0)
    );
    assert!(waiting.rev > initial.rev);
    let pending = h.report(Some(waiting.rev));
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), pending.as_mut())
            .await
            .is_err()
    );
    h.gateway
        .mcp
        .as_ref()
        .unwrap()
        .bus
        .answer_card(&card.id, Answer::Approve, Some("once"), "owner")
        .unwrap();
    let answered = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap();
    assert_eq!(
        (answered.bots[0].waiting, answered.bots[0].tool_calls),
        (0, 0)
    );
    assert!(answered.rev > waiting.rev);
    assert_eq!(asked.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn expiry_and_acp_disconnect_clear_waiting_and_wake_pollers() {
    let h = Home::start(Duration::from_millis(300)).await;
    let asked = h.ask();
    h.cards.wait_for(1).await;
    let waiting = h.report(None).await;
    assert_eq!(waiting.bots[0].waiting, 1);
    let expired = tokio::time::timeout(Duration::from_secs(2), h.report(Some(waiting.rev)))
        .await
        .unwrap();
    assert_eq!(expired.bots[0].waiting, 0);
    assert!(expired.rev > waiting.rev);
    assert_eq!(asked.await.unwrap().status(), StatusCode::OK);
    let asked = h.ask();
    h.cards.wait_for(2).await;
    let waiting = h.report(None).await;
    assert_eq!(waiting.bots[0].waiting, 1);
    asked.abort();
    let _ = asked.await;
    let withdrawn = tokio::time::timeout(Duration::from_secs(2), h.report(Some(waiting.rev)))
        .await
        .unwrap();
    assert_eq!(withdrawn.bots[0].waiting, 0);
    assert!(withdrawn.rev > waiting.rev);
}

#[tokio::test]
async fn corrupt_bot_records_fail_instead_of_reporting_an_empty_snapshot() {
    let h = Home::start(Duration::from_secs(60)).await;
    let path = h
        .gateway
        .plugins
        .data_dir()
        .join("plugins/bots/app.activity.json");
    std::fs::write(path, "not json").unwrap();
    let response = h
        .app
        .clone()
        .oneshot(h.request(None, Some(h.peer)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()["x-wonton-error"], "storage");
}

#[tokio::test]
async fn malformed_revisions_do_not_start_long_polls() {
    let h = Home::start(Duration::from_secs(60)).await;
    for after in ["-1", "18446744073709551616", "invalid"] {
        let mut request = h.request(None, Some(h.peer));
        *request.uri_mut() = format!("{BOT_ACTIVITY_PATH}?after={after}")
            .parse()
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), h.app.clone().oneshot(request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn unsent_confirmation_cards_do_not_leave_the_bot_waiting() {
    let h = Home::start(Duration::from_secs(60)).await;
    h.cards
        .broken
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let before = h.report(None).await;
    assert_eq!(
        h.ask().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let after = h.report(None).await;
    assert_eq!((after.bots[0].waiting, after.bots[0].tool_calls), (0, 0));
    assert!(after.rev > before.rev);
}

#[tokio::test]
async fn profile_update_advances_profile_rev_and_wakes_long_polls() {
    let h = Home::start(Duration::from_secs(60)).await;
    let before = h.report(None).await;
    assert_eq!(before.bots[0].profile_rev, 0);

    let app = h.app.clone();
    let request = h.request(Some(before.rev), Some(h.peer));
    let pending = tokio::spawn(async move { report(app.oneshot(request).await.unwrap()).await });

    let updated = h
        .gateway
        .plugins
        .update_bot_profile(
            &id(),
            BotProfileUpdate {
                name: Some("新的名字".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.profile_rev, 1);

    let woken = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(woken.bots[0].profile_rev, 1);
    assert!(woken.rev > before.rev);

    let fresh = h.report(None).await;
    assert_eq!(fresh.bots[0].profile_rev, 1);
}
