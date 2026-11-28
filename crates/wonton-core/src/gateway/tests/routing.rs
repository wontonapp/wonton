//! 不认识的主机、未安装、未就绪和没有 OIDC 都回报原因。

use super::*;

#[tokio::test]
async fn unknown_hosts_and_unready_plugins_report_the_reason() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get("evil.example", "/", &[]).await.error_kind(),
        Some("unknown_host")
    );
    let missing = h.get("app-missing.wonton.localhost", "/", &[]).await;
    assert_eq!(missing.error_kind(), Some("not_installed"));
    let app = h.app_session().await;
    let manifest =
        PluginPackage::manifest_of(WHOAMI, &Languages::from_header(Some("zh-Hans"))).unwrap();
    h.manager
        .register_for_test(manifest, PluginStatus::Installing, None);
    let busy = h.get(PLUGIN, "/", &[("cookie", &app)]).await;
    assert_eq!(busy.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(busy.error_kind(), Some("not_ready"));
    let no_oidc = h
        .get(
            "auth.wonton.localhost",
            "/.well-known/openid-configuration",
            &[],
        )
        .await;
    assert_eq!(no_oidc.error_kind(), Some("oidc_unavailable"));
}
