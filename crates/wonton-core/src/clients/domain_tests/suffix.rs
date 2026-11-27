//! 后缀：固定名字的安装改不了；无名字的安装能填能改，并回旧值给壳。

use super::*;

#[tokio::test]
async fn suffix_fixed_when_server_name_set() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    let mut put = request(Some("other.example.test"));
    assert!(matches!(
        clients.save(put.clone()).await,
        Err(DomainError::Conflict(_))
    ));
    put.suffix = None;
    assert!(clients.save(put).await.is_ok());
    let view = clients.view().await.unwrap();
    assert_eq!(view.suffix.as_deref(), Some("abcdefgh.example.test"));
    assert!(!view.suffix_editable);
}
/// “不要名字”的安装没有 `entry/settings.json`：后缀能填、能改，改的时候回旧值给壳。
#[tokio::test]
async fn suffix_editable_in_none_mode() {
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let saved = clients.save(request(Some("example.test"))).await.unwrap();
    assert_eq!(saved.previous_suffix, None);
    let saved = clients
        .save(request(Some("second.example.test")))
        .await
        .unwrap();
    assert!(saved.suffix_changed);
    assert_eq!(saved.previous_suffix.as_deref(), Some("example.test"));
    let view = clients.view().await.unwrap();
    assert!(view.suffix_editable);
    assert_eq!(view.suffix.as_deref(), Some("second.example.test"));
}
