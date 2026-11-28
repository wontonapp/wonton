//! 配对后的建连与代理：只有拿着会话凭据的本 App 能走代理，服务器记住在磁盘上，换了地址也照旧连上。

use std::net::{Ipv4Addr, SocketAddr};

use wonton_client::HomeStatus;

use super::support::{Home, Phone, WEB_URL, get, wait_until_ticket_has};

#[tokio::test]
async fn a_paired_device_connects_and_only_this_app_can_use_its_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    assert_eq!(
        view.server_id,
        Some(home.server().endpoint().id().to_string())
    );
    assert_eq!(view.web_url.as_deref(), Some(WEB_URL));
    assert_eq!(view.path.map(|path| path.relay), Some(false));

    let proxy = view.proxy.unwrap();
    assert_eq!(proxy.domain, "wonton.localhost");
    let (status, body) = get(&proxy, WEB_URL, "/hello", true).await;
    assert_eq!((status, body.as_str()), (200, "/hello"));
    let (status, _) = get(&proxy, WEB_URL, "/hello", false).await;
    assert_eq!(status, 407, "不带会话凭据的请求（同机其他 App）要被拒");
    let (status, _) = get(&proxy, "http://cdn.example.com", "/app.js", true).await;
    assert_eq!(status, 403, "家里服务器以外的地址不进隧道");
}

#[tokio::test]
async fn the_server_is_remembered_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let (status, _) = get(&view.proxy.unwrap(), WEB_URL, "/again", true).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_server_that_moved_is_reached_at_its_new_address_without_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    let moved = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        wonton_tunnel::test_support::unused_port(),
    ));
    home.report_address(moved);
    phone.connect(&home);
    wait_until_ticket_has(dir.path(), moved).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    home.stop().await;
    home.start_again(moved.port()).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    home.wait_for_a_connection(&phone).await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}
