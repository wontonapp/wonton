//! 网关转发静态网页的数据存储（ADR-0080 第 6 节）：去掉前缀转给提供方、写要同源、没声明的回 404、
//! 提供方停了如实回 503。提供方是测试里的假服务，契约本身由提供方的契约测试管。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;

use crate::gateway::fakes::{bind, serve};
use crate::gateway::tests::Harness;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, PluginId};
use crate::plugin::{PluginPackage, PluginStatus};

const HOST: &str = "io-github-alice-todo.wonton.localhost";

/// 提供方收到的（方法，路径和查询，插件 ID 头）。
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

async fn fake_provider(seen: Seen) -> String {
    let router = Router::new().fallback(move |req: Request| {
        let seen = seen.clone();
        async move {
            let id = req
                .headers()
                .get("x-wonton-plugin")
                .map(|v| v.to_str().unwrap().to_owned())
                .unwrap_or_default();
            let target = req.uri().to_string();
            seen.lock()
                .unwrap()
                .push((req.method().to_string(), target.clone(), id));
            (StatusCode::OK, format!("stored {target}")).into_response()
        }
    });
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr.to_string()
}

fn parse(source: &str) -> Manifest {
    PluginPackage::parse(source, &Languages::default())
        .unwrap()
        .manifest
}

fn provider(id: &str, capability: &str) -> Manifest {
    parse(&format!(
        "services:\n  s:\n    image: x\nx-wonton:\n  spec: '0.12'\n  id: {id}\n  version: 1.0.0\n  \
         kind: system\n  title: {{zh: 托管, en: Host}}\n  description: {{zh: 托管, en: Host}}\n  \
         permissions: []\n  provides:\n    {capability}:\n      service: s\n      port: 8080\n      \
         mount: /srv/x\n"
    ))
}

fn todo(storage: &str) -> Manifest {
    parse(&format!(
        "x-wonton:\n  spec: '0.12'\n  id: io.github.alice.todo\n  version: 1.0.0\n  kind: app\n  \
         runtime: static_web\n  title: {{zh: 待办, en: To-do}}\n  description: {{zh: 待办, en: To-do}}\n  \
         permissions: []\n{storage}"
    ))
}

struct Bed {
    h: Harness,
    cookie: String,
    pages: Seen,
    data: Seen,
}

/// 网页的提供方在运行；数据存储的提供方按 `storage_running`。
async fn bed(storage: &str, storage_running: bool) -> Bed {
    let h = Harness::start(None).await;
    let (pages, data) = (Seen::default(), Seen::default());
    let page_host = fake_provider(pages.clone()).await;
    let data_host = fake_provider(data.clone()).await;
    let manager = &h.manager;
    manager.register_hosted_provider_for_test(
        provider("app.wonton.pages", "static_web"),
        "static_web",
        Some(page_host),
    );
    manager.register_hosted_provider_for_test(
        provider("app.wonton.data", "web_storage"),
        "web_storage",
        storage_running.then_some(data_host),
    );
    manager.register_for_test(todo(storage), PluginStatus::Running, None);
    let cookie = h.app_session_on(HOST).await;
    Bed {
        h,
        cookie,
        pages,
        data,
    }
}

impl Bed {
    async fn send(
        &self,
        method: Method,
        path: &str,
        extra: &[(&str, &str)],
    ) -> crate::gateway::tests::Reply {
        let mut headers = vec![
            ("cookie", self.cookie.as_str()),
            ("accept", "application/json"),
        ];
        headers.extend_from_slice(extra);
        self.h
            .send(method, HOST, path, &headers, Body::from("[]"))
            .await
    }

    /// 带上插件自己的 `Origin`。
    async fn send_own(&self, method: Method, path: &str) -> crate::gateway::tests::Reply {
        let own = self.h.origin(HOST);
        self.send(method, path, &[("origin", &own)]).await
    }

    fn data_seen(&self) -> Vec<(String, String, String)> {
        self.data.lock().unwrap().clone()
    }
}

