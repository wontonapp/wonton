//! 经隧道打开插件网页：设备身份替代会话 Cookie，跨插件的请求不带它。

use super::*;

use std::sync::atomic::Ordering;

use hyper::StatusCode;
use iroh::SecretKey;

use super::super::isolation_tests::{OTHER, browser_cases};
use super::super::proxy_sso_tests::{KEY, VAULT_HOST, leaked, received, register_vault};
use super::super::tests::PLUGIN;

#[tokio::test]
async fn paired_device_opens_plugins_without_any_gateway_session() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, _, text) = phone.get(PLUGIN, "/hello?x=1").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let seen: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(seen["path"], "/hello?x=1");
    assert_eq!(seen["headers"]["host"], format!("{PLUGIN}:{PORT}"));
    assert!(seen["headers"].get("cookie").is_none(), "{seen}");
    assert!(
        seen["headers"].get("x-forwarded-for").is_none(),
        "隧道请求没有客户端 IP：{seen}"
    );
    assert_eq!(home.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn paired_devices_open_proxy_sign_in_pages_without_a_password() {
    let home = Home::start(None).await;
    let hits = register_vault(&home.plugins).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let forged = [("cookie", "vault_web=forged")];
    let (status, headers, text) = phone.send(Method::GET, VAULT_HOST, "/leak", &forged).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(received(&text, "cookie"), [format!("vault_web={KEY}")]);
    assert!(leaked(&headers).is_empty(), "{headers:?}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cross_plugin_requests_do_not_carry_the_device_identity() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    // 隧道身份和 Cookie 一样随请求自动带上，所以按同一套规则只认本插件页面的请求和导航。
    let origin = |host: &str| format!("http://{host}:{PORT}");
    let cases = browser_cases(&origin(PLUGIN), &origin(OTHER));
    for case in &cases {
        let (status, headers, _) = phone
            .send(case.method.clone(), PLUGIN, "/notes", &case.headers())
            .await;
        case.check(status, &headers);
    }
    let recognized = cases.iter().filter(|c| c.recognized).count();
    assert_eq!(
        home.hits.load(Ordering::SeqCst),
        recognized,
        "不认人的请求不得到达插件"
    );
}
