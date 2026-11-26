//! 专用密码管理接口的行为测试：审计具体到设备、设备吊销连带（ADR-0021、ADR-0046）。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use hyper::header::AUTHORIZATION;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::clients::credentials::{CredentialExec, ExecError, ExecOutput};
use crate::clients::passwords::State;
use crate::config::PublicUrls;
use crate::gateway::fakes::loopback_tunnel;
use crate::gateway::tests::{CORE, Reply, TOKEN, owner};
use crate::gateway::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::mcp::audit::Audit;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::PluginPackage;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::Store;
use futures_util::future::BoxFuture;

const PLUGIN: &str = "app.demo.gw";

const MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 凭据应用, en: Credentials app }
  description:
    zh: 凭据接口测试用
    en: For password API tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{user}"
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

/// 共享密码的清单（`per_client: false`）：吊销默认换值，`keep_shared_passwords` 时原样留着。
const KEEP_SHARED_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 凭据应用, en: Credentials app }
  description:
    zh: 凭据接口测试用
    en: For password API tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{user}"
          per_client: false
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          rotate: [pw, rotate, "{name}"]
          ends_sessions: false
"#;

/// 网关测试不关心应用里的输出；退出 0 就当命令成功。
struct AlwaysOk;

/// 固定一种结果，测 502 和超时。
#[derive(Clone, Copy)]
enum Outcome {
    Exit(i64),
    Timeout,
}

struct Fixed(Outcome);

impl CredentialExec for Fixed {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async {
            match self.0 {
                Outcome::Exit(code) => Ok(ExecOutput {
                    exit_code: code,
                    stdout: String::new(),
                    stderr: "应用拒绝".into(),
                }),
                Outcome::Timeout => Err(ExecError::Timeout),
            }
        })
    }
}

