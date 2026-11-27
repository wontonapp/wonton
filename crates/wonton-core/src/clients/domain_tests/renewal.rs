//! 续期：没勾条款不联系 CA；设置文件坏了要通知，这一轮不签发。

use super::*;

/// 续期循环和保存走同一道门：没勾订户协议之前绝不联系 CA，哪怕域名和凭据都齐了。
/// “不要名字”的安装没有设置文件、条款没有环境初值，这条就是它的唯一保险。
#[tokio::test]
async fn renewal_waits_for_the_terms_before_contacting_the_ca() {
    use std::sync::atomic::Ordering;
    unsafe { std::env::set_var("WONTON_ACME_DIRECTORY", "https://pebble.test:14000/dir") };
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let settle = async || {
        for _ in 0..20_000 {
            if clients.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("签发没在限定的轮数里结束");
    };
    // 什么都没有：一轮续期不起任何容器。
    clients.renew_if_configured();
    settle().await;
    assert!(clients.engine.recorded_creations().is_empty());
    // 域名、服务商、凭据都有，只差条款：还是不联系 CA。
    clients
        .apply(|store| {
            store.suffix = Some("home.example.test".into());
            store.provider = Some("httpreq".into());
            store.credentials = [("HTTPREQ_ENDPOINT".into(), "x".into())].into();
            Ok(())
        })
        .await
        .unwrap();
    clients.renew_if_configured();
    settle().await;
    assert!(clients.engine.recorded_creations().is_empty());
    // 勾了条款：下一轮续期排到签发，签发在假 Docker 上起 lego 容器时失败。
    clients
        .apply(|store| {
            store.terms_accepted = true;
            Ok(())
        })
        .await
        .unwrap();
    clients.renew_if_configured();
    settle().await;
    assert!(!clients.engine.recorded_creations().is_empty());
}
/// 域名设置文件坏了：发通知、这一轮不联系 CA，也不当没有域名。
#[tokio::test]
async fn an_unreadable_domain_file_is_reported() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = crate::matrix::fake::start().await;
    let http = crate::http::HttpClient::new();
    let matrix = std::sync::Arc::new(
        crate::matrix::Matrix::connect(&config, dir.path(), http)
            .await
            .unwrap(),
    );
    let notices = std::sync::Arc::new(crate::matrix::NoticeRoom::new(
        matrix,
        "owner",
        dir.path().join("notice-room"),
        std::sync::Arc::new(crate::plugin::languages::Languages::default),
    ));
    let clients = super::Domain::new(
        dir.path().to_owned(),
        dir.path().join("data"),
        Engine::for_test(),
        Some(notices),
    );
    std::fs::create_dir_all(dir.path().join("entry")).unwrap();
    std::fs::write(dir.path().join("entry/domain.json"), "{").unwrap();
    clients.renew_if_configured();
    for _ in 0..20_000 {
        if !fake.state().sent.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let sent = fake.state().sent.clone();
    assert!(
        sent.iter().any(|(_, message)| message["body"]
            .as_str()
            .is_some_and(|body| body.contains("domain settings"))),
        "{sent:?}"
    );
    assert_eq!(clients.pending.load(Ordering::SeqCst), 0);
    assert!(clients.engine.recorded_creations().is_empty());
}
