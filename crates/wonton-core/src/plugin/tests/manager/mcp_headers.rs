//! mcp.headers 的解析错误。

use super::super::super::proxy_auth::resolve_mcp_headers;
use super::super::super::{InstallError, PluginPackage};
use super::super::{langs, message, package};
use super::manager;

/// mcp.headers 解析不了（这里是密钥还没有值）是插件交的东西不对：报 Invalid，说清是哪个插件、哪个密钥。
#[tokio::test]
async fn mcp_headers_that_cannot_be_resolved_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let host = manager(&dir);
    let package = PluginPackage::parse(&package(
        "  mcp:\n    image: nginx",
        "  permissions: []\n  secrets:\n    pat:\n      collect: {service: mcp, path: /run/token}\n  \
         mcp:\n    service: mcp\n    port: 8080\n    \
         transport: streamable-http\n    headers:\n      Authorization: Bearer {secret:pat}",
    ), &langs())
    .unwrap();
    let error = host
        .mcp_upstream(&package.manifest, "net", &Default::default())
        .await
        .unwrap_err();
    assert!(matches!(error, InstallError::Invalid(_)), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("app.test") && text.contains("pat"), "{text}");
}

/// collect 的文件常是 echo 写的：结尾的换行也让头值不合法。报错说清原因，但不带密钥的值。
#[test]
fn a_collected_secret_with_trailing_newline_is_rejected_for_mcp_headers() {
    use std::collections::BTreeMap;

    use hyper::header::AUTHORIZATION;

    let package = PluginPackage::parse(&package(
        "  mcp:\n    image: nginx\n  init:\n    image: nginx",
        "  permissions: []\n  secrets:\n    pat:\n      collect: {service: init, path: /run/token}\n  \
         mcp:\n    service: mcp\n    port: 8080\n    transport: streamable-http\n    headers:\n      \
         Authorization: Bearer {secret:pat}",
    ), &langs())
    .unwrap();
    let newline = BTreeMap::from([("pat".to_owned(), b"tok-123\n".to_vec())]);
    let error = resolve_mcp_headers(&package.manifest, &newline, &langs()).unwrap_err();
    let expected = message(
        "plugin-install-proxy-secret-newline",
        &[("name", "authorization"), ("secret", "pat")],
    );
    assert_eq!(error, expected, "{error}");
    assert!(error.contains("pat"), "{error}");
    assert!(!error.contains("tok-123"), "{error}");
    let clean = BTreeMap::from([("pat".to_owned(), b"tok-123".to_vec())]);
    let headers = resolve_mcp_headers(&package.manifest, &clean, &langs()).unwrap();
    let map = headers.custom_headers();
    assert_eq!(map[&AUTHORIZATION], "Bearer tok-123");
    assert!(map[&AUTHORIZATION].is_sensitive(), "密钥要标记 sensitive");
}
