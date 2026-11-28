//! 签发失败：旧证书留着，状态报错。

use super::*;

/// 签发失败：旧证书原样留着，状态带 lego 的错误。设置直接从装机环境来，不经 `save`
/// 排的那次后台签发，结果才确定。
#[tokio::test]
async fn issue_failure_keeps_previous_certificate_and_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    fs::write(&env_file, "HTTPREQ_ENDPOINT=https://relay.test/present\n").unwrap();
    settings_with_provider(dir.path(), &env_file);
    certificate(dir.path());
    let before = fs::read(dir.path().join("entry/certs/site.pem")).unwrap();
    let clients = domain(dir.path());
    // 假 Engine 连不上 Docker：签发死在起 lego 之前，错误要来自这一步。
    let error = clients.issue_now().await.unwrap_err();
    let text = format!("{error:#}").to_lowercase();
    assert!(
        text.contains("docker") || text.contains("lego") || text.contains("socket"),
        "{text}"
    );
    assert_eq!(
        fs::read(dir.path().join("entry/certs/site.pem")).unwrap(),
        before
    );
    match clients.view().await.unwrap().certificate {
        wonton_protocol::CertificateState::Renewing { error, .. } => {
            assert!(!error.is_empty())
        }
        other => panic!("状态应当是 renewing，实际 {other:?}"),
    }
}
