//! 主人查看插件密钥接口的行为测试（#339）：能列出、能取值；Bot、插件、未登录都拒，网页跨站也拒；
//! 错误按原因分；取值写审计且审计里没有值。

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use http_body_util::BodyExt;
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, ORIGIN};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fakes::loopback_tunnel;
use super::files_tests::Reply;
use super::tests::{CORE, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::mcp::audit::Audit;
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};

const PLUGIN: &str = "app.secrets.demo";
const LIST: &str = "/_wonton/api/plugins/app.secrets.demo/secrets";
const VALUE: &str = "/_wonton/api/plugins/app.secrets.demo/secrets/admin_password";
/// 给主人看的有两项（一项有值一项还没有），不给主人看的有 generate 和 collect 各一项。
const COMPOSE: &str = r#"
name: secrets-demo
services:
  app:
    image: traefik/whoami:v1.11
x-wonton:
  spec: '0.4'
  id: app.secrets.demo
  version: 0.1.0
  kind: app
  title: { zh: 密钥演示, en: Secrets demo }
  description: { zh: 测试主人查看密钥, en: For secret viewing tests }
  permissions: []
  secrets:
    admin_password:
      generate: { bytes: 32 }
      show_owner: { label: { zh: 管理员密码, en: Admin password, ja: 管理者パスワード } }
    db_password:
      generate: {}
      show_owner: { label: { zh: 数据库密码, en: Database password } }
    web_key:
      generate: { bytes: 16 }
    recovery_key:
      collect: { service: app, path: /tmp/recovery.txt }
"#;
const SECRET_VALUE: &str = "s3cret-value";

struct Home {
    app: Router,
    dir: tempfile::TempDir,
    device: TunnelPeer,
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
    let manifest = PluginPackage::manifest_of(
        COMPOSE,
        &crate::plugin::languages::Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    assert_eq!(manifest.id.as_str(), PLUGIN);
    manager.secrets_for_test(
        &manifest.id,
        &[("admin_password", SECRET_VALUE), ("web_key", "web-value")],
    );
    manager.register_for_test(manifest, PluginStatus::Running, None);
    let tunnel_devices = loopback_tunnel(dir.path()).await;
    let (tunnel, members) = tunnel_devices;
    let key = SecretKey::generate();
    tunnel.devices.add(key.public(), None).unwrap();
    let audit = Audit::new(dir.path().join("audit.jsonl"));
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members,
        manager,
        None,
        HttpClient::new(),
        tunnel,
    )
    .with_audit(Some(Arc::new(audit)));
    Home {
        app: router(Arc::new(gateway)),
        dir,
        device: TunnelPeer(key.public()),
    }
}

