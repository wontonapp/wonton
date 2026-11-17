//! 专用密码管理接口的行为测试：审计具体到设备、设备吊销连带（ADR-0021、ADR-0046）。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use hyper::header::AUTHORIZATION;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::config::PublicUrls;
use crate::gateway::fakes::loopback_tunnel;
use crate::gateway::tests::{CORE, Reply, TOKEN, owner};
use crate::gateway::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::mcp::audit::Audit;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;

const PLUGIN: &str = "app.demo.gw";

struct Home {
    app: Router,
    dir: tempfile::TempDir,
    gateway: Arc<Gateway>,
}

async fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
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
        install_timeout: std::time::Duration::from_secs(5),
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
    let audit = Audit::new(dir.path().join("audit.jsonl"));
    let gateway = Arc::new(
        Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members,
            manager,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_audit(Some(Arc::new(audit))),
    );
    Home {
        app: router(gateway.clone()),
        dir,
        gateway,
    }
}

impl Home {
    async fn send(&self, request: Request<Body>) -> Reply {
        Reply::of(self.app.clone().oneshot(request).await.unwrap()).await
    }

    fn request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("{CORE}:18480"))
            .header("x-wonton-clients", "1")
    }

    fn audit_lines(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.dir.path().join("audit.jsonl")).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn value_read_audited_per_device() {
    let h = home().await;
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let created = store.create(PLUGIN, "phone", None).unwrap();
    let device = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();

    let reply = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{}", created.id),
            )
            .extension(TunnelPeer(device))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let value = reply.json();
    assert_eq!(value["username"], "owner");
    assert_eq!(value["password"], created.value);

    let lines = h.audit_lines();
    let reads: Vec<_> = lines
        .iter()
        .filter(|line| line["event"] == "client_password_read")
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0]["plugin"], PLUGIN);
    assert_eq!(reads[0]["by"], format!("owner/device:{device}"));
    // 值本身不进审计。
    let audit_text = std::fs::read_to_string(h.dir.path().join("audit.jsonl")).unwrap();
    assert!(!audit_text.contains(&created.value));
}

#[tokio::test]
async fn device_revoke_revokes_its_passwords() {
    let h = home().await;
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let creator = SecretKey::generate().public();
    let viewer = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add(creator, Some("平板".into()))
        .unwrap();
    h.gateway
        .tunnel
        .devices
        .add(viewer, Some("手机".into()))
        .unwrap();

    // 平板建一个，手机看一次；另一条谁都没碰。
    let created = h
        .send(
            h.request(
                Method::POST,
                &format!("/_wonton/clients/{PLUGIN}/passwords"),
            )
            .extension(TunnelPeer(creator))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"label": "日历"}).to_string()))
            .unwrap(),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let seen = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/pw-1"),
            )
            .extension(TunnelPeer(viewer))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.text());
    let untouched = store.create(PLUGIN, "电脑", None).unwrap();

    // 吊销平板：它建的密码立即失效；没碰过的不动。
    let revoked = h
        .send(
            h.request(Method::DELETE, &format!("/_wonton/api/devices/{creator}"))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert!(store.authenticate(PLUGIN, "pw-1").unwrap().is_none());
    assert!(
        store
            .authenticate(PLUGIN, &untouched.value)
            .unwrap()
            .is_some()
    );

    // 手机看过的那条跟着手机的吊销失效。
    let second = store.create(PLUGIN, "邮件", None).unwrap();
    let seen = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{}", second.id),
            )
            .extension(TunnelPeer(viewer))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.text());
    let revoked = h
        .send(
            h.request(Method::DELETE, &format!("/_wonton/api/devices/{viewer}"))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert!(store.authenticate(PLUGIN, &second.value).unwrap().is_none());
    assert!(
        store
            .authenticate(PLUGIN, &untouched.value)
            .unwrap()
            .is_some()
    );
}
