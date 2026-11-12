//! 面板接口（#35）：Bot 钉页的列表、移除、标已看、收回同意。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode, header};
use iroh::SecretKey;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tower::ServiceExt;
use wonton_protocol::{
    BOT_PANEL_PINS_PATH, PanelScope, bot_panel_consent_path, bot_panel_pin_path,
    bot_panel_pin_seen_path,
};

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN};
use super::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::plugin::bot_profile::tests::Bed;
use crate::plugin::pins::PanelPin;

struct Home {
    bed: Bed,
    app: Router,
    peer: TunnelPeer,
}

impl Home {
    async fn start() -> Self {
        let bed = Bed::start().await;
        let (tunnel, members) = loopback_tunnel(bed.dir.path()).await;
        let peer = TunnelPeer(SecretKey::generate().public());
        tunnel.devices.add(peer.0, Some("Phone".into())).unwrap();
        let profile = bed.owner.clone();
        let gateway = Arc::new(
            Gateway::new(
                crate::config::PublicUrls {
                    scheme: "http".into(),
                    base_domain: CORE.into(),
                    port: 18480,
                },
                profile,
                TOKEN.into(),
                members,
                bed.manager.clone(),
                None,
                HttpClient::new(),
                tunnel,
            )
            .with_matrix(Some(bed.matrix.clone())),
        );
        Self {
            bed,
            app: router(gateway),
            peer,
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        peer: Option<TunnelPeer>,
        origin: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, CORE)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let body = body
            .map(|body| Body::from(body.to_string()))
            .unwrap_or_default();
        let mut request = request.body(body).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(peer);
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    /// 直接在记录里放一个钉页。
    fn pin(&self, scope: PanelScope, attention: bool) -> String {
        let id = "pin-1".to_owned();
        let plugin = crate::plugin::PluginId::parse("app.echo").unwrap();
        self.bed
            .manager
            .store()
            .pins()
            .with(&plugin, |record| {
                let entry = record.bots.entry("echo".into()).or_default();
                entry.main_consent = true;
                entry.pins.push(PanelPin {
                    id: id.clone(),
                    scope,
                    tab: wonton_protocol::TabRef::Url {
                        url: "https://example.com/".into(),
                    },
                    title: "看板".into(),
                    attention,
                });
            })
            .unwrap();
        id
    }
}

fn main() -> PanelScope {
    PanelScope::Main
}

#[tokio::test]
async fn list_remove_seen_and_consent() {
    let h = Home::start().await;
    let pin = h.pin(main(), true);

    let (status, body) = h
        .request(Method::GET, BOT_PANEL_PINS_PATH, None, Some(h.peer), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pins"][0]["id"], pin);
    assert_eq!(body["pins"][0]["name"], "回声");
    assert_eq!(body["consents"][0]["main_consent"], true);

    let (status, _) = h
        .request(
            Method::POST,
            &bot_panel_pin_seen_path(&pin),
            None,
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let plugin = crate::plugin::PluginId::parse("app.echo").unwrap();
    assert!(!h.bed.manager.store().pins().load(&plugin).unwrap().bots["echo"].pins[0].attention);

    let (status, _) = h
        .request(
            Method::PUT,
            &bot_panel_consent_path("app.echo", "echo"),
            Some(json!({"allowed": false})),
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!h.bed.manager.store().pins().load(&plugin).unwrap().bots["echo"].main_consent);

    let (status, _) = h
        .request(
            Method::DELETE,
            &bot_panel_pin_path(&pin),
            None,
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        h.bed.manager.store().pins().load(&plugin).unwrap().bots["echo"]
            .pins
            .is_empty()
    );
}

#[tokio::test]
async fn unknown_pins_are_404_and_unpaired_or_cross_site_is_rejected() {
    let h = Home::start().await;
    let (status, _) = h
        .request(
            Method::DELETE,
            &bot_panel_pin_path("missing"),
            None,
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let stranger = TunnelPeer(SecretKey::generate().public());
    let (status, _) = h
        .request(Method::GET, BOT_PANEL_PINS_PATH, None, Some(stranger), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .request(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            None,
            Some(h.peer),
            Some("https://elsewhere.example/"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ui_message_reuses_one_owner_device() {
    let h = Home::start().await;
    let pin = h.pin(
        PanelScope::Room {
            room_id: "!room:test".into(),
        },
        false,
    );
    // Bed 里 Bot 的家是 !room:test，但假服务器里还没这个房间：造一个，主人在里面。
    h.bed.fake.state().rooms.insert(
        "!room:test".into(),
        crate::matrix::fake::Room {
            members: BTreeSet::from(["@owner:test".into()]),
            ..Default::default()
        },
    );
    let before = h.bed.fake.state().devices("@owner:test").len();
    let body = |text: &str| json!({"pin": pin, "text": text});
    let (status, why) = h
        .request(
            Method::POST,
            wonton_protocol::MCP_APP_MESSAGE_PATH,
            Some(body("进展如何")),
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{why}");
    let (status, _) = h
        .request(
            Method::POST,
            wonton_protocol::MCP_APP_MESSAGE_PATH,
            Some(body("还差多少")),
            Some(h.peer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let after = h.bed.fake.state().devices("@owner:test").len();
    assert_eq!(after - before, 1, "两条消息只应登一台设备");
}
