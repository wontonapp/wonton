//! 管理接口的文字按请求的 `Accept-Language` 写（ADR-0043）。

use hyper::StatusCode;

use super::tests::{CORE, Harness};
use crate::l10n::CATALOG;

/// 401 的文字按请求的 Accept-Language 写（ADR-0043）；期望值由目录格式化出来比。
#[tokio::test]
async fn the_denied_message_follows_the_accept_language_header() {
    let h = Harness::start(None).await;
    let ja = h
        .get(CORE, "/_wonton/api/plugins", &[("accept-language", "ja")])
        .await;
    assert_eq!(ja.status, StatusCode::UNAUTHORIZED);
    let fr = h
        .get(
            CORE,
            "/_wonton/api/plugins",
            &[("accept-language", "fr-FR")],
        )
        .await;
    assert_eq!(fr.status, StatusCode::UNAUTHORIZED);
    // fr-FR 目录里没有，回退英文；两种头要真的给出两种话。
    let expected = |tag: &str| CATALOG.format(CATALOG.negotiate(&[tag]), "gateway-denied", &[]);
    assert_eq!(ja.json()["error"]["message"], expected("ja"));
    assert_eq!(fr.json()["error"]["message"], expected("en"));
    assert_ne!(expected("ja"), expected("en"));
}
