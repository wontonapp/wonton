use super::*;
use crate::clients::state::Kind;
use crate::gateway::tests::Reply;
use crate::plugin::Permission;
use std::{fs, time::Duration};

fn record_path(h: &Harness, id: &PluginId) -> std::path::PathBuf {
    h.data_dir.join("plugins").join(format!("{id}.json"))
}

/// 第二个客户端插件：和 `register` 一样登记，换一个 id 和标签。
fn register_second(h: &Harness) -> PluginId {
    register_variant(
        h,
        &CLIENT_APP.replace("io.wonton.test.client", "io.wonton.test.second"),
        "second",
    )
}

/// 另一个清单的客户端插件：登记、给路由、放容器，入口标签用调用方给的。
fn register_variant(h: &Harness, source: &str, label: &str) -> PluginId {
    let manifest = PluginPackage::manifest_of(source, &Languages::default()).unwrap();
    let http = manifest.clients.as_ref().unwrap().http.clone().unwrap();
    let id = manifest.id.clone();
    h.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    h.manager
        .clients_route_for_test(&id, http, Some(h.upstream.to_string()));
    h.manager.clients_entry_for_test(&id, label, false, false);
    let mut record = h.manager.record_for_test(&id).unwrap();
    record.source = source.to_owned();
    record.granted_permissions = vec![Permission::NetworkClients];
    h.docker.add_image_container(
        &Instance::default().container(&id.slug(), "app"),
        "example/app:1",
    );
    h.manager.store().save(&record).unwrap();
    id
}

impl Rig {
    async fn list(&self) -> Reply {
        send_as(
            &self.app,
            None,
            Method::GET,
            CORE,
            &wonton_protocol::clients_path(),
            &[
                ("authorization", &format!("Bearer {TOKEN}")),
                ("x-wonton-clients", "1"),
            ],
            "",
        )
        .await
    }

    async fn entry_request(&self) -> Reply {
        let request = hyper::Request::builder()
            .uri("/")
            .header("host", "client.home.example.test")
            .header("x-wonton-entry", "public")
            .body(axum::body::Body::empty())
            .unwrap();
        Reply::of(
            crate::gateway::clients_entry::dispatch(
                &self.gateway,
                request,
                "client".into(),
                Kind::Public,
            )
            .await,
        )
        .await
    }

    async fn reinstall_client(&self) -> String {
        let container = Instance::default().container(&self.id.slug(), "app");
        self.h.docker.add_container(&container, self.id.as_str());
        self.h
            .manager
            .uninstall(&self.id, true, &Languages::default())
            .await
            .unwrap();
        self.h
            .manager
            .install(
                CLIENT_APP,
                &[Permission::NetworkClients],
                Languages::default(),
            )
            .await
            .unwrap();
        container
    }

    fn upgrade_client(&self) -> tokio::task::JoinHandle<Result<(), crate::plugin::InstallError>> {
        let manager = self.h.manager.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            let package =
                PluginPackage::parse(&CLIENT_APP.replace("1.0.0", "1.1.0"), &Languages::default())
                    .unwrap();
            manager
                .upgrade_upload(
                    &id,
                    package,
                    vec![Permission::NetworkClients],
                    vec![],
                    false,
                    &[],
                    &Default::default(),
                    &Languages::default(),
                )
                .await
        })
    }
}

