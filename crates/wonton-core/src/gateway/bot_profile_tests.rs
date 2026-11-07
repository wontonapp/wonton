use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode, header};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{StateAvatars, bot_profile_path};

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN};
use super::tunnel_tests::Phone;
use super::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::plugin::PluginId;
use crate::plugin::bot_profile::tests::Bed;
use crate::plugin::languages::Languages;
use crate::tunnel::TunnelEntry;

struct Home {
    bed: Bed,
    app: Router,
    gateway: Arc<Gateway>,
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
            app: router(gateway.clone()),
            bed,
            gateway,
            peer,
        }
    }

    async fn put(&self, id: &str, body: Value) -> (StatusCode, Value) {
        self.request(id, body, Some(self.peer), &[]).await
    }

    async fn request(
        &self,
        id: &str,
        body: Value,
        peer: Option<TunnelPeer>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(Method::PUT)
            .uri(bot_profile_path(id))
            .header(header::HOST, CORE)
            .header(header::CONTENT_TYPE, "application/json");
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(peer);
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let is_json = response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value == "application/json");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = if is_json {
            serde_json::from_slice(&body).unwrap()
        } else {
            Value::String(String::from_utf8(body.to_vec()).unwrap())
        };
        (status, body)
    }
}

#[tokio::test]
async fn owner_can_rename_replace_static_avatar_and_clear_it() {
    let h = Home::start().await;
    let png = include_bytes!("../../tests/fixtures/avatars/square.png");
    let url = h.bed.media(png).await;
    let (status, response) = h
        .put(
            "app.echo",
            json!({
                "name": "  小云  ", "avatar": {"action":"set", "url":url},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["profile_rev"], 1);
    assert_eq!(
        h.bed
            .fake
            .state()
            .user("@echo:test")
            .display_name
            .as_deref(),
        Some("小云")
    );
    assert_eq!(
        h.bed.fake.state().user("@echo:test").avatar_url.as_deref(),
        Some(url.as_str())
    );
    let (status, response) = h
        .put("app.echo", json!({"avatar":{"action":"clear"}}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["profile_rev"], 2);
    assert!(h.bed.fake.state().user("@echo:test").avatar_url.is_none());
}

#[tokio::test]
async fn changing_one_state_preserves_other_and_unknown_keys_and_clear_keeps_field() {
    let h = Home::start().await;
    let png = include_bytes!("../../tests/fixtures/avatars/square.png");
    let url = h.bed.media(png).await;
    let mut media = crate::bot_media::inspect(png).unwrap();
    media.url = url.clone();
    let original = StateAvatars {
        states: [
            ("idle".to_owned(), media.clone()),
            ("future_state".to_owned(), media.clone()),
        ]
        .into(),
    };
    h.bed
        .matrix
        .as_bot(
            "@echo:test",
            async |session: &crate::matrix::Session<'_>| session.set_avatars(&original).await,
        )
        .await
        .unwrap();
    assert_eq!(
        h.put(
            "app.echo",
            json!({"states":{"working":{"action":"set","url":url}}})
        )
        .await
        .0,
        StatusCode::OK
    );
    let profile = h.bed.manager.bot_profile(&h.bed.id).await.unwrap();
    assert_eq!(profile.avatars.states["idle"], media);
    assert_eq!(profile.avatars.states["future_state"], media);
    assert_eq!(profile.avatars.states["working"], media);
    assert_eq!(
        h.put(
            "app.echo",
            json!({"states":{"working":{"action":"clear"},"idle":{"action":"clear"}}})
        )
        .await
        .0,
        StatusCode::OK
    );
    let profile = h.bed.manager.bot_profile(&h.bed.id).await.unwrap();
    assert_eq!(
        profile.avatars.states.keys().collect::<Vec<_>>(),
        ["future_state"]
    );
    h.bed
        .matrix
        .as_bot(
            "@echo:test",
            async |session: &crate::matrix::Session<'_>| {
                session.set_avatars(&StateAvatars::default()).await
            },
        )
        .await
        .unwrap();
    assert_eq!(
        h.put("app.echo", json!({"states":{"idle":{"action":"clear"}}}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        h.bed.fake.state().user("@echo:test").avatars,
        Some(json!({"states":{}}))
    );
}

#[tokio::test]
async fn rejected_names_media_missing_bot_and_matrix_have_distinct_errors() {
    let h = Home::start().await;
    for (name, problem) in [
        (" ", "empty"),
        ("wOnToN", "reserved"),
        ("云吞", "reserved"),
        ("雲吞", "reserved"),
        ("主人", "reserved"),
        ("a\nb", "control_characters"),
    ] {
        let (status, body) = h.put("app.echo", json!({"name":name})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["kind"], "invalid_input");
        assert_eq!(body["error"]["problem"], problem);
    }
    let raw = h.bed.media(b"not image").await;
    let (status, body) = h
        .put(
            "app.echo",
            json!({"name":"not applied", "avatar":{"action":"set","url":raw}}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["kind"], "media_rejected");
    assert_eq!(body["error"]["problem"], "format");
    let (status, body) = h.put("app.absent", json!({"name":"new"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["kind"], "not_found");
    h.bed.manager.register_for_test(
        crate::plugin::PluginPackage::manifest_of(
            super::tests::WHOAMI,
            &Languages::from_header(Some("zh-Hans")),
        )
        .unwrap(),
        crate::plugin::PluginStatus::Running,
        None,
    );
    let (status, body) = h.put("app.whoami", json!({"name":"new"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["kind"], "not_found");
    h.bed.fake.state().refuse_profile = true;
    let (status, body) = h.put("app.echo", json!({"name":"new"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["kind"], "matrix_unavailable");
    assert_eq!(h.bed.manager.profile_revisions().get("app.echo"), 0);
}

#[tokio::test]
async fn only_a_paired_tunnel_device_outside_a_plugin_page_can_edit_profiles() {
    let h = Home::start().await;
    let (status, _) = h
        .request(
            "app.echo",
            json!({"name":"new"}),
            None,
            &[("authorization", &format!("Bearer {TOKEN}"))],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = h
        .request(
            "app.echo",
            json!({"name":"new"}),
            Some(h.peer),
            &[("origin", "http://other.invalid")],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    h.gateway.tunnel.devices.remove(&h.peer.0).unwrap();
    assert_eq!(
        h.put("app.echo", json!({"name":"new"})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.bed
            .fake
            .state()
            .user("@echo:test")
            .display_name
            .as_deref(),
        Some("Echo")
    );
}

#[tokio::test]
async fn owner_profile_edit_crosses_the_real_tunnel_and_local_proxy() {
    let h = Home::start().await;
    let key = SecretKey::generate();
    h.gateway.tunnel.devices.add(key.public(), None).unwrap();
    let _entry = TunnelEntry::spawn(&h.gateway.tunnel, h.app.clone());
    let phone = Phone::connect(key, &h.gateway.tunnel).await;
    let request = phone
        .request(Method::PUT, CORE, &bot_profile_path("app.echo"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"name":"隧道小云"}).to_string()))
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["profile_rev"], 1);
    assert_eq!(
        h.bed
            .fake
            .state()
            .user("@echo:test")
            .display_name
            .as_deref(),
        Some("隧道小云")
    );
    let id = PluginId::parse("app.echo").unwrap();
    assert_eq!(
        h.bed
            .manager
            .bot_profile(&id)
            .await
            .unwrap()
            .name
            .as_deref(),
        Some("隧道小云")
    );
}

#[tokio::test]
async fn owner_name_reservation_uses_the_latest_profile_case_insensitively() {
    let h = Home::start().await;
    h.gateway
        .owner
        .update(&wonton_protocol::OwnerProfileUpdate {
            display_name: "Alice".into(),
            email: None,
        })
        .unwrap();
    let (status, body) = h.put("app.echo", json!({"name":" aLiCe "})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["problem"], "reserved");
    let (status, body) = h.put("app.echo", json!({"name":"云".repeat(33)})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["problem"], "too_long");
    assert_eq!(
        h.bed
            .fake
            .state()
            .user("@echo:test")
            .display_name
            .as_deref(),
        Some("Echo")
    );
}

#[tokio::test]
async fn invalid_poster_or_animated_static_avatar_rejects_the_entire_update() {
    let h = Home::start().await;
    let video = h
        .bed
        .media(include_bytes!("../../tests/fixtures/avatars/h264.mp4"))
        .await;
    let bad_poster = h
        .bed
        .media(include_bytes!("../../tests/fixtures/avatars/rectangle.png"))
        .await;
    let gif = h
        .bed
        .media(include_bytes!("../../tests/fixtures/avatars/animated.gif"))
        .await;
    let (status, body) = h
        .put(
            "app.echo",
            json!({"name":"not applied", "states":{"idle":{
                "action":"set", "url":video, "poster":bad_poster,
            }}}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["problem"], "not_square");
    let (status, body) = h
        .put("app.echo", json!({"avatar":{"action":"set","url":gif}}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["problem"], "format");
    let (status, body) = h
        .put(
            "app.echo",
            json!({"avatar":{"action":"set","url":"https://invalid.example/avatar"}}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["problem"], "format");
    assert_eq!(
        h.bed
            .fake
            .state()
            .user("@echo:test")
            .display_name
            .as_deref(),
        Some("Echo")
    );
    assert!(h.bed.fake.state().user("@echo:test").avatars.is_none());
    assert!(h.bed.fake.state().user("@echo:test").avatar_url.is_none());
    assert_eq!(h.bed.manager.profile_revisions().get("app.echo"), 0);
}