impl CredentialExec for AlwaysOk {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async {
            Ok(ExecOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

/// 数跑过几次命令：keep_shared 吊销一次都不该跑。
#[derive(Clone, Default)]
struct Counting(Arc<AtomicUsize>);

impl Counting {
    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl CredentialExec for Counting {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        _argv: &'a [String],
        _stdin: Option<&'a [u8]>,
        _timeout: std::time::Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ExecOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

struct Home {
    app: Router,
    dir: tempfile::TempDir,
    gateway: Arc<Gateway>,
    manifest: &'static str,
}

async fn home() -> Home {
    home_with(MANIFEST).await
}

async fn home_with(manifest: &'static str) -> Home {
    home_with_manifest_and_file(manifest, None).await
}

async fn home_with_password_file(contents: Option<&str>) -> Home {
    home_with_manifest_and_file(GATEWAY_MANIFEST, contents).await
}

async fn home_with_manifest_and_file(manifest: &'static str, contents: Option<&str>) -> Home {
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
    manager.set_credential_exec(Arc::new(AlwaysOk));
    let parsed = PluginPackage::parse(manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    manager.register_for_test(
        parsed,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
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
        manifest,
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

    fn owner_request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        self.request(method, path)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
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

    fn set_status(&self, status: crate::plugin::manager::PluginStatus) {
        let manifest =
            PluginPackage::parse(self.manifest, &Languages::from_header(Some("zh-Hans")))
                .unwrap()
                .manifest;
        self.gateway
            .manager()
            .register_for_test(manifest, status, Some("127.0.0.1:9".into()));
    }

    async fn create(&self, label: &str) -> Reply {
        self.send(
            self.owner_request(
                Method::POST,
                &format!("/_wonton/clients/{PLUGIN}/passwords"),
            )
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "label": label }).to_string()))
            .unwrap(),
        )
        .await
    }
}

impl Gateway {
    fn manager(&self) -> &Arc<PluginManager> {
        &self.plugins
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
async fn device_revoke_keep_shared_keeps_value_and_skips_command() {
    let h = home_with(KEEP_SHARED_MANIFEST).await;
    let exec = Counting::default();
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(exec.clone()));
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let device = SecretKey::generate().public();
    let device_id = device.to_string();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();
    let created = store
        .create(PLUGIN, "日历", Some((&device_id, Some("平板"))))
        .unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"keep_shared_passwords": true}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert_eq!(exec.calls(), 0);
    let kept = store.snapshot(PLUGIN, &created.id).unwrap();
    assert_eq!(kept.value, created.value);
    assert_eq!(kept.state, State::Ready);
    // 别的设备手上同一个老值照样能用。
    assert!(
        store
            .authenticate(PLUGIN, &created.value)
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
async fn device_revoke_null_body_rotates_shared() {
    let h = home_with(KEEP_SHARED_MANIFEST).await;
    let exec = Counting::default();
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(exec.clone()));
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let device = SecretKey::generate().public();
    let device_id = device.to_string();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();
    let created = store
        .create(PLUGIN, "日历", Some((&device_id, Some("平板"))))
        .unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("null"))
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert_eq!(exec.calls(), 1);
    let rotated = store.snapshot(PLUGIN, &created.id).unwrap();
    assert_ne!(rotated.value, created.value);
    assert_eq!(rotated.state, State::Ready);
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

#[tokio::test]
async fn device_revoke_invalid_body_is_invalid_input() {
    let h = home().await;
    let device = SecretKey::generate().public();
    h.gateway.tunnel.devices.add(device, None).unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        revoked.status,
        StatusCode::BAD_REQUEST,
        "{}",
        revoked.text()
    );
    assert_eq!(revoked.error_kind(), Some("invalid_input"));
    // 解析失败时设备留着，什么都没动。
    assert!(h.gateway.tunnel.devices.get(&device).is_some());
}

#[tokio::test]
async fn list_marks_unissued_credentials_and_pending_after_deferral() {
    let h = home().await;
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let plugins = list.json();
    let item = &plugins[0];
    assert_eq!(item["auth"], "credentials");
    assert_eq!(item["username"], "owner");
    assert_eq!(item["passwords"], json!([]));
    assert_eq!(item["blocked"], "credentials_not_issued");

    // 插件没在跑：202 落盘，列表标成待换值，入口仍然不给开。
    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h.create("手机").await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED, "{}", deferred.text());
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    assert_eq!(plugins[0]["passwords"][0]["pending"], "rotate");
    assert_eq!(plugins[0]["blocked"], "credentials_not_issued");

    // 启动后重试成功：值转正、入口解除封锁。
    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .reconcile_credentials(&PluginId::parse(PLUGIN).unwrap())
        .await;
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    assert!(plugins[0]["passwords"][0]["pending"].is_null());
    assert!(plugins[0]["blocked"].is_null());
}

#[tokio::test]
async fn value_read_marks_pending_and_command_failure_is_502() {
    let h = home().await;
    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h.create("手机").await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED, "{}", deferred.text());
    let id = h
        .gateway
        .plugins
        .client_passwords
        .as_ref()
        .unwrap()
        .snapshots(PLUGIN)[0]
        .id
        .clone();

