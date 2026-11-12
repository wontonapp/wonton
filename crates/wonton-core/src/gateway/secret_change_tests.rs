//! 改一项填写密钥（ADR-0064）：只有主人能改，值不进响应和审计，用到它的服务才重建。

use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;
use wonton_protocol::SecretString;

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use crate::config::PublicUrls;
use crate::gateway::Gateway;
use crate::gateway::tests::Reply;
use crate::mcp::audit::Audit;
use crate::plugin::PluginManager;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::names::container_name;
use crate::plugin::store::Store;

const PLUGIN: &str = "app.secret";

/// 两个服务：`web` 引用密钥，`worker` 不引用。
fn compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\n    environment:\n      TOKEN: ${{WONTON_SECRET_TOKEN}}\n  worker:\n    image: busybox\n\
         x-wonton:\n  spec: '0.8'\n  id: {PLUGIN}\n  version: 1.0.0\n  kind: app\n  title: {{zh: 密钥, en: Secret}}\n  \
         description: {{zh: 测试, en: Test}}\n  permissions: []\n  secrets:\n    token:\n      input:\n        title: {{zh: 令牌, en: Token}}\n    greeting:\n      input:\n        title: {{zh: 招呼, en: Greeting}}\n        required: false\n"
    )
}

struct Home {
    _dir: TempDir,
    docker: FakeDocker,
    app: Router,
}

async fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let (docker, engine) = FakeDocker::start().await;
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 8080,
    };
    let profile = owner_store(dir.path());
    let http = crate::http::HttpClient::new();
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
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
    let manager = Arc::new(
        PluginManager::new(
            engine,
            Store::open(dir.path().join("plugins")).unwrap(),
            http.clone(),
            None,
            lifecycle,
            ImageWorker::in_process(),
        )
        .with_lan(super::lan_api_tests::lan()),
    );
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members,
        manager.clone(),
        None,
        http,
        tunnel,
    )
    .with_audit(Some(Arc::new(Audit::new(dir.path().join("audit.jsonl")))));
    let app = super::router(Arc::new(gateway));
    let store = Store::open(dir.path().join("plugins")).unwrap();
    store
        .secrets()
        .save(
            &crate::plugin::PluginId::parse(PLUGIN).unwrap(),
            &[("token".to_owned(), b"old".to_vec())].into(),
        )
        .unwrap();
    manager.install(&compose(), &[], zh()).await.unwrap();
    Home {
        _dir: dir,
        docker,
        app,
    }
}

fn owner_store(dir: &std::path::Path) -> Arc<crate::owner::OwnerStore> {
    crate::owner::OwnerStore::for_test(dir, &owner())
}

fn zh() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

