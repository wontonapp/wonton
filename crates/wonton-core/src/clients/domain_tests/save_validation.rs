//! 保存时的校验：订户协议必勾、服务商代码字符集、自己的证书不能再签发。

use super::*;

#[tokio::test]
async fn put_domain_requires_terms() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.accept_terms = false;
    let error = domain(dir.path()).save(put).await.err().unwrap();
    assert!(matches!(error, DomainError::Invalid(_)), "{error}");
}
#[tokio::test]
async fn provider_name_charset_only() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.provider = "Cloudflare_DNS".into();
    assert!(matches!(
        domain(dir.path()).save(put.clone()).await,
        Err(DomainError::Invalid(_))
    ));
    // 核心不按服务商分支：不在任何预设里的代码照样收下。
    put.provider = "notapreset1".into();
    assert!(domain(dir.path()).save(put).await.is_ok());
}
/// 自己的证书：设置收下也没人签，PUT 直接回冲突。
#[tokio::test]
async fn external_certificate_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let path = dir.path().join("entry/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["external_certificate"] = serde_json::json!(true);
    fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
    assert!(matches!(
        domain(dir.path()).save(request(None)).await,
        Err(DomainError::Conflict(_))
    ));
}
