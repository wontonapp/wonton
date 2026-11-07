//! 服务器文字按请求的语言写（ADR-0043）：权限说明走 Fluent 目录，按 `Accept-Language` 协商，
//! 期望值用目录本身格式化出来比。

use wonton_protocol::PermissionRisk;

use super::super::languages::Languages;
use super::super::manifest::Permission;
use super::super::views_permissions::{permission_line, permission_text};
use crate::l10n::CATALOG;

#[test]
fn permission_lines_follow_the_requested_language() {
    // 目录里有的语言给那一条；没有的（fr-FR）回退英文。
    for (header, tag) in [("ja", "ja"), ("fr-FR", "en"), ("zh-TW", "zh-Hant")] {
        let langs = Languages::from_header(Some(header));
        let expected = CATALOG.negotiate(&[tag]);
        let line = permission_line(&Permission::NetworkLan, &langs);
        assert_eq!(line.risk, Some(PermissionRisk::High));
        assert_eq!(
            line.text,
            CATALOG.format(expected, "permission-network-lan", &[]),
            "{header}"
        );
        assert_eq!(
            line.notice.expect("LAN ports carry a notice"),
            CATALOG.format(expected, "permission-notice-network-lan", &[]),
            "{header}"
        );
    }
    // 两种请求真的给出两种语言，防止协商被绕过。
    let ja = permission_text(&Permission::NetworkLan, &Languages::from_header(Some("ja")));
    let en = permission_text(
        &Permission::NetworkLan,
        &Languages::from_header(Some("fr-FR")),
    );
    assert_ne!(ja, en);
}
