use super::support::{Home, Phone, paired};
use wonton_client::HomeStatus;

#[tokio::test]
async fn scanning_a_code_pairs_the_phone_and_it_reconnects_after_restart() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home.server.endpoint().id().to_string())
    );
    assert!(view.proxy.is_some());
    assert_eq!(*home.codes.names.lock().unwrap(), vec!["Alice 的 iPhone"]);
    drop(phone);

    // 存下的是服务端的地址，不是一次性的码：重启后直接连上，不用再配对。
    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}
