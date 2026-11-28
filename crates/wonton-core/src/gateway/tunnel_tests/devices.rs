//! 设备登记和吊销：没登记的设备进不来，吊销立刻生效；主人的设备能看列表，不能替主人登记。

use super::*;

use std::sync::atomic::Ordering;

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use super::super::tests::PLUGIN;

#[tokio::test]
async fn only_registered_devices_get_in_and_revocation_takes_effect_at_once() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    let phone = Phone::start(device.clone(), &home).await;

    let (status, headers, _) = phone.get(PLUGIN, "/").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        header(&headers, wonton_tunnel::ERROR_HEADER),
        Some("unauthorized")
    );

    home.register(&device).await;
    // 被拒绝过的连接不会自己重拨，要有人重试。
    phone.client.retry_now();
    assert_eq!(phone.get(PLUGIN, "/").await.0, StatusCode::OK);

    let path = format!("/_wonton/api/devices/{}", device.public());
    assert_eq!(home.manage(Method::DELETE, &path, "").await, StatusCode::OK);
    // 吊销会立即断开这台设备现有的隧道连接，正在进行的请求随之中断。
    phone.wait_until_rejected().await;
    let (status, headers, _) = phone.get(PLUGIN, "/").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        header(&headers, wonton_tunnel::ERROR_HEADER),
        Some("unauthorized")
    );
    assert_eq!(
        home.hits.load(Ordering::SeqCst),
        1,
        "吊销后的请求不得到达插件"
    );
}

/// 设备管理按人（ADR-0045）：主人的设备能列所有设备，登记设备仍只认主人令牌；家人的设备只能看到自己的
/// （家人一侧在 family_tests 里）。
#[tokio::test]
async fn the_owners_device_lists_devices_but_cannot_add_without_the_token() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, _, _) = phone.get(CORE, "/_wonton/api/devices").await;
    assert_eq!(status, StatusCode::OK, "主人的设备看得到设备列表");
    let (status, headers, _) = phone
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/devices",
            &[("content-type", "application/json")],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "登记设备仍只认主人令牌");
    assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
}