impl Home {
    async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: body.to_vec(),
        }
    }

    fn request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("{CORE}:18480"))
    }

    /// 主人令牌。
    async fn get(&self, path: &str, language: &str) -> Reply {
        self.send(
            self.request(Method::GET, path)
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(ACCEPT_LANGUAGE, language)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    /// 已配对设备（App 的身份），不带别的头。
    async fn get_as_device(&self, path: &str, origin: Option<&str>) -> Reply {
        let mut request = self.request(Method::GET, path).extension(self.device);
        if let Some(origin) = origin {
            request = request.header(ORIGIN, origin);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    fn audit_lines(&self) -> Vec<Value> {
        audit_lines(self.dir.path())
    }
}

fn audit_lines(dir: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn owner_lists_declared_secrets_without_values() {
    let h = home().await;
    let reply = h.get(LIST, "zh-CN").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header("cache-control"), "no-store");
    // 只列声明了的项，按名字序，label 按请求的语言挑好；值不出现在列表里。
    assert_eq!(
        reply.json(),
        json!({"secrets": [
            {"name": "admin_password", "label": "管理员密码"},
            {"name": "db_password", "label": "数据库密码"}
        ]})
    );
    let english = h.get(LIST, "en").await;
    assert_eq!(english.json()["secrets"][0]["label"], "Admin password");
}

#[tokio::test]
async fn owner_gets_a_declared_value() {
    let h = home().await;
    let reply = h.get(VALUE, "zh").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header("cache-control"), "no-store");
    assert_eq!(reply.json(), json!({"value": SECRET_VALUE}));
}

#[tokio::test]
async fn other_identities_are_rejected() {
    let h = home().await;
    for path in [LIST, VALUE] {
        // 什么身份都没有。
        let reply = h
            .send(h.request(Method::GET, path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(reply.error_kind(), "unauthenticated");
        // Bot 或插件：不是主人的令牌，一律按未登录拒。
        let reply = h
            .send(
                h.request(Method::GET, path)
                    .header(AUTHORIZATION, "Bearer some-bot-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(reply.error_kind(), "unauthenticated");
    }
}

#[tokio::test]
async fn paired_device_can_read_but_not_from_another_page() {
    let h = home().await;
    // App 经隧道以设备身份读，同主人。
    let reply = h.get_as_device(VALUE, None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({"value": SECRET_VALUE}));
    // 插件网页带着设备身份发来的请求：权限不足。
    let reply = h.get_as_device(VALUE, Some("https://other.example")).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), "cross_site");
}

#[tokio::test]
async fn undeclared_collect_and_missing_plugin_have_distinct_errors() {
    let h = home().await;
    // generate 但没声明 show_owner；错误响应同样不缓存。
    let reply = h
        .get(
            "/_wonton/api/plugins/app.secrets.demo/secrets/web_key",
            "zh",
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "not_found");
    assert_eq!(reply.header("cache-control"), "no-store");
    // collect 项是插件交给核心的凭据，声明不了。
    let reply = h
        .get(
            "/_wonton/api/plugins/app.secrets.demo/secrets/recovery_key",
            "zh",
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "not_found");
    // 清单里没有的名字。
    let reply = h
        .get("/_wonton/api/plugins/app.secrets.demo/secrets/nope", "zh")
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "not_found");
    // 插件没装。
    for path in [
        "/_wonton/api/plugins/app.missing/secrets",
        "/_wonton/api/plugins/app.missing/secrets/admin_password",
    ] {
        let reply = h.get(path, "zh").await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.error_kind(), "not_installed");
    }
}

#[tokio::test]
async fn value_not_generated_yet_reports_not_ready() {
    let h = home().await;
    // 声明了 show_owner，但密钥文件里还没有这一项（密钥丢失后插件没启动）。
    let reply = h
        .get(
            "/_wonton/api/plugins/app.secrets.demo/secrets/db_password",
            "zh",
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "not_ready");
}

#[tokio::test]
async fn value_fetch_is_audited_without_the_value() {
    let h = home().await;
    let reply = h.get(VALUE, "zh").await;
    assert_eq!(reply.status, StatusCode::OK);
    let lines = h.audit_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["event"], "plugin_secret_view");
    assert_eq!(lines[0]["by"], "owner_token");
    assert_eq!(lines[0]["plugin"], PLUGIN);
    assert_eq!(lines[0]["secret"], "admin_password");
    // 审计里没有值，也不进任何字段。
    assert!(lines[0].get("value").is_none());
    let raw = std::fs::read_to_string(h.dir.path().join("audit.jsonl")).unwrap();
    assert!(!raw.contains(SECRET_VALUE), "{raw}");
    // 列表不写审计。
    assert!(h.get(LIST, "zh").await.status.is_success());
    assert_eq!(h.audit_lines().len(), 1, "列表不该写审计");
}

#[tokio::test]
async fn device_fetch_is_audited_with_the_device_id() {
    let h = home().await;
    let reply = h.get_as_device(VALUE, None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let lines = h.audit_lines();
    assert_eq!(lines[0]["by"], format!("owner/device:{}", h.device.0));
}
