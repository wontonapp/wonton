//! 凭据只写不读、字面量转义限制；首次或换服务商要凭据，同服务商留用旧凭据。

use super::*;

/// 凭据只写不读：文件 0600，GET 只回“写过”，值不在设置页的状态里。
#[tokio::test]
async fn credentials_write_only_and_private() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let path = dir.path().join("entry/domain.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved["credentials"]["HTTPREQ_ENDPOINT"],
        "https://relay.test/present"
    );
    let view = clients.view().await.unwrap();
    assert!(view.credentials_set);
}
#[tokio::test]
async fn credentials_accept_backslash_and_dollar_but_not_single_quote() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.credentials[0].value = SecretString::new("a\\b$c d");
    assert!(domain(dir.path()).save(put.clone()).await.is_ok());
    put.credentials[0].value = SecretString::new("it's");
    assert!(matches!(
        domain(dir.path()).save(put).await,
        Err(DomainError::Invalid(_))
    ));
}
#[tokio::test]
async fn first_provider_requires_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let mut put = request(Some("example.test"));
    put.credentials.clear();
    assert!(matches!(
        clients.save(put).await,
        Err(DomainError::Invalid(_))
    ));
    assert!(!dir.path().join("entry/domain.json").exists());
}
#[tokio::test]
async fn changed_provider_requires_its_own_credentials() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let before = fs::read(dir.path().join("entry/domain.json")).unwrap();
    let mut put = request(None);
    put.provider = "cloudflare".into();
    put.credentials.clear();
    assert!(matches!(
        clients.save(put).await,
        Err(DomainError::Invalid(_))
    ));
    assert_eq!(
        fs::read(dir.path().join("entry/domain.json")).unwrap(),
        before
    );
}
#[tokio::test]
async fn unchanged_provider_retains_credentials() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let before = fs::read(dir.path().join("entry/domain.json")).unwrap();
    let mut put = request(None);
    put.credentials.clear();
    assert!(!clients.save(put).await.unwrap().provider_changed);
    assert_eq!(
        fs::read(dir.path().join("entry/domain.json")).unwrap(),
        before
    );
}
