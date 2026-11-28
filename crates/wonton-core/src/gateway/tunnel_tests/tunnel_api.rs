//! `/_wonton/api/tunnel` 下发给 App 的服务器地址和中继表。

use super::*;

use std::net::SocketAddr;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::StatusCode;
use serde_json::Value;
use tower::ServiceExt;

use super::super::tests::PLUGIN;

#[tokio::test]
async fn tunnel_api_reports_the_server_address_to_the_owner() {
    let home = Home::start(None).await;
    let announced = SocketAddr::new("192.168.1.2".parse().unwrap(), home.tunnel.port);
    home.tunnel.endpoint.add_external_addr(announced).await;
    let request = hyper::Request::builder()
        .uri("/_wonton/api/tunnel")
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let info: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info["endpoint_id"], home.tunnel.endpoint.id().to_string());
    assert_eq!(info["base_domain"], CORE);
    assert_eq!(info["direct_addrs"][0], announced.to_string(), "{info}");
    let bound = home.tunnel.endpoint.bound_sockets()[0].to_string();
    assert!(
        !info["direct_addrs"]
            .as_array()
            .unwrap()
            .contains(&Value::String(bound)),
        "{info}"
    );
}

/// 中继的准入令牌单独放在 `token` 里下发，地址里不带；令牌打码、n0 跟着下发。
#[tokio::test]
async fn tunnel_api_reports_relay_tokens_out_of_the_url() {
    let home =
        Home::start_relays(RelayTable::parse("https://sekrit@relay.example.org;n0").unwrap()).await;
    let request = hyper::Request::builder()
        .uri("/_wonton/api/tunnel")
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let info: Value = serde_json::from_slice(&bytes).unwrap();
    let relay = &info["relays"][0];
    assert_eq!(relay["url"], "https://relay.example.org/");
    assert_eq!(relay["token"], "sekrit");
    assert_eq!(relay["tier"], 0);
    assert!(info["relays"].as_array().unwrap().len() > 1, "n0 跟着下发");
    // 插件网页通常带设备身份（上一条测试里被拒）；带主人令牌时也能读到同一份表。
    let request = hyper::Request::builder()
        .uri("/_wonton/api/tunnel")
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::ORIGIN, format!("http://{PLUGIN}:{PORT}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // 日志里的 Debug 不打印令牌。
    let debug = format!("{:?}", home.tunnel.relays());
    assert!(debug.contains("https://relay.example.org/"), "{debug}");
    assert!(!debug.contains("sekrit"), "{debug}");
}
