//! 网关转给插件网页的“打开者”（`X-Wonton-Web-User`）：只由核心按会话写，客户端带的一律剥掉。
//! 插件自己网络里的容器、没登录、没授权、OIDC 回调都不带；Bot 页和普通插件页两条路都要按这个来。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hyper::StatusCode;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::WEB_USER_HEADER;
use super::bot_web_tests::{bot, host, member, open_whoami, setup, source};
use super::fakes::{bind, serve};
use super::family_tests::send_as;
use super::tests::{Harness, PLUGIN, TOKEN, WHOAMI};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginPackage, PluginStatus};

/// 上游收到的请求：路径和全部请求头（重名的也在）。
type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

const OWNER_ID: &str = "@owner:wonton.localhost";
const MIA_ID: &str = "@mia:wonton.localhost";
/// 客户端自己塞的，核心认的身份不是它。
const FORGED: &str = "@mallory:wonton.localhost";

fn pairs(req: &Request) -> Vec<(String, String)> {
    req.headers()
        .iter()
        .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into()))
        .collect()
}

/// 记下每个请求的路径和头；`/ws` 像插件网页那样完成升级。
async fn capture(seen: Seen) -> SocketAddr {
    let ws_seen = seen.clone();
    let router = Router::new()
        .route(
            "/ws",
            get(move |mut req: Request| {
                let seen = ws_seen.clone();
                async move {
                    seen.lock()
                        .unwrap()
                        .push((req.uri().path().to_owned(), pairs(&req)));
                    let on_upgrade = hyper::upgrade::on(&mut req);
                    tokio::spawn(async move {
                        if let Ok(upgraded) = on_upgrade.await {
                            let mut io = hyper_util::rt::TokioIo::new(upgraded);
                            let mut buf = [0u8; 64];
                            if let Ok(n) = io.read(&mut buf).await {
                                let _ = io.write_all(&buf[..n]).await;
                            }
                        }
                    });
                    Response::builder()
                        .status(StatusCode::SWITCHING_PROTOCOLS)
                        .header("connection", "upgrade")
                        .header("upgrade", "echo")
                        .body(axum::body::Body::empty())
                        .unwrap()
                }
            }),
        )
        .fallback(move |req: Request| {
            let seen = seen.clone();
            async move {
                let path = req.uri().path().to_owned();
                seen.lock().unwrap().push((path.clone(), pairs(&req)));
                Json(json!({"path": path, "headers": pairs(&req)})).into_response()
            }
        });
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr
}

/// 这条路径上收到的 `name` 头的全部值。
fn seen_headers(seen: &Seen, path: &str, name: &str) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(seen_path, _)| seen_path == path)
        .flat_map(|(_, headers)| headers)
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
        .collect()
}

/// 客户端伪造的两条同名头：值都不是核心认的人。
fn forged() -> [(&'static str, &'static str); 2] {
    [(WEB_USER_HEADER, FORGED), (WEB_USER_HEADER, "eve")]
}

/// 把普通插件页换成记录请求的上游。
async fn plain(h: &Harness) -> Seen {
    let seen = Seen::default();
    let addr = capture(seen.clone()).await;
    h.manager.register_for_test(
        PluginPackage::manifest_of(&source(), &Languages::default()).unwrap(),
        PluginStatus::Running,
        Some(addr.to_string()),
    );
    seen
}

/// 把 Bot 页（`default`）换成记录请求的上游。
async fn bot_page(h: &Harness) -> Seen {
    let other = setup(h).await;
    drop(other);
    let seen = Seen::default();
    let addr = capture(seen.clone()).await;
    h.manager.bot_web_route_for_test(
        bot_ref(),
        crate::plugin::upstreams::Endpoint::Tcp(addr.to_string()),
        true,
    );
    seen
}

#[tokio::test]
async fn a_forged_header_is_replaced_on_a_plugin_page() {
    let h = Harness::start(None).await;
    let seen = plain(&h).await;
    let session = h.app_session().await;
    let mut headers = forged().to_vec();
    headers.push(("cookie", session.as_str()));
    let reply = h.get(PLUGIN, "/hello", &headers).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    // 客户端那两条都剥掉，上游只能看到核心写的那一条。
    assert_eq!(seen_headers(&seen, "/hello", WEB_USER_HEADER), [OWNER_ID]);
}

#[tokio::test]
async fn a_forged_header_is_replaced_on_a_bot_page() {
    let h = Harness::start(None).await;
    let seen = bot_page(&h).await;
    let host = host("default");
    let session = h.app_session_on(&host).await;
    let mut headers = forged().to_vec();
    headers.push(("cookie", session.as_str()));
    let reply = h.get(&host, "/panel", &headers).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(seen_headers(&seen, "/panel", WEB_USER_HEADER), [OWNER_ID]);
}

