//! CAA 分类：对上账号、没有、对不上。

use super::*;

/// 假解析结果只喂给分类函数：三种都按“对上账号 / 没有 / 对不上”进状态。
#[test]
fn caa_states() {
    use crate::entry_dns::{CaaIssue, caa_state};
    let account = "https://pebble.test/my-account/1";
    let bound = CaaIssue {
        tag: "issue".into(),
        issuer: Some("letsencrypt.org".into()),
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[bound], account, "letsencrypt.org"),
        CaaState::Bound
    );
    assert_eq!(
        caa_state(&[], account, "letsencrypt.org"),
        CaaState::Missing
    );
    let mismatch = CaaIssue {
        tag: "issue".into(),
        issuer: Some("letsencrypt.org".into()),
        parameters: vec![("accounturi".into(), "https://other.test/1".into())],
    };
    assert_eq!(
        caa_state(&[mismatch], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
    // issuer 是别家：即使账号和方法都对也不算绑定（真实 RDATA 的版本在 entry_dns 的测试里）。
    let other_issuer = CaaIssue {
        tag: "issue".into(),
        issuer: Some("other-ca.example".into()),
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[other_issuer], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
    let empty_issuer = CaaIssue {
        tag: "issue".into(),
        issuer: None,
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[empty_issuer], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
}
