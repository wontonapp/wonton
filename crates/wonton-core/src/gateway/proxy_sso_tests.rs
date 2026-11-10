//! 网关代登录（`sso: proxy`）的行为：只给主人在插件自己域名上的请求代加凭据，浏览器伪造不了、也拿不到。
//! 经隧道的已配对设备见 `tunnel_tests`。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::Request;
use axum::response::IntoResponse;
use axum::{Json, Router};
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::session;
use super::tests::{CORE, Harness, PLUGIN};
use crate::plugin::languages::Languages;
use crate::plugin::proxy_auth::ProxyAuth;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};

/// 代登录的示例插件：网关给它的网页代加一个请求头和一个 Cookie，Cookie 的值是插件的网页密钥。
const VAULT: &str = "name: vault
services:
  web:
    image: example/vault
x-wonton:
  spec: '0.4'
  id: app.vault
  version: 0.1.0
  kind: app
  title: {zh: 保险箱, en: Vault}
  description: {zh: 网关代登录的示例, en: Proxy sign-in example}
  webui:
    service: web
    port: 80
    sso: proxy
    proxy:
      headers: {Remote-User: '{user}', Remote-Email: '{email}'}
      cookies: {vault_web: '{secret:web_key}'}
  secrets:
    web_key: {generate: {bytes: 32}}
  permissions: []
";

pub(super) const VAULT_HOST: &str = "app-vault.wonton.localhost";

/// 核心为它生成的网页密钥。
pub(super) const KEY: &str = "4f1c0de5a7b2e9d3c6a8f0b1e2d3c4b5a69788796a5b4c3d2e1f0a9b8c7d6e5f";

