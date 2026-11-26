//! `GET SHARED_BOTS_PATH` 的 HTTP 行为：按调用人列出 Bot 和原因，不受 `open_apps` 过滤。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{StatusCode, header};
use iroh::SecretKey;
use tower::ServiceExt;
use wonton_protocol::{
    AppScope, BotAccess, BotChatUnavailable, BotKeeper, RoleDraft, SHARED_BOTS_PATH,
    SharedBotsState, SharedBotsView,
};

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::matrix::{Matrix, fake};
use crate::owner::OwnerStore;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginId, PluginManager};
use crate::tunnel::TunnelEntry;

const PLUGIN: &str = "app.shared";

const MANIFEST: &str = "services:
  bot:
    image: example/bot
x-wonton:
  spec: '0.4'
  id: app.shared
  version: 1.0.0
  kind: app
  title: {zh: 共享助手, en: Shared assistant}
  description: {zh: Bot, en: Bot}
  permissions: [matrix.bot]
";

struct Home {
    gateway: Arc<Gateway>,
    app: Router,
    _entry: TunnelEntry,
    _dir: tempfile::TempDir,
}

impl Home {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (_docker, engine) = FakeDocker::start().await;
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
            .with_matrix(Some(matrix.clone())),
        );
        plugins
            .install(
                MANIFEST,
                &[Permission::MatrixBot],
                Languages::from_header(Some("zh-Hans")),
            )
            .await
            .unwrap();
        plugins.bot_records_for_test(
            &PluginId::parse(PLUGIN).unwrap(),
            &[(
                "owner-private",
                "@private:test",
                BotKeeper::User {
                    username: "owner".to_owned(),
                },
                "private-token",
            )],
        );
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
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
            .with_matrix(Some(matrix)),
        );
        let app = router(gateway.clone());
        let entry = TunnelEntry::spawn(&gateway.tunnel, app.clone());
        Self {
            gateway,
            app,
            _entry: entry,
            _dir: dir,
        }
    }

    fn family_peer(&self) -> TunnelPeer {
        self.gateway
            .members
            .invite(crate::members::MemberRecord {
                role_id: "family".into(),
                ..crate::members::MemberRecord::for_test("mia")
            })
            .unwrap();
        self.gateway.members.mark_joined("mia").unwrap();
        let peer = TunnelPeer(SecretKey::generate().public());
        self.gateway
            .tunnel
            .devices
            .add_if(peer.0, None, Some("mia".into()), |_| true)
            .unwrap();
        let family = self.gateway.roles.get("family").unwrap();
        let mut permissions = family.permissions.clone();
        permissions.open_apps = AppScope::All;
        self.gateway
            .roles
            .update(
                "family",
                &RoleDraft {
                    name: "family".into(),
                    permissions,
                },
            )
            .unwrap();
        peer
    }

    fn request(&self, peer: Option<TunnelPeer>) -> hyper::Request<Body> {
        let mut req = hyper::Request::builder()
            .method("GET")
            .uri(SHARED_BOTS_PATH)
            .header(header::HOST, "wonton.localhost:18480")
            .header(header::ACCEPT_LANGUAGE, "zh-Hans")
            .body(Body::empty())
            .unwrap();
        if let Some(peer) = peer {
            req.extensions_mut().insert(peer);
        }
        req
    }

    async fn view(&self, peer: TunnelPeer) -> (StatusCode, SharedBotsView) {
        let response = self
            .app
            .clone()
            .oneshot(self.request(Some(peer)))
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }
}

#[tokio::test]
async fn owner_private_bot_is_listed_with_reason_and_shared_bot_with_shared() {
    let home = Home::start().await;
    let peer = home.family_peer();
    // 给同一个插件补一个家庭共享 Bot，再以家人身份读。
    home.gateway.plugins.bot_records_for_test(
        &PluginId::parse(PLUGIN).unwrap(),
        &[
            (
                "owner-private",
                "@private:test",
                BotKeeper::User {
                    username: "owner".to_owned(),
                },
                "private-token",
            ),
            (
                "shared",
                "@shared:test",
                BotKeeper::Household,
                "shared-token",
            ),
        ],
    );
    home.gateway.members.set_role("mia", "family").unwrap();
    let (status, view) = home.view(peer).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view.state, SharedBotsState::Ready);
    let mut bots: Vec<_> = view.bots.into_iter().collect();
    bots.sort_by(|a, b| a.bot.bot.cmp(&b.bot.bot));
    assert_eq!(bots.len(), 2);
    assert_eq!(
        bots[0].access,
        BotAccess::Unusable {
            reason: BotChatUnavailable::OwnerPrivate
        }
    );
    assert_eq!(bots[0].user_id.as_deref(), Some("@private:test"));
    assert_eq!(bots[0].title, "共享助手");
    assert_eq!(bots[1].access, BotAccess::Shared);
    assert_eq!(bots[1].user_id.as_deref(), Some("@shared:test"));
}

#[tokio::test]
async fn app_scope_is_a_reason_not_a_filter() {
    let home = Home::start().await;
    let peer = home.family_peer();
    let family = home.gateway.roles.get("family").unwrap();
    let mut permissions = family.permissions.clone();
    permissions.open_apps = AppScope::Selected {
        plugin_ids: vec!["app.other".into()],
    };
    home.gateway
        .roles
        .update(
            "family",
            &RoleDraft {
                name: "family".into(),
                permissions,
            },
        )
        .unwrap();
    let (status, view) = home.view(peer).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view.state, SharedBotsState::Ready);
    assert_eq!(view.bots.len(), 1);
    assert_eq!(
        view.bots[0].access,
        BotAccess::Unusable {
            reason: BotChatUnavailable::AppNotShared
        }
    );
}

#[tokio::test]
async fn missing_bot_record_is_a_row_not_an_empty_list() {
    let home = Home::start().await;
    let peer = home.family_peer();
    // 删掉 Bot 记录，模拟插件在、Bot 还没建出来。
    home.gateway
        .plugins
        .bot_records_for_test(&PluginId::parse(PLUGIN).unwrap(), &[]);
    let response = home
        .app
        .clone()
        .oneshot(home.request(Some(peer)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let view: SharedBotsView = serde_json::from_slice(&body).unwrap();
    assert_eq!(view.bots.len(), 1);
    assert_eq!(view.bots[0].user_id, None);
    assert_eq!(
        view.bots[0].access,
        BotAccess::Unusable {
            reason: BotChatUnavailable::BotMissing
        }
    );
}

#[tokio::test]
async fn devices_without_a_user_cannot_read_the_list() {
    let home = Home::start().await;
    let stranger = TunnelPeer(SecretKey::generate().public());
    let response = home
        .app
        .clone()
        .oneshot(home.request(Some(stranger)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let problem = String::from_utf8_lossy(&body);
    assert!(problem.contains("unauthenticated"), "{problem}");
}