#[tokio::test]
async fn a_family_member_with_the_app_open_gets_her_id_on_a_plugin_page() {
    let h = Harness::start(None).await;
    // 没有 Bot 记录时，插件自己的地址就是这个插件网页的地址。
    let seen = plain(&h).await;
    let device = member(&h);
    open_whoami(&h);
    let reply = send_as(
        &h.app,
        Some(device),
        hyper::Method::GET,
        PLUGIN,
        "/hello",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(seen_headers(&seen, "/hello", WEB_USER_HEADER), [MIA_ID]);
}

#[tokio::test]
async fn a_family_member_with_the_app_open_gets_her_id_on_a_bot_page() {
    let h = Harness::start(None).await;
    let seen = bot_page(&h).await;
    let device = member(&h);
    open_whoami(&h);
    let host = host("default");
    let reply = send_as(
        &h.app,
        Some(device),
        hyper::Method::GET,
        &host,
        "/panel",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(seen_headers(&seen, "/panel", WEB_USER_HEADER), [MIA_ID]);
}

fn bot_ref() -> wonton_protocol::BotRef {
    bot("default")
}

#[tokio::test]
async fn a_family_member_without_the_app_never_reaches_the_plugin() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let plain_seen = plain(&h).await;
    let device = member(&h);
    let bot_host = host("default");
    let bot_seen = Seen::default();
    h.manager.bot_web_route_for_test(
        bot_ref(),
        crate::plugin::upstreams::Endpoint::Tcp(capture(bot_seen.clone()).await.to_string()),
        true,
    );
    for (target, path) in [(PLUGIN, "/hello"), (bot_host.as_str(), "/panel")] {
        let reply = send_as(
            &h.app,
            Some(device),
            hyper::Method::GET,
            target,
            path,
            &[(WEB_USER_HEADER, FORGED)],
            "",
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{target} {}",
            reply.text()
        );
    }
    assert!(plain_seen.lock().unwrap().is_empty());
    assert!(bot_seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_plugins_own_network_gets_no_header() {
    let h = Harness::start(None).await;
    let seen = plain(&h).await;
    h.manager
        .register_source_for_test("app.whoami", "127.0.0.0/8", vec![]);
    let reply = h
        .get(PLUGIN, "/internal", &[(WEB_USER_HEADER, FORGED)])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(seen_headers(&seen, "/internal", WEB_USER_HEADER).is_empty());
}

#[tokio::test]
async fn an_oidc_callback_gets_no_header() {
    let h = Harness::start(None).await;
    let seen = plain(&h).await;
    // 认得出人也照旧不带：应用在回调里自己换凭据。
    let session = h.app_session().await;
    let reply = h
        .send(
            hyper::Method::POST,
            PLUGIN,
            "/cb",
            &[
                ("content-type", "application/x-www-form-urlencoded"),
                ("cookie", session.as_str()),
                (WEB_USER_HEADER, FORGED),
            ],
            axum::body::Body::from("code=fixture-code&state=fixture-state"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(seen_headers(&seen, "/cb", WEB_USER_HEADER).is_empty());
}

#[tokio::test]
async fn a_websocket_upgrade_carries_the_header() {
    let h = Harness::start(None).await;
    let seen = bot_page(&h).await;
    let host = host("default");
    let session = h.app_session_on(&host).await;
    let (listener, addr) = bind().await;
    serve(listener, h.app.clone()).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let name = WEB_USER_HEADER;
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nCookie: {session}\r\n\
         Connection: Upgrade\r\nUpgrade: echo\r\n{name}: {FORGED}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_exact(&mut byte),
        )
        .await
        .expect("等到升级响应")
        .unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    assert_eq!(seen_headers(&seen, "/ws", WEB_USER_HEADER), [OWNER_ID]);
}

/// 上游回显收到的头（`headers` 是名字到值的表）。
#[tokio::test]
async fn the_owner_id_is_visible_in_the_forwarded_body() {
    let h = Harness::start(None).await;
    let session = h.app_session().await;
    let reply = h
        .get(
            PLUGIN,
            "/hello",
            &[("cookie", &session), (WEB_USER_HEADER, FORGED)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let key = WEB_USER_HEADER.to_ascii_lowercase();
    assert_eq!(reply.json()["headers"][key.as_str()], OWNER_ID);
}

#[tokio::test]
async fn the_header_is_not_confused_with_the_proxy_sign_in_name() {
    let h = Harness::start(None).await;
    let _ = WHOAMI;
    let session = h.app_session().await;
    let reply = h.get(PLUGIN, "/hello", &[("cookie", &session)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let headers = &reply.json()["headers"];
    assert!(headers.get("remote-user").is_none());
    let key = WEB_USER_HEADER.to_ascii_lowercase();
    assert_eq!(headers[key.as_str()], OWNER_ID);
}

/// 唯一一处钉拼写的地方：它是跨仓库的协议名，别处的伪造和断言都用常量。
#[tokio::test]
async fn the_header_constant_matches_the_spec_spelling() {
    assert_eq!(WEB_USER_HEADER, "X-Wonton-Web-User");
    let _ = TOKEN;
}