    let value = h
        .send(
            h.owner_request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{id}"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(value.status, StatusCode::OK);
    assert_eq!(
        value
            .headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some("rotate")
    );

    // 命令失败：502、记录仍在，错误文字来自应用。
    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(Fixed(Outcome::Exit(2))));
    let failed = h.create("平板").await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY);
    assert!(failed.text().contains("应用拒绝"));
    let pending = h
        .gateway
        .plugins
        .client_passwords
        .as_ref()
        .unwrap()
        .snapshots(PLUGIN)
        .into_iter()
        .find(|snapshot| snapshot.label == "平板")
        .unwrap();
    assert_eq!(
        failed
            .headers
            .get(wonton_protocol::PASSWORD_ID_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some(pending.id.as_str())
    );
}

#[tokio::test]
async fn revoke_maps_202_502_and_200_and_keeps_value_until_done() {
    let h = home().await;
    let created = h.create("手机").await;
    assert_eq!(created.status, StatusCode::CREATED);
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let id = store.snapshots(PLUGIN)[0].id.clone();
    let value = created.json()["password"].as_str().unwrap().to_owned();
    let path = format!("/_wonton/clients/{PLUGIN}/passwords/{id}");

    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED);
    assert_eq!(
        deferred
            .headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some("revoke")
    );
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_some()
    );

    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(Fixed(Outcome::Timeout)));
    let failed = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY, "{}", failed.text());
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_some()
    );

    h.gateway.plugins.set_credential_exec(Arc::new(AlwaysOk));
    let done = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    assert_eq!(
        done.headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        None,
        "做完的 200 不再标待完成"
    );
    let gone = h
        .send(
            h.owner_request(Method::GET, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(gone.status, StatusCode::BAD_REQUEST);
}

const SHARED_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.shared
  version: 1.0.0
  kind: app
  title: { zh: 共享凭据, en: Shared credentials }
  description:
    zh: 共享密码冲突测试用
    en: For shared password conflict tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          per_client: false
          ends_sessions: false
          issue: [pw, issue, "{name}"]
          rotate: [pw, rotate, "{name}"]
"#;

#[tokio::test]
async fn second_shared_password_is_conflict() {
    let h = home().await;
    let manifest = PluginPackage::parse(SHARED_MANIFEST, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    h.gateway.plugins.register_for_test(
        manifest,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    let path = "/_wonton/clients/app.demo.shared/passwords";
    let body = || Body::from(json!({ "label": "共享" }).to_string());
    let first = h
        .send(
            h.owner_request(Method::POST, path)
                .header(CONTENT_TYPE, "application/json")
                .body(body())
                .unwrap(),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.text());
    assert_eq!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .snapshots("app.demo.shared")[0]
            .id,
        "shared"
    );
    let second = h
        .send(
            h.owner_request(Method::POST, path)
                .header(CONTENT_TYPE, "application/json")
                .body(body())
                .unwrap(),
        )
        .await;
    assert_eq!(second.status, StatusCode::CONFLICT);
}

const GATEWAY_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.gw
  version: 1.0.0
  kind: app
  title: { zh: 网关应用, en: Gateway app }
  description:
    zh: 网关验证测试用
    en: For gateway auth tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        gateway:
          headers:
            X-Remote-User: "{user}"
"#;

fn register(home: &Home, manifest: &str) {
    let manifest = PluginPackage::parse(manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    home.gateway.plugins.register_for_test(
        manifest,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    // 列表要读安装记录（入口标签、开关），补一条最小记录。
    home.gateway
        .plugins
        .clients_entry_for_test(&id, "客户端", false, false);
}

const EMAIL_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.email
  version: 1.0.0
  kind: app
  title: { zh: 邮箱应用, en: Email app }
  description:
    zh: 邮箱用户名测试用
    en: For email username tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{email}"
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

const TCP_MANIFEST: &str = r#"
services:
  ssh:
    image: example/ssh:1
x-wonton:
  spec: '0.4'
  id: app.demo.tcp
  version: 1.0.0
  kind: app
  title: { zh: 同步, en: Sync }
  description:
    zh: 只有 TCP 入口
    en: TCP entries only
  permissions: [network.clients]
  clients:
    tcp:
      - name: ssh
        service: ssh
        port: 22
"#;

#[tokio::test]
async fn gateway_plugin_password_post_still_creates_a_plain_value() {
    let h = home().await;
    register(&h, GATEWAY_MANIFEST);
    let reply = h
        .send(
            h.owner_request(Method::POST, "/_wonton/clients/app.demo.gw/passwords")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "label": "手机" }).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let value = reply.json()["password"].as_str().unwrap().to_owned();
    assert!(store.authenticate("app.demo.gw", &value).unwrap().is_some());
}

#[tokio::test]
async fn email_username_is_filled_from_the_owner() {
    let h = home().await;
    register(&h, EMAIL_MANIFEST);
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let plugins = list.json();
    let item = plugins
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "app.demo.email")
        .unwrap();
    assert_eq!(item["username"], "owner@wonton.internal");
}

#[tokio::test]
async fn tcp_only_plugin_is_listed_without_http_auth() {
    let h = home().await;
    register(&h, TCP_MANIFEST);
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    let item = plugins
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "app.demo.tcp")
        .unwrap();
    assert_eq!(item["auth"], "app");
    assert!(item["passwords"].as_array().unwrap().is_empty());
    // 只声明 tcp 的插件没有 HTTP 入口；TCP 转发还没接，先不列入开关。
    assert!(item["entries"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn uninstalled_plugin_is_plugin_unavailable() {
    let h = home().await;
    let reply = h
        .send(
            h.owner_request(Method::POST, "/_wonton/clients/app.missing.gw/passwords")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "label": "手机" }).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["error"]["kind"], "plugin_unavailable");
}