// 页面写 /_wonton/data/…：前缀去掉、查询串留着、插件 ID 头由核心写，响应照样盖上网页的 CSP，不进缓存。
#[tokio::test]
async fn data_requests_reach_the_storage_provider_without_the_prefix() {
    let bed = bed("  requires: [web_storage]\n", true).await;
    let put = bed
        .send_own(Method::PUT, "/_wonton/data/todo.json?v=2")
        .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.text());
    let get = bed.send(Method::GET, "/_wonton/data/todo.json", &[]).await;
    assert_eq!(get.text(), "stored /todo.json");
    assert_eq!(
        bed.data_seen(),
        [
            (
                "PUT".into(),
                "/todo.json?v=2".into(),
                "io.github.alice.todo".into()
            ),
            (
                "GET".into(),
                "/todo.json".into(),
                "io.github.alice.todo".into()
            ),
        ]
    );
    let csp = get.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("default-src 'self'"), "{csp}");
    assert_eq!(get.headers["cache-control"], "no-store");
    assert!(
        bed.pages.lock().unwrap().is_empty(),
        "数据请求不经网页的提供方"
    );
}

// 写要同源：别的网页的写在会话这一关就被当成没登录（401），没带 Origin 的写到这里回 403；
// 只读的不看 Origin，浏览器自己标了同源的照样能写。
#[tokio::test]
async fn writes_need_the_plugins_own_origin() {
    let bed = bed("  wants: [web_storage]\n", true).await;
    let other = "http://io-github-mallory-x.wonton.localhost";
    let mkcol = Method::from_bytes(b"MKCOL").unwrap();
    for (method, origin) in [
        (Method::PUT, other),
        (Method::DELETE, other),
        (mkcol.clone(), other),
    ] {
        let reply = bed
            .send(
                method.clone(),
                "/_wonton/data/a.json",
                &[("origin", origin)],
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method}");
    }
    let r#move = Method::from_bytes(b"MOVE").unwrap();
    for method in [Method::PUT, Method::DELETE, mkcol, r#move, Method::POST] {
        let reply = bed.send(method.clone(), "/_wonton/data/a.json", &[]).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(reply.error_kind(), Some("cross_site"));
    }
    assert!(bed.data_seen().is_empty());

    let propfind = Method::from_bytes(b"PROPFIND").unwrap();
    assert_eq!(
        bed.send(propfind, "/_wonton/data/", &[]).await.status,
        StatusCode::OK
    );
    let own = bed.h.origin(HOST);
    let by_origin = bed
        .send(Method::DELETE, "/_wonton/data/a.json", &[("origin", &own)])
        .await;
    assert_eq!(by_origin.status, StatusCode::OK);
    let marked = [("sec-fetch-site", "same-origin")];
    let by_fetch_metadata = bed.send(Method::PUT, "/_wonton/data/a.json", &marked).await;
    assert_eq!(by_fetch_metadata.status, StatusCode::OK);
    assert_eq!(bed.data_seen().len(), 3);
}

// 没声明数据存储的静态网页：这个路径回 404，不转给任何提供方。
#[tokio::test]
async fn pages_without_storage_get_not_found() {
    let bed = bed("", true).await;
    let reply = bed.send(Method::GET, "/_wonton/data/a.json", &[]).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), Some("not_found"));
    assert!(bed.data_seen().is_empty());
    assert!(bed.pages.lock().unwrap().is_empty());
}

// 数据存储的提供方停了：数据路径回 503；只是 wants 时网页本身照常，requires 时网页标异常。
#[tokio::test]
async fn a_stopped_storage_provider_is_reported_as_not_ready() {
    let id = PluginId::parse("io.github.alice.todo").unwrap();
    let status = |bed: &Bed| {
        let plugin = bed
            .h
            .manager
            .installed_plugin(&id, &Languages::default())
            .unwrap();
        serde_json::to_value(&plugin.status).unwrap()["state"].clone()
    };
    let wants = bed("  wants: [web_storage]\n", false).await;
    let reply = wants.send(Method::GET, "/_wonton/data/a.json", &[]).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("not_ready"));
    assert_eq!(
        wants.send(Method::GET, "/", &[]).await.status,
        StatusCode::OK
    );
    assert_eq!(status(&wants), json!("running"));

    let requires = bed("  requires: [web_storage]\n", false).await;
    assert_eq!(status(&requires), json!("degraded"));
}
