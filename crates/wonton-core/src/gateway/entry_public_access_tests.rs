//! 公网总开关在 8484 上的闸门：只拦带公网标记的请求，局域网和经 iroh 的配对前端照旧。

use hyper::{Method, StatusCode};

use crate::gateway::entry_tests::Home;

const PUBLIC: [(&str, &str); 1] = [("x-wonton-entry", "public")];

fn server(home: &Home) -> String {
    home.gateway
        .matrix
        .as_ref()
        .unwrap()
        .server_name()
        .to_owned()
}

#[tokio::test]
async fn entry_federation_and_discovery_get_503_without_touching_the_upstream() {
    let home = Home::start_with_entry(true, Some(false)).await;
    let server = server(&home);
    for path in [
        "/.well-known/wonton",
        "/.well-known/matrix/server",
        "/_matrix/federation/v1/version",
        "/_matrix/key/v2/server_keys/ed25519:a",
    ] {
        let reply = home.request_with(Method::GET, &server, path, &PUBLIC).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply.json()["errcode"], "M_UNKNOWN");
        assert_eq!(reply.error_kind(), Some("matrix_unavailable"));
        let head = home
            .request_with(Method::HEAD, &server, path, &PUBLIC)
            .await;
        assert_eq!(head.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(head.text.is_empty());
    }
    assert!(home.received.lock().unwrap().is_empty(), "关着时不该碰上游");
}

#[tokio::test]
async fn lan_and_paired_requests_keep_working_while_public_is_off() {
    let home = Home::start_with_entry(true, Some(false)).await;
    let server = server(&home);
    for headers in [&[("x-wonton-entry", "lan")][..], &[][..]] {
        let reply = home
            .request_with(Method::GET, &server, "/.well-known/wonton", headers)
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let forwarded = home
        .request_with(Method::PUT, &server, "/_matrix/federation/v1/send/txn", &[])
        .await;
    assert_eq!(forwarded.status, StatusCode::ACCEPTED);
    assert_eq!(home.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn public_plugin_entries_get_the_public_access_page() {
    let home = Home::start_with_entry(true, Some(false)).await;
    let server = server(&home);
    let reply = home
        .request_with(Method::GET, &format!("app.{server}"), "/", &PUBLIC)
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.headers.get("x-wonton-error").unwrap(), "entry_closed");
    let lan = home
        .request_with(
            Method::GET,
            &format!("app.lan.{server}"),
            "/",
            &[("x-wonton-entry", "lan")],
        )
        .await;
    // 没有这个插件：过了闸门之后照旧规则回 421。
    assert_eq!(lan.status, StatusCode::MISDIRECTED_REQUEST);
}

#[tokio::test]
async fn public_requests_pass_again_once_the_switch_is_on() {
    let home = Home::start_with_entry(true, Some(true)).await;
    let server = server(&home);
    let discovery = home
        .request_with(Method::GET, &server, "/.well-known/wonton", &PUBLIC)
        .await;
    assert_eq!(discovery.status, StatusCode::OK);
    let unknown_plugin = home
        .request_with(Method::GET, &format!("app.{server}"), "/", &PUBLIC)
        .await;
    assert_eq!(unknown_plugin.status, StatusCode::MISDIRECTED_REQUEST);
}