/// 登记代登录的示例插件，上游是 [`upstream`]。返回上游收到的请求数。
pub(super) async fn register_vault(manager: &PluginManager) -> Arc<AtomicUsize> {
    let hits = Arc::new(AtomicUsize::new(0));
    let addr = upstream(hits.clone()).await;
    let manifest = PluginPackage::parse(VAULT, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let generated = BTreeMap::from([("web_key".to_owned(), KEY.to_owned())]);
    let auth = ProxyAuth::resolve(
        &manifest,
        &generated,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
    .unwrap();
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, Some(addr.to_string()));
    manager.proxy_for_test(&id, auth);
    hits
}

/// 假插件：按顺序回显收到的全部请求头（重复的也在）。`/leak` 还像常见的网页那样把收到的 `vault_web`
/// 写回响应：续期会话的 `Set-Cookie`、带着它的跳转地址和调试头。
async fn upstream(hits: Arc<AtomicUsize>) -> SocketAddr {
    let router = Router::new().fallback(move |req: Request| {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            let received: Vec<(String, String)> = req
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into()))
                .collect();
            let mut response = Json(json!({"headers": received})).into_response();
            if req.uri().path() == "/leak" {
                let key = session::cookies(req.headers(), "vault_web").next();
                let key = key.unwrap_or_default();
                let headers = response.headers_mut();
                for (name, value) in [
                    ("set-cookie", format!("vault_web={key}; Path=/; HttpOnly")),
                    ("set-cookie", format!("VAULT_WEB={key}")),
                    ("set-cookie", "=vault_web=x".to_owned()),
                    ("set-cookie", format!("backup={key}")),
                    ("set-cookie", "theme=dark; Path=/".to_owned()),
                    ("location", format!("/?k={key}")),
                    ("x-session", format!("Bearer {key}")),
                    ("x-plain", "hello".to_owned()),
                ] {
                    headers.append(name, value.parse().unwrap());
                }
            }
            response
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

/// 上游收到的名为 `name` 的每一个请求头。
pub(super) fn received(body: &str, name: &str) -> Vec<String> {
    let body: Value = serde_json::from_str(body).unwrap();
    body["headers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str().unwrap().to_owned())
        .collect()
}

/// 响应头里带着密钥的。
pub(super) fn leaked(headers: &HeaderMap) -> Vec<String> {
    headers
        .iter()
        .filter(|(_, v)| String::from_utf8_lossy(v.as_bytes()).contains(KEY))
        .map(|(k, v)| format!("{k}: {v:?}"))
        .collect()
}

#[tokio::test]
async fn the_owner_is_signed_in_with_credentials_the_browser_cannot_forge() {
    let h = Harness::start(None).await;
    let hits = register_vault(&h.manager).await;
    let session = h.app_session_on(VAULT_HOST).await;
    let cookie = format!("{session}; vault_web=forged; theme=dark; Vault_Web=forged-too");
    let headers = [
        ("cookie", cookie.as_str()),
        ("remote-user", "mallory"),
        ("Remote-User", "eve"),
    ];
    let reply = h.get(VAULT_HOST, "/settings", &headers).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json().to_string();
    // 同名的 Cookie 和请求头（不分大小写）都换掉，只剩网关加的一份；网关自己的会话照旧去掉。
    assert_eq!(
        received(&body, "cookie"),
        [format!("theme=dark; vault_web={KEY}")]
    );
    assert_eq!(received(&body, "remote-user"), ["owner"]);
    assert_eq!(received(&body, "remote-email"), ["owner@wonton.internal"]);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_credentials_never_reach_the_browser() {
    let h = Harness::start(None).await;
    register_vault(&h.manager).await;
    let session = h.app_session_on(VAULT_HOST).await;
    let reply = h.get(VAULT_HOST, "/leak", &[("cookie", &session)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(leaked(&reply.headers).is_empty(), "{:?}", reply.headers);
    // 设置代加 Cookie 的（不分大小写，包括没有名字、浏览器读成同名的）都去掉，其余的保留。
    let set_cookies: Vec<&str> = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(set_cookies, ["theme=dark; Path=/"]);
    assert!(reply.headers.get(header::LOCATION).is_none());
    assert_eq!(reply.headers["x-plain"], "hello");
}

#[tokio::test]
async fn only_the_owner_on_the_plugins_own_pages_gets_proxy_credentials() {
    let h = Harness::start(None).await;
    let hits = register_vault(&h.manager).await;
    // 没登录：打开页面跳到核心登录，其余请求 401，都不转发。
    let page = h.get(VAULT_HOST, "/", &[("accept", "text/html")]).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert!(
        page.location().starts_with(&h.origin(CORE)),
        "{}",
        page.location()
    );
    let guess = [("cookie", "vault_web=guess")];
    let api = h.get(VAULT_HOST, "/api/settings", &guess).await;
    assert_eq!(api.status, StatusCode::UNAUTHORIZED);
    // 别的插件的会话在这里不算数。
    let whoami = h.app_session().await;
    let other = h
        .get(VAULT_HOST, "/api/settings", &[("cookie", &whoami)])
        .await;
    assert_eq!(other.status, StatusCode::UNAUTHORIZED);
    // 主人的会话，但请求来自别的插件的页面：按没登录处理。
    let session = h.app_session_on(VAULT_HOST).await;
    let fetch = [
        ("cookie", session.as_str()),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "cors"),
    ];
    let cross = h.get(VAULT_HOST, "/api/settings", &fetch).await;
    assert_eq!(cross.status, StatusCode::UNAUTHORIZED);
    let origin = h.origin(PLUGIN);
    let form = [("cookie", session.as_str()), ("origin", origin.as_str())];
    let post = h
        .send(
            Method::POST,
            VAULT_HOST,
            "/api/settings",
            &form,
            Default::default(),
        )
        .await;
    assert_eq!(post.status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "没认出主人的请求不转发");
    // 别的插件的网页收不到这个插件的凭据。
    let seen = h.get(PLUGIN, "/", &[("cookie", &whoami)]).await.json();
    assert!(!seen.to_string().contains(KEY), "{seen}");
    assert!(seen["headers"].get("remote-user").is_none(), "{seen}");
}

#[tokio::test]
async fn the_plugins_own_network_reaches_it_without_a_session_or_identity() {
    let h = Harness::start(None).await;
    let hits = register_vault(&h.manager).await;
    h.manager
        .register_source_for_test("app.vault", "127.0.0.1/32", vec![]);
    // 容器自己发来的请求带什么同名请求头和 Cookie 都剥掉：插件后端可能转发的是某个浏览器的请求，
    // 那套值属于别人，不是网关给它的。
    let reply = h
        .get(
            VAULT_HOST,
            "/",
            &[("remote-user", "forged"), ("cookie", "vault_web=forged")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(!reply.text().contains("forged"), "{}", reply.text());
}
