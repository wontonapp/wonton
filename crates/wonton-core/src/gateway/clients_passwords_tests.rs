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
    home_with_password_file(None).await
}

async fn home_with_password_file(contents: Option<&str>) -> Home {
    let dir = tempfile::tempdir().unwrap();
    if let Some(contents) = contents {
        let path = dir.path().join(crate::clients::passwords::FILE_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
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
    let source = format!(
        "services:\n  app:\n    image: example/app:1\nx-wonton:\n  spec: '0.14'\n  id: {PLUGIN}\n  version: 1.0.0\n  kind: app\n  title: {{en: Test}}\n  description: {{en: Test}}\n  permissions: [network.clients]\n  clients:\n    http:\n      service: app\n      port: 8080\n      auth:\n        gateway:\n          headers: {{X-Remote-User: '{{user}}'}}\n"
    );
    let manifest = crate::plugin::PluginPackage::manifest_of(&source, &Default::default()).unwrap();
    manager.register_for_test(manifest, crate::plugin::PluginStatus::Running, None);
    manager.clients_entry_for_test(
        &crate::plugin::PluginId::parse(PLUGIN).unwrap(),
        "gw",
        false,
        false,
    );
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

    async fn as_owner(
        &self,
        device: iroh::EndpointId,
        method: Method,
        path: &str,
        body: &str,
    ) -> Reply {
        self.send(
            self.request(method, path)
                .extension(TunnelPeer(device))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
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

#[tokio::test]
async fn created_passwords_are_discoverable_and_revocable_from_the_list() {
    let h = home().await;
    let device = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add_if(device, Some("Phone".into()), None, |_| true)
        .unwrap();
    let created = h
        .as_owner(
            device,
            Method::POST,
            &wonton_protocol::clients_passwords_path(PLUGIN),
            r#"{"label":"Calendar"}"#,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let value = created.json()["password"].as_str().unwrap().to_owned();
    let listed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    assert!(!listed.text().contains(&value));
    let body = listed.json();
    let passwords = body[0]["passwords"].as_array().unwrap();
    assert_eq!(passwords.len(), 1);
    assert_eq!(passwords[0]["label"], "Calendar");
    assert_eq!(passwords[0]["created_by_device"], "Phone");
    assert!(passwords[0].get("value").is_none());
    assert!(passwords[0].get("password").is_none());
    let path = wonton_protocol::clients_password_path(PLUGIN, passwords[0]["id"].as_str().unwrap());
    let read = h.as_owner(device, Method::GET, &path, "").await;
    assert_eq!(read.json()["password"], value);
    let viewed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(
        viewed.json()[0]["passwords"][0]["last_viewed_by"],
        json!(["Phone"])
    );
    let revoked = h.as_owner(device, Method::DELETE, &path, "").await;
    assert_eq!(revoked.status, StatusCode::OK);
    let listed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(listed.json()[0]["passwords"], json!([]));
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn list_reports_corrupt_password_storage_instead_of_an_empty_list() {
    let h = home_with_password_file(Some("invalid json")).await;
    let reply = h
        .send(
            h.request(Method::GET, &wonton_protocol::clients_path())
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
}

#[tokio::test]
async fn restored_password_summaries_keep_creation_and_view_order_and_pending_state() {
    let older = SecretKey::generate().public();
    let newer = SecretKey::generate().public();
    let contents = json!({"plugins": {(PLUGIN): [
        {"id": "pw-9", "label": "First", "value": "not-in-list-first",
         "created_by_device": "Tablet", "viewed_by": {(older.to_string()): 1, (newer.to_string()): 2}, "pending": true},
        {"id": "pw-2", "label": "Second", "value": "not-in-list-second"}
    ]}}).to_string();
    let h = home_with_password_file(Some(&contents)).await;
    for (id, name) in [(older, "Older"), (newer, "Newer")] {
        h.gateway
            .tunnel
            .devices
            .add_if(id, Some(name.into()), None, |_| true)
            .unwrap();
    }
    let reply = h
        .send(
            h.request(Method::GET, &wonton_protocol::clients_path())
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let body = reply.json();
    let passwords = body[0]["passwords"].as_array().unwrap();
    assert_eq!(
        passwords
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["pw-9", "pw-2"]
    );
    assert_eq!(passwords[0]["last_viewed_by"], json!(["Newer", "Older"]));
    assert_eq!(passwords[0]["pending"], "revoke");
    assert!(passwords[1]["pending"].is_null());
    assert!(!reply.text().contains("not-in-list"));
}
