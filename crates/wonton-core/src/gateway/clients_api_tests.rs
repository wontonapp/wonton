use crate::clients::domain::Domain;
use crate::entry_containers::EntryContainers;
use crate::entry_settings::EntrySettings;
use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness, TOKEN};
use crate::members::MemberRecord;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::names::Instance;
use crate::plugin::{PluginPackage, PluginStatus};
use axum::Router;
use hyper::{Method, StatusCode};
use serde_json::json;
use std::sync::Arc;

struct Rig {
    h: Harness,
    gateway: Arc<super::Gateway>,
    app: Router,
    id: PluginId,
    install: std::path::PathBuf,
}

impl Rig {
    async fn start(lan: bool, ingress: bool, public: bool) -> Self {
        let h = Harness::start(None).await;
        let id = register(&h);
        let install = prepare_entry_files(&h);
        let entry = EntryContainers::new(
            h.docker.engine().await,
            install.clone(),
            h.data_dir.clone(),
            Instance::default(),
            Some("wonton-core".into()),
        )
        .with_lan_forward_available(lan);
        let domain = Domain::new(
            install.clone(),
            h.data_dir.clone(),
            h.docker.engine().await,
            Instance::default(),
            None,
        );
        let gateway = Arc::new(
            super::Gateway::new(
                h.gateway.public.clone(),
                h.gateway.owner.clone(),
                TOKEN.into(),
                h.members.clone(),
                h.manager.clone(),
                None,
                h.http.clone(),
                h.tunnel.clone(),
            )
            .with_entry(Some(entry))
            .with_clients(Some(domain))
            .with_public_ingress_configured(ingress)
            .with_public_access(public),
        );
        h.manager.attach_clients_gateway(&gateway);
        let app = crate::gateway::router(gateway.clone());
        Self {
            h,
            gateway,
            app,
            id,
            install,
        }
    }

    async fn switch(&self, kind: &str, enabled: bool) -> crate::gateway::tests::Reply {
        let bearer = format!("Bearer {TOKEN}");
        send_as(
            &self.app,
            None,
            Method::PUT,
            CORE,
            &wonton_protocol::clients_entry_path(self.id.as_str(), kind),
            &[
                ("authorization", &bearer),
                ("x-wonton-clients", "1"),
                ("content-type", "application/json"),
            ],
            &json!({"enabled": enabled}).to_string(),
        )
        .await
    }

    fn crt_list(&self, kind: &str) -> String {
        std::fs::read_to_string(self.install.join(format!("entry/certs/{kind}.crt-list"))).unwrap()
    }

    fn reloads(&self) -> usize {
        self.h
            .docker
            .writes()
            .iter()
            .filter(|call| call.as_str() == "POST /containers/wonton-entry/kill")
            .count()
    }
}

fn register(h: &Harness) -> PluginId {
    let manifest = PluginPackage::manifest_of(CLIENT_APP, &Languages::default()).unwrap();
    let http = manifest.clients.as_ref().unwrap().http.clone().unwrap();
    let id = manifest.id.clone();
    h.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    h.manager
        .clients_route_for_test(&id, http, Some(h.upstream.to_string()));
    h.manager
        .clients_entry_for_test(&id, "client", false, false);
    let mut record = h.manager.record_for_test(&id).unwrap();
    record.source = CLIENT_APP.to_owned();
    record.granted_permissions = vec![crate::plugin::Permission::NetworkClients];
    h.manager.store().save(&record).unwrap();
    h.docker.add_image_container(
        &Instance::default().container(&id.slug(), "app"),
        "example/app:1",
    );
    h.docker.add_stranger_container(
        "wonton-core",
        json!({"Labels":{"com.docker.compose.project":"wonton"}}),
    );
    for network in ["clients", "edge", "relay"] {
        h.docker.add_unlabeled_network(&format!("wonton_{network}"));
    }
    id
}