#[tokio::test]
async fn upgrade_commit_keeps_successfully_closed_entry_closed() {
    let rig = Rig::start(true, true, true).await;
    let container = rig.reinstall_client().await;
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let old = rig
        .gateway
        .clients
        .resolve("client", Kind::Public)
        .unwrap()
        .unwrap()
        .revoked;
    let held = rig
        .h
        .docker
        .hold(&format!("DELETE /containers/{container}"));
    let upgrading = rig.upgrade_client();
    tokio::time::timeout(Duration::from_secs(10), held.reached())
        .await
        .unwrap();
    assert_eq!(
        rig.h.manager.record_for_test(&rig.id).unwrap().version,
        "1.0.0"
    );
    assert!(
        rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert_eq!(rig.switch("public", false).await.status, StatusCode::OK);
    assert!(*old.borrow());
    held.release();
    upgrading.await.unwrap().unwrap();
    crate::clients::apply::reconcile(&rig.gateway)
        .await
        .unwrap();
    let record = rig.h.manager.record_for_test(&rig.id).unwrap();
    assert_eq!(record.version, "1.1.0");
    assert!(!record.clients_entries.public);
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
    assert!(*old.borrow());
    assert_eq!(rig.entry_request().await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unrelated_install_does_not_block_listing_or_closing() {
    let rig = Rig::start(true, true, true).await;
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let old = rig
        .gateway
        .clients
        .resolve("client", Kind::Public)
        .unwrap()
        .unwrap()
        .revoked;
    let id = PluginId::parse("io.wonton.test.installing").unwrap();
    let held = rig.h.docker.hold(&format!(
        "POST /containers/{}/start",
        Instance::default().container(&id.slug(), "app")
    ));
    let manager = rig.h.manager.clone();
    let installing = tokio::spawn(async move {
        manager
            .install(
                &CLIENT_APP.replace("io.wonton.test.client", "io.wonton.test.installing"),
                &[Permission::NetworkClients],
                Languages::default(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), held.reached())
        .await
        .unwrap();
    assert!(rig.h.manager.store().load(&id).unwrap().is_none());
    let listed = rig.list().await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    assert_eq!(listed.json()[0]["id"], rig.id.as_str());
    assert_eq!(listed.json().as_array().unwrap().len(), 1);
    assert_eq!(rig.switch("public", false).await.status, StatusCode::OK);
    assert!(*old.borrow());
    assert_eq!(rig.entry_request().await.status, StatusCode::FORBIDDEN);
    held.release();
    installing.await.unwrap().unwrap();
}

#[tokio::test]
async fn missing_installed_record_is_not_hidden_as_installing() {
    let rig = Rig::start(true, true, true).await;
    rig.h.manager.store().remove(&rig.id).unwrap();
    let reply = rig.list().await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
}

#[tokio::test]
async fn setup_loss_returns_closed_switches_on_the_detecting_get() {
    let rig = Rig::start(true, true, true).await;
    assert_eq!(rig.switch("lan", true).await.status, StatusCode::OK);
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let old = rig
        .gateway
        .clients
        .resolve("client", Kind::Public)
        .unwrap()
        .unwrap()
        .revoked;
    rig.h.docker.set_state(
        &Instance::default().container(&rig.id.slug(), "app"),
        json!({"Status": "running", "Running": true, "StartedAt": "2026-02-01T00:00:00Z"}),
    );
    rig.h.docker.on_exec(|_, _, _| (1, String::new()));
    let reply = rig.list().await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let body = reply.json();
    assert_eq!(body[0]["blocked"], "setup_pending");
    assert!(body[0]["lan_url"].is_null());
    assert!(body[0]["public_url"].is_null());
    let entries = body[0]["entries"].as_array().unwrap();
    assert!(entries.iter().all(|entry| entry["enabled"] == false));
    let record = rig.h.manager.record_for_test(&rig.id).unwrap();
    assert!(!record.clients_entries.lan && !record.clients_entries.public);
    assert!(*old.borrow());
    assert_eq!(rig.entry_request().await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn rollback_metadata_keeps_entry_closed_after_successful_put() {
    let rig = Rig::start(true, true, true).await;
    let container = rig.reinstall_client().await;
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let revoked = rig
        .gateway
        .clients
        .resolve("client", Kind::Public)
        .unwrap()
        .unwrap()
        .revoked;
    let (snapshot, resume) = rig.h.manager.hold_recovery_metadata_for_test(&rig.id);
    rig.h
        .docker
        .fail_next_create(&container, "new version rejected");
    let upgrading = rig.upgrade_client();

    // 停在 back_to_old 已重建旧容器并读出记录之后；关闭必须在旧副本写回之前成功。
    let stale = tokio::time::timeout(Duration::from_secs(10), snapshot)
        .await
        .unwrap()
        .unwrap();
    assert!(stale.clients_entries.public);
    assert_eq!(stale.version, "1.0.0");
    assert_eq!(
        rig.h.manager.route(&rig.id.slug(), "/").unwrap().status,
        PluginStatus::Upgrading
    );
    let closed = tokio::time::timeout(Duration::from_secs(10), rig.switch("public", false))
        .await
        .unwrap();
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.text());
    assert!(*revoked.borrow());
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    resume.send(()).unwrap();

    assert!(
        tokio::time::timeout(Duration::from_secs(10), upgrading)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(
        rig.h.manager.route(&rig.id.slug(), "/").unwrap().status,
        PluginStatus::Running
    );
    let record = rig.h.manager.record_for_test(&rig.id).unwrap();
    assert_eq!(record.version, "1.0.0");
    assert!(!record.clients_entries.public);
    crate::clients::apply::reconcile(&rig.gateway)
        .await
        .unwrap();
    let listed = rig.list().await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    let body = listed.json();
    assert!(body[0]["public_url"].is_null());
    let public = body[0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"]["kind"] == "public")
        .unwrap();
    assert_eq!(public["enabled"], false);
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
    assert!(*revoked.borrow());
    assert_eq!(rig.entry_request().await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn broken_record_client_can_be_uninstalled() {
    let rig = Rig::start(true, true, true).await;
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    fs::write(record_path(&rig.h, &rig.id), "{\"id\": ").unwrap();
    rig.h
        .manager
        .uninstall(&rig.id, false, &Languages::default())
        .await
        .unwrap();
    assert!(!record_path(&rig.h, &rig.id).exists());
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
}

#[tokio::test]
async fn broken_record_does_not_block_switching_another_client() {
    let rig = Rig::start(true, true, true).await;
    let other = register_second(&rig.h);
    fs::write(record_path(&rig.h, &rig.id), "{\"id\": ").unwrap();

    let listed = rig.list().await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    assert_eq!(listed.json().as_array().unwrap().len(), 1);
    assert_eq!(listed.json()[0]["id"], other.as_str());

    assert_eq!(
        rig.switch_id(&other, "public", true).await.status,
        StatusCode::OK
    );
    assert!(rig.crt_list("public").contains("second.home.example.test"));
    assert_eq!(
        rig.switch_id(&other, "public", false).await.status,
        StatusCode::OK
    );
    assert!(!rig.crt_list("public").contains("second.home.example.test"));

    rig.h
        .manager
        .uninstall(&other, false, &Languages::default())
        .await
        .unwrap();
    assert!(!record_path(&rig.h, &other).exists());
    assert!(record_path(&rig.h, &rig.id).exists());
}

#[tokio::test]
async fn missing_client_record_blocks_uninstalling_another_client() {
    let rig = Rig::start(true, true, true).await;
    let other = register_second(&rig.h);
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    rig.h.manager.store().remove(&other).unwrap();

    let error = rig
        .h
        .manager
        .uninstall(&rig.id, false, &Languages::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, crate::plugin::InstallError::UpstreamUnavailable(_)),
        "{error:?}"
    );
    assert_eq!(error.kind(), "upstream_unavailable");
    assert!(record_path(&rig.h, &rig.id).exists());
}

const CREDENTIALS_APP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.credentials
  version: 1.0.0
  kind: app
  title: { en: Credentials }
  description: { en: Credentials entry test }
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

/// 有 HTTP 入口之外还声明了 `clients.tcp`；TCP 转发还没接。
const TCP_APP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.tcp
  version: 1.0.0
  kind: app
  title: { en: Tcp }
  description: { en: Tcp entry test }
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        app:
          setup_done: ['true']
    tcp:
      - name: ssh
        service: app
        port: 22
"#;

#[tokio::test]
async fn credentials_entry_opens_after_first_issue() {
    let rig = Rig::start(true, true, true).await;
    let id = register_variant(&rig.h, CREDENTIALS_APP, "credentials");

    let blocked = rig.switch_id(&id, "lan", true).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT, "{}", blocked.text());
    assert_eq!(blocked.error_kind(), Some("conflict"));
    assert!(
        blocked.text().contains("has not issued a password"),
        "{}",
        blocked.text()
    );
    assert!(
        !rig.h
            .manager
            .record_for_test(&id)
            .unwrap()
            .clients_entries
            .lan
    );

    rig.h
        .manager
        .client_passwords
        .as_ref()
        .unwrap()
        .mark_issued(id.as_str())
        .unwrap();
    let opened = rig.switch_id(&id, "lan", true).await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.text());
    assert!(
        rig.h
            .manager
            .record_for_test(&id)
            .unwrap()
            .clients_entries
            .lan
    );
    assert!(
        rig.crt_list("lan")
            .contains("credentials.lan.home.example.test")
    );
}

#[tokio::test]
async fn tcp_entries_are_not_listed_and_rejected_until_forwarding_exists() {
    let rig = Rig::start(true, true, true).await;
    let id = register_variant(&rig.h, TCP_APP, "tcp");

    let listed = rig.list().await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    let item = listed
        .json()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id.as_str())
        .cloned()
        .unwrap();
    let kinds: Vec<&str> = item["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["lan", "public"]);

    let reply = rig.switch_id(&id, "ssh", true).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("invalid_input"));
}