impl Home {
    async fn put(&self, name: &str, body: Value, bearer: Option<&str>) -> Reply {
        let mut request = hyper::Request::builder()
            .method(Method::PUT)
            .uri(format!(
                "http://{CORE}/_wonton/api/plugins/{PLUGIN}/secrets/{name}"
            ))
            .header(hyper::header::HOST, CORE)
            .header(hyper::header::CONTENT_TYPE, "application/json");
        if let Some(bearer) = bearer {
            request = request.header(hyper::header::AUTHORIZATION, bearer);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    fn audit(&self) -> Vec<Value> {
        let text =
            std::fs::read_to_string(self._dir.path().join("audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// 全进程一份的日志缓冲：装上以后所有日志都写进来，测试里查有没有漏值。
fn logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
            type Writer = Sink;
            fn make_writer(&'a self) -> Self::Writer {
                Sink(self.0.clone())
            }
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(Sink(buffer.clone()))
            .with_ansi(false)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        buffer
    })
    .clone()
}

#[tokio::test]
async fn the_value_never_reaches_the_logs() {
    let logs = logs();
    logs.lock().unwrap().clear();
    let home = home().await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = home
        .put(
            "token",
            serde_json::json!({"value": "log-only-secret"}),
            Some(&bearer),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let text = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    assert!(
        text.contains("secret_update"),
        "日志没装上或没有改动日志：{text}"
    );
    assert!(!text.contains("log-only-secret"), "{text}");
}

#[tokio::test]
async fn the_secret_api_is_owner_only() {
    let home = home().await;
    let reply = home
        .put("token", serde_json::json!({"value": "new"}), None)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_change_rebuilds_only_the_service_that_uses_it() {
    let home = home().await;
    let web = container_name("app-secret", "web");
    let worker = container_name("app-secret", "worker");
    let (web_before, worker_before) = (home.docker.creations(&web), home.docker.creations(&worker));
    let bearer = format!("Bearer {TOKEN}");
    let reply = home
        .put(
            "token",
            serde_json::json!({"value": "fresh"}),
            Some(&bearer),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        home.docker.creations(&web),
        web_before + 1,
        "用到它的服务重建"
    );
    assert_eq!(
        home.docker.creations(&worker),
        worker_before,
        "别的服务不动"
    );
    assert!(
        home.docker.env(&web).iter().any(|e| e == "TOKEN=fresh"),
        "{:?}",
        home.docker.env(&web)
    );
    let stored = Store::open(home._dir.path().join("plugins"))
        .unwrap()
        .secrets()
        .load(&crate::plugin::PluginId::parse(PLUGIN).unwrap())
        .unwrap();
    assert_eq!(
        stored
            .as_ref()
            .and_then(|s| s.get("token"))
            .map(Vec::as_slice),
        Some(&b"fresh"[..]),
        "存下来了：{stored:?}"
    );
    // 详情里有“已填写”和时间，没有值。
    let detail = reply.json();
    let token = detail["secret_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["name"] == "token")
        .unwrap();
    assert_eq!(token["filled"], true);
    assert!(token["changed_at_ms"].is_number());
    assert!(!detail.to_string().contains("fresh"), "{detail}");
    // 审计只记哪一项，不记值。
    let event = home.audit().pop().unwrap();
    assert_eq!(event["event"], "plugin_secret_change");
    assert_eq!(event["secret"], "token");
    assert!(!event.to_string().contains("fresh"), "{event}");
}

#[tokio::test]
async fn required_values_cannot_be_cleared_but_optional_ones_can() {
    let home = home().await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = home
        .put("token", serde_json::json!({"value": null}), Some(&bearer))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
    let reply = home
        .put(
            "greeting",
            serde_json::json!({"value": "hi"}),
            Some(&bearer),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = home
        .put(
            "greeting",
            serde_json::json!({"value": null}),
            Some(&bearer),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let detail = reply.json();
    let greeting = detail["secret_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["name"] == "greeting")
        .unwrap();
    assert_eq!(greeting["filled"], false);
}

#[tokio::test]
async fn an_unknown_name_is_invalid_input() {
    let home = home().await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = home
        .put("nope", serde_json::json!({"value": "x"}), Some(&bearer))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
}

#[tokio::test]
async fn the_value_never_shows_up_in_the_preview() {
    let home = home().await;
    let preview = home
        .app
        .clone()
        .oneshot(
            hyper::Request::builder()
                .method(Method::GET)
                .uri(format!("http://{CORE}/_wonton/api/plugins/{PLUGIN}"))
                .header(hyper::header::HOST, CORE)
                .header(hyper::header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let bytes = preview.into_body().collect().await.unwrap().to_bytes();
    let detail: Value = serde_json::from_slice(&bytes).unwrap();
    let token = detail["secret_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["name"] == "token")
        .unwrap();
    assert_eq!(token["filled"], true);
    assert!(
        !detail.to_string().contains("old"),
        "种子值不进详情：{detail}"
    );
}

#[test]
fn a_secret_value_is_redacted_in_debug() {
    assert_eq!(format!("{:?}", SecretString::new("s3cret")), "<redacted>");
}

/// 上传安装：值走 `Wonton-Plugin-Secrets` 头（base64url JSON），URL 里不出现值。
#[tokio::test]
async fn uploaded_installs_take_secrets_from_the_header() {
    use base64::Engine as _;
    let home = home().await;
    let compose = compose().replace("app.secret", "app.secret.upload");
    let package = crate::plugin::PluginPackage::parse(&compose, &zh()).unwrap();
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&serde_json::json!({"token": "s3cret-header"})).unwrap());
    let bearer = format!("Bearer {TOKEN}");
    let url = format!("http://{CORE}/_wonton/api/plugins?grant=");
    assert!(!url.contains("s3cret-header"));
    let reply = home
        .app
        .clone()
        .oneshot(
            hyper::Request::builder()
                .method(Method::POST)
                .uri(&url)
                .header(hyper::header::HOST, CORE)
                .header(hyper::header::AUTHORIZATION, &bearer)
                .header(hyper::header::CONTENT_TYPE, "application/x-tar")
                .header("wonton-plugin-secrets", &header)
                .body(Body::from(package.archive()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = reply.status();
    let bytes = reply.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let web = container_name("app-secret-upload", "web");
    assert!(
        home.docker
            .env(&web)
            .iter()
            .any(|e| e == "TOKEN=s3cret-header"),
        "{:?}",
        home.docker.env(&web)
    );
}

/// 头坏了：400，报错里只有解码位置，没有值。
#[tokio::test]
async fn a_broken_secrets_header_is_invalid_input() {
    let home = home().await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = home
        .app
        .clone()
        .oneshot(
            hyper::Request::builder()
                .method(Method::POST)
                .uri(format!("http://{CORE}/_wonton/api/plugins?grant="))
                .header(hyper::header::HOST, CORE)
                .header(hyper::header::AUTHORIZATION, &bearer)
                .header(hyper::header::CONTENT_TYPE, "application/yaml")
                .header("wonton-plugin-secrets", "!!!not-base64")
                .body(Body::from(compose()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    let bytes = reply.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("base64url"), "{text}");
    assert!(
        !text.contains("not-base64") || !text.contains("s3cret"),
        "{text}"
    );
}

/// 重建失败：值退回原样、容器还是旧值；同一个值再提一次也能重试（不会因为“没变化”早退）。
#[tokio::test]
async fn a_failed_change_rolls_back_and_can_be_retried() {
    let home = home().await;
    let web = container_name("app-secret", "web");
    let bearer = format!("Bearer {TOKEN}");
    home.docker.fail_starts(Some("启动失败"));
    let failed = home
        .put(
            "token",
            serde_json::json!({"value": "zz-rollback"}),
            Some(&bearer),
        )
        .await;
    assert!(!failed.status.is_success(), "{}", failed.text());
    let stored = Store::open(home._dir.path().join("plugins"))
        .unwrap()
        .secrets()
        .load(&crate::plugin::PluginId::parse(PLUGIN).unwrap())
        .unwrap();
    assert_eq!(
        stored
            .as_ref()
            .and_then(|s| s.get("token"))
            .map(Vec::as_slice),
        Some(&b"old"[..]),
        "值退回"
    );
    assert!(
        home.docker.env(&web).iter().any(|e| e == "TOKEN=old"),
        "{:?}",
        home.docker.env(&web)
    );
    home.docker.fail_starts(None);
    let retried = home
        .put(
            "token",
            serde_json::json!({"value": "zz-rollback"}),
            Some(&bearer),
        )
        .await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.text());
    assert!(
        home.docker
            .env(&web)
            .iter()
            .any(|e| e == "TOKEN=zz-rollback"),
        "{:?}",
        home.docker.env(&web)
    );
}