fn prepare_entry_files(h: &Harness) -> std::path::PathBuf {
    let install = h.data_dir.join("entry-test");
    std::fs::create_dir_all(install.join("entry/certs")).unwrap();
    let tls = rcgen::generate_simple_self_signed(vec![
        "home.example.test".into(),
        "*.home.example.test".into(),
        "*.lan.home.example.test".into(),
    ])
    .unwrap();
    std::fs::write(
        install.join("entry/certs/site.pem"),
        format!("{}{}", tls.cert.pem(), tls.signing_key.serialize_pem()),
    )
    .unwrap();
    let settings = EntrySettings {
        domain: "home.example.test".into(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: None,
        acme_ca: None,
        acme_resolvers: vec![],
        acme_email: None,
        acme_accept_terms: false,
        acme_network: "bridge".into(),
        dns_provider: None,
        dns_env_file: None,
        acme_propagation_disable_ans: false,
        external_certificate: true,
    };
    std::fs::write(
        install.join("entry/settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    crate::entry_compose::write(&install, &settings).unwrap();
    install
}

const CLIENT_APP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.14'
  id: io.wonton.test.client
  version: 1.0.0
  kind: app
  title: { en: Client }
  description: { en: Client test }
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        app:
          setup_done: ['true']
"#;

#[tokio::test]
async fn list_shows_only_clients_plugins() {
    let h = Harness::start(None).await;
    let manifest = PluginPackage::manifest_of(CLIENT_APP, &Languages::default()).unwrap();
    h.manager
        .register_for_test(manifest.clone(), PluginStatus::Running, None);
    h.manager
        .clients_entry_for_test(&manifest.id, "client", false, false);
    let bearer = format!("Bearer {TOKEN}");
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        &wonton_protocol::clients_path(),
        &[("authorization", &bearer), ("x-wonton-clients", "1")],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let body = reply.json();
    let plugins = body.as_array().unwrap();
    assert_eq!(plugins.len(), 1, "{body}");
    assert_eq!(plugins[0]["id"], "io.wonton.test.client");
    assert!(reply.headers.get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn family_member_forbidden() {
    let h = Harness::start(None).await;
    h.members
        .invite(MemberRecord {
            username: "mia".into(),
            user_id: "@mia:test".into(),
            display_name: "Mia".into(),
            email: None,
            role_id: crate::roles::DEFAULT_INVITE_ROLE.to_owned(),
            status: wonton_protocol::MemberStatus::Joined,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    for (method, path, body) in [
        (Method::GET, wonton_protocol::clients_path(), ""),
        (
            Method::PUT,
            format!(
                "{}/io.wonton.test.client/entries/public",
                wonton_protocol::clients_path()
            ),
            "{\"enabled\":true}",
        ),
    ] {
        let reply = send_as(
            &h.app,
            Some(device),
            method,
            CORE,
            &path,
            &[
                ("x-wonton-clients", "1"),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
        assert!(reply.headers.get("access-control-allow-origin").is_none());
    }
}

#[tokio::test]
async fn missing_clients_header_rejected() {
    let h = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    for (method, path, body) in [
        (Method::GET, wonton_protocol::clients_path(), ""),
        (
            Method::PUT,
            format!(
                "{}/io.wonton.test.client/entries/lan",
                wonton_protocol::clients_path()
            ),
            "{\"enabled\":true}",
        ),
    ] {
        let reply = send_as(
            &h.app,
            None,
            method,
            CORE,
            &path,
            &[
                ("authorization", &bearer),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
        assert!(reply.headers.get("access-control-allow-origin").is_none());
    }
}

#[tokio::test]
async fn no_mcp_tool_for_switches() {
    let home = crate::gateway::mcp_tests::start(Some(vec![crate::plugin::Permission::Tools])).await;
    let agent = home.agent().await;
    let tools = agent.peer().list_all_tools().await.unwrap();
    for tool in tools {
        let name = tool.name.as_ref();
        assert!(
            !name.starts_with("wonton__") || (!name.contains("clients") && !name.contains("entry")),
            "{name}"
        );
    }
}

#[tokio::test]
async fn opening_without_container_is_not_reported_as_success() {
    let h = Harness::start(None).await;
    let manifest = PluginPackage::manifest_of(CLIENT_APP, &Languages::default()).unwrap();
    h.manager
        .register_for_test(manifest.clone(), PluginStatus::Running, None);
    h.manager
        .clients_entry_for_test(&manifest.id, "client", false, false);
    let bearer = format!("Bearer {TOKEN}");
    let path = format!(
        "{}/io.wonton.test.client/entries/public",
        wonton_protocol::clients_path()
    );
    let reply = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        &path,
        &[
            ("authorization", &bearer),
            ("x-wonton-clients", "1"),
            ("content-type", "application/json"),
        ],
        "{\"enabled\":true}",
    )
    .await;
    assert_eq!(reply.json()["error"]["kind"], "core_not_in_container");
    assert!(
        !h.manager
            .record_for_test(&manifest.id)
            .unwrap()
            .clients_entries
            .public
    );
}

#[tokio::test]
async fn reload_failure_rolls_back_switch() {
    let rig = Rig::start(true, true, true).await;
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let before = rig.crt_list("public");
    let guard = rig
        .gateway
        .clients
        .resolve("client", crate::clients::state::Kind::Public)
        .unwrap()
        .unwrap()
        .revoked;
    rig.h
        .docker
        .fail_next_kill("wonton-entry", "signal rejected");
    let reply = rig.switch("public", false).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("ingress_unavailable"));
    assert!(
        rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert_eq!(rig.crt_list("public"), before);
    assert!(*guard.borrow(), "回滚不能复活已经关闭的连接");
}

#[tokio::test]
async fn uninstall_removes_label_from_crt_lists() {
    let rig = Rig::start(true, true, true).await;
    assert_eq!(rig.switch("lan", true).await.status, StatusCode::OK);
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    rig.h
        .manager
        .uninstall(&rig.id, false, &Languages::default())
        .await
        .unwrap();
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
    assert!(!rig.crt_list("lan").contains("client.lan.home.example.test"));
    assert!(rig.h.manager.record_for_test(&rig.id).is_none());
    assert_eq!(
        rig.h
            .manager
            .store()
            .retained_entry_labels()
            .unwrap()
            .get(rig.id.as_str())
            .map(String::as_str),
        Some("client")
    );
}

#[tokio::test]
async fn open_lan_writes_lan_crt_list_and_reloads() {
    let rig = Rig::start(true, true, true).await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        reply.json()["lan_url"],
        "https://client.lan.home.example.test:8483"
    );
    assert!(rig.crt_list("lan").contains("client.lan.home.example.test"));
    assert_eq!(rig.reloads(), 1);
    assert!(
        rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .lan
    );
    crate::clients::apply::reconcile(&rig.gateway)
        .await
        .unwrap();
    assert!(rig.crt_list("lan").contains("client.lan.home.example.test"));
    assert_eq!(rig.reloads(), 1, "无变化的对账不应重复重载");
    assert_eq!(rig.switch("lan", false).await.status, StatusCode::OK);
    assert!(!rig.crt_list("lan").contains("client.lan.home.example.test"));
}

#[tokio::test]
async fn open_public_without_ingress_conflict() {
    let rig = Rig::start(true, false, true).await;
    let reply = rig.switch("public", true).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error_kind(), Some("conflict"));
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert_eq!(rig.reloads(), 0);
}

#[tokio::test]
async fn setup_pending_blocks_switch() {
    let rig = Rig::start(true, true, true).await;
    rig.h.docker.on_exec(|_, _, _| (1, String::new()));
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error_kind(), Some("setup_pending"));
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .lan
    );
    assert_eq!(rig.reloads(), 0);
}

#[tokio::test]
async fn public_switch_kept_while_public_access_off() {
    let rig = Rig::start(true, true, false).await;
    let reply = rig.switch("public", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["blocked"], "public_access_off");
    assert!(reply.json()["public_url"].is_null());
    assert!(
        rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
}

#[tokio::test]
async fn write_failure_keeps_switch_closed() {
    let rig = Rig::start(true, true, true).await;
    let before = std::fs::read(rig.install.join("entry/haproxy.cfg")).unwrap();
    // 原子写入要创建这个临时文件；目录占位让写入失败，但旧配置仍可读。
    std::fs::create_dir(rig.install.join("entry/certs/public.tmp")).unwrap();
    let reply = rig.switch("public", true).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("ingress_unavailable"));
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert_eq!(
        std::fs::read(rig.install.join("entry/haproxy.cfg")).unwrap(),
        before
    );
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
}

#[tokio::test]
async fn unavailable_lan_is_not_reported_as_open() {
    let rig = Rig::start(false, true, true).await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error_kind(), Some("lan_unavailable"));
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .lan
    );
}

#[tokio::test]
async fn opening_is_not_published_until_reload_succeeds() {
    let rig = Rig::start(true, true, true).await;
    let hold = rig.h.docker.hold("POST /containers/wonton-entry/kill");
    let gateway = rig.gateway.clone();
    let id = rig.id.clone();
    let opening = tokio::spawn(async move {
        crate::clients::apply::set_entry(&gateway, &id, crate::clients::state::Kind::Public, true)
            .await
            .map_err(|error| error.message)
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), hold.reached())
        .await
        .unwrap();
    let state = rig
        .gateway
        .clients
        .resolve("client", crate::clients::state::Kind::Public)
        .unwrap()
        .unwrap();
    assert!(!state.switches.public);
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    hold.release();
    opening.await.unwrap().unwrap();
    assert!(
        rig.gateway
            .clients
            .resolve("client", crate::clients::state::Kind::Public)
            .unwrap()
            .unwrap()
            .switches
            .public
    );
}

#[tokio::test]
async fn rejected_configuration_never_opens_entry() {
    let rig = Rig::start(true, true, true).await;
    rig.h.docker.on_exec(|_, command, _| {
        if command.first().is_some_and(|program| program == "haproxy") {
            (1, "invalid certificate".to_owned())
        } else {
            (0, String::new())
        }
    });
    let reply = rig.switch("public", true).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("ingress_unavailable"));
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .public
    );
    assert!(!rig.crt_list("public").contains("client.home.example.test"));
    assert_eq!(rig.reloads(), 0);
}

#[tokio::test]
async fn switched_entry_forwards_without_rewriting_host() {
    use crate::clients::state::Kind;
    use crate::gateway::tests::Reply;
    let rig = Rig::start(true, true, true).await;
    let request = || {
        hyper::Request::builder()
            .uri("/files?item=one")
            .header("host", "client.home.example.test:18443")
            .header("x-wonton-entry", "public")
            .body(axum::body::Body::empty())
            .unwrap()
    };
    let closed = Reply::of(
        crate::gateway::clients_entry::dispatch(
            &rig.gateway,
            request(),
            "client".into(),
            Kind::Public,
        )
        .await,
    )
    .await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
    assert_eq!(rig.switch("public", true).await.status, StatusCode::OK);
    let opened = Reply::of(
        crate::gateway::clients_entry::dispatch(
            &rig.gateway,
            request(),
            "client".into(),
            Kind::Public,
        )
        .await,
    )
    .await;
    assert_eq!(opened.status, StatusCode::OK);
    assert_eq!(
        opened.json()["headers"]["host"],
        "client.home.example.test:18443"
    );
    assert_eq!(
        opened.json()["headers"]["x-forwarded-host"],
        "client.home.example.test:18443"
    );
    assert!(opened.json()["headers"]["x-wonton-entry"].is_null());
    assert_eq!(rig.switch("public", false).await.status, StatusCode::OK);
    let closed = Reply::of(
        crate::gateway::clients_entry::dispatch(
            &rig.gateway,
            request(),
            "client".into(),
            Kind::Public,
        )
        .await,
    )
    .await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
}
