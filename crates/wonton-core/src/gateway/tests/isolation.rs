//! 只有插件自己的网络能在没有会话时到达它。

use super::*;

#[tokio::test]
async fn only_the_plugins_own_network_reaches_it_without_a_session() {
    let unauthenticated = &[("accept", "application/json")];
    // 别的插件网络里的容器访问它。
    let h = Harness::start(None).await;
    h.manager
        .register_source_for_test("app.other", "127.0.0.1/32", vec![]);
    let other = h.get(PLUGIN, "/", unauthenticated).await;
    assert_eq!(other.error_kind(), Some("unauthenticated"));

    // 它的容器访问别的插件。
    let h = Harness::start(None).await;
    super::proxy_sso_tests::register_vault(&h.manager).await;
    h.manager
        .register_source_for_test("app.whoami", "127.0.0.1/32", vec![]);
    let elsewhere = h
        .get(super::proxy_sso_tests::VAULT_HOST, "/", unauthenticated)
        .await;
    assert_eq!(elsewhere.error_kind(), Some("unauthenticated"));

    // 不在任何插件网络里的来源。
    let h = Harness::start(None).await;
    let outside = h.get(PLUGIN, "/", unauthenticated).await;
    assert_eq!(outside.error_kind(), Some("unauthenticated"));
}
