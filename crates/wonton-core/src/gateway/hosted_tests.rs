//! 网关转发静态网页（ADR-0080）：和提供方无关的那一侧——插件 ID 头只由核心写、CSP 由核心算、
//! `fallback` 由网关补取、提供方停了如实回 503。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::gateway::fakes::{bind, serve};
use crate::gateway::tests::Harness;
use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::PluginId;

const HOST: &str = "io-github-alice-snake.wonton.localhost";

/// 提供方收到的（路径，插件 ID 头的各个值）。
type Seen = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// 假提供方：收到的插件 ID 头和路径原样记下；`/missing` 这类路径回 404，其余回 200，还故意带一个宽松的 CSP。
async fn fake_provider(seen: Seen) -> String {
    let router = Router::new().fallback(move |req: Request| {
        let seen = seen.clone();
        async move {
            let ids = req
                .headers()
                .get_all("x-wonton-plugin")
                .iter()
                .map(|v| v.to_str().unwrap().to_owned())
                .collect();
            let path = req.uri().path().to_owned();
            seen.lock().unwrap().push((path.clone(), ids));
            let status = if path.starts_with("/missing") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            };
            let mut response: Response = (status, format!("served {path}")).into_response();
            response.headers_mut().insert(
                "content-security-policy",
                HeaderValue::from_static("default-src *"),
            );
            response
        }
    });
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr.to_string()
}

fn provider_manifest() -> crate::plugin::manifest::Manifest {
    let source = "services:\n  static:\n    image: caddy\nx-wonton:\n  spec: '0.12'\n  id: app.wonton.static-host\n  \
                  version: 1.0.0\n  kind: system\n  title: {zh: 静态托管, en: Static host}\n  \
                  description: {zh: 托管, en: Hosting}\n  permissions: []\n  provides:\n    static_web:\n      \
                  service: static\n      port: 8080\n      mount: /srv/static\n";
    PluginPackage::parse(source, &Languages::default())
        .unwrap()
        .manifest
}

fn snake(extra: &str) -> crate::plugin::manifest::Manifest {
    let source = format!(
        "x-wonton:\n  spec: '0.12'\n  id: io.github.alice.snake\n  version: 1.0.0\n  kind: app\n  \
         runtime: static_web\n  title: {{zh: 贪吃蛇, en: Snake}}\n  description: {{zh: 贪吃蛇, en: Snake}}\n  \
         webui: {{path: /}}\n{extra}"
    );
    PluginPackage::parse(&source, &Languages::default())
        .unwrap()
        .manifest
}

async fn with_provider(extra: &str, running: bool) -> (Harness, String, Seen) {
    let h = Harness::start(None).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = fake_provider(seen.clone()).await;
    h.manager.register_hosted_provider_for_test(
        provider_manifest(),
        "static_web",
        running.then_some(provider),
    );
    h.manager
        .register_for_test(snake(extra), crate::plugin::PluginStatus::Running, None);
    let cookie = h.app_session_on(HOST).await;
    (h, cookie, seen)
}

// 页面自己带的 X-Wonton-* 到不了提供方：提供方只看到核心写的那一个，值是这个插件的 ID。
#[tokio::test]
async fn the_provider_only_sees_the_plugin_header_the_core_wrote() {
    let (h, cookie, seen) = with_provider("  permissions: []\n", true).await;
    let page = h
        .get(
            HOST,
            "/game.js",
            &[
                ("cookie", &cookie),
                ("x-wonton-plugin", "io.github.mallory.steal"),
                ("x-wonton-other", "x"),
            ],
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.text(), "served /game.js");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen.last().unwrap(),
        &(
            "/game.js".to_owned(),
            vec!["io.github.alice.snake".to_owned()]
        )
    );
}

// 响应头由核心按清单定：提供方给的宽松 CSP 被盖掉，声明的来源对各类资源放行，没声明的连不出去。
#[tokio::test]
async fn the_gateway_csp_replaces_the_providers_and_lists_declared_origins() {
    let extra = "  permissions: [network.egress]\n  static_web:\n    connect: [https://api.open-meteo.com]\n";
    let (h, cookie, _) = with_provider(extra, true).await;
    let page = h.get(HOST, "/", &[("cookie", &cookie)]).await;
    let csp = page.headers["content-security-policy"].to_str().unwrap();
    assert_eq!(
        page.headers
            .get_all("content-security-policy")
            .iter()
            .count(),
        1
    );
    assert!(csp.starts_with("default-src 'self'"), "{csp}");
    assert!(
        csp.contains("connect-src 'self' https://api.open-meteo.com"),
        "{csp}"
    );
    assert!(
        csp.contains("frame-src https://api.open-meteo.com"),
        "{csp}"
    );
    assert_eq!(page.headers["x-content-type-options"], "nosniff");

    let (h, cookie, _) = with_provider("  permissions: []\n", true).await;
    let page = h.get(HOST, "/", &[("cookie", &cookie)]).await;
    let csp = page.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("connect-src 'self';"), "{csp}");
    assert!(csp.contains("frame-src 'none'"), "{csp}");
}

// 单页应用：找不到的路径回 fallback；没写 fallback 的照实 404；不是 GET 的不补取。
#[tokio::test]
async fn missing_paths_fall_back_only_when_the_manifest_says_so() {
    let extra = "  permissions: []\n  static_web:\n    fallback: index.html\n";
    let (h, cookie, seen) = with_provider(extra, true).await;
    let page = h.get(HOST, "/missing/route", &[("cookie", &cookie)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.text(), "served /index.html");
    let paths: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    assert_eq!(paths, ["/missing/route", "/index.html"]);
    assert!(page.headers.contains_key("content-security-policy"));

    let (h, cookie, _) = with_provider("  permissions: []\n", true).await;
    let page = h.get(HOST, "/missing/route", &[("cookie", &cookie)]).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
}

// 提供方没在运行：503，状态是“依赖的插件没有在运行”，不显示成静态网页自己坏了。
#[tokio::test]
async fn a_stopped_provider_answers_not_ready_and_marks_the_page_degraded() {
    let (h, cookie, seen) = with_provider("  permissions: []\n", false).await;
    let page = h
        .get(
            HOST,
            "/",
            &[("cookie", &cookie), ("accept", "application/json")],
        )
        .await;
    assert_eq!(page.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(page.error_kind(), Some("not_ready"));
    assert!(seen.lock().unwrap().is_empty());
    let id = PluginId::parse("io.github.alice.snake").unwrap();
    let snake = h
        .manager
        .installed_plugin(&id, &Languages::default())
        .unwrap();
    let status = serde_json::to_value(&snake.status).unwrap();
    assert_eq!(status["state"], json!("degraded"), "{status}");
}

// 没有会话照旧要登录：访问控制在网关，和容器插件的网页一样，提供方不鉴权。
#[tokio::test]
async fn static_pages_still_need_a_session() {
    let (h, _, seen) = with_provider("  permissions: []\n", true).await;
    let page = h.get(HOST, "/", &[("accept", "application/json")]).await;
    assert_eq!(page.status, StatusCode::UNAUTHORIZED);
    assert!(seen.lock().unwrap().is_empty());
}
